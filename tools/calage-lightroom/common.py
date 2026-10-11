from PIL import Image
import numpy as np

SRC = '/tmp/mires'

def load(n):
    return np.asarray(Image.open(f'{SRC}/{n}.png').convert('RGB')).astype(np.float64) / 255

def to_lin(v):
    return np.where(v <= 0.04045, v / 12.92, ((v + 0.055) / 1.055) ** 2.4)

def to_srgb(v):
    v = np.clip(v, 0, 1)
    return np.where(v <= 0.0031308, v * 12.92, 1.055 * v ** (1 / 2.4) - 0.055)

REF = load('ref')

def pair(n):
    cap = load(n)
    h = min(REF.shape[0], cap.shape[0])
    return REF[:h].reshape(-1, 3), cap[:h].reshape(-1, 3)

def fit_matrix(n):
    """Matrice 3x3 en lumière linéaire, ajustée canal par canal sur les pixels non écrêtés."""
    a, b = pair(n)
    la, lb = to_lin(a), to_lin(b)
    M = np.zeros((3, 3))
    for c in range(3):
        ok = (b[:, c] > 0.02) & (b[:, c] < 0.97)
        M[c] = np.linalg.lstsq(la[ok], lb[ok, c], rcond=None)[0]
    pred = to_srgb(la @ M.T)
    return M, np.abs(pred - b).mean() * 255, np.abs(pred - b).max() * 255

LUT_N = 33  # points de la courbe par canal, en échelle perceptuelle

def fit_matrix_curves(n, mask_edges=True):
    """Modèle : matrice 3x3 en lumière linéaire, puis une courbe 1D par canal.

    La matrice porte le mélange des canaux (balance des blancs, étalonnage, saturation),
    la courbe porte le rendu et l'écrêtage — ce que Lightroom fait après, et qu'une
    matrice seule ne peut pas représenter.
    """
    a, b = pair(n)
    la = to_lin(a)
    lb = to_lin(b)
    M = np.zeros((3, 3))
    for c in range(3):
        ok = (b[:, c] > 0.02) & (b[:, c] < 0.97)
        if ok.sum() < 5000:
            ok = np.ones(len(b), bool)
        M[c] = np.linalg.lstsq(la[ok], lb[ok, c], rcond=None)[0]
    t = la @ M.T
    u = to_srgb(np.clip(t, 0, 1))              # abscisse perceptuelle de la courbe
    luts = np.zeros((3, LUT_N))
    idx = np.clip((u * (LUT_N - 1)), 0, LUT_N - 1)
    for c in range(3):
        lo = np.floor(idx[:, c]).astype(int)
        for k in range(LUT_N):
            sel = lo == k
            luts[c, k] = np.median(b[sel, c]) if sel.sum() > 30 else np.nan
        # Trous comblés par interpolation, puis extrémités tenues.
        good = ~np.isnan(luts[c])
        if good.sum() >= 2:
            luts[c] = np.interp(np.arange(LUT_N), np.where(good)[0], luts[c][good])
        else:
            luts[c] = np.linspace(0, 1, LUT_N)
    pred = apply_mc(a, M, luts)
    err = np.abs(pred - b)
    return M, luts, err.mean() * 255, np.percentile(err, 99) * 255

def apply_mc(a, M, luts):
    t = to_lin(a) @ M.T
    u = to_srgb(np.clip(t, 0, 1)) * (LUT_N - 1)
    out = np.empty_like(a)
    for c in range(3):
        k = np.clip(np.floor(u[:, c]), 0, LUT_N - 2).astype(int)
        f = u[:, c] - k
        out[:, c] = luts[c][k] * (1 - f) + luts[c][k + 1] * f
    return np.clip(out, 0, 1)

def _lut_from(t, b, w=None):
    """Courbe par canal : médiane de la sortie observée, par tranche de l'entrée prédite."""
    u = to_srgb(np.clip(t, 0, 1))
    luts = np.zeros((3, LUT_N))
    for c in range(3):
        lo = np.clip(np.floor(u[:, c] * (LUT_N - 1)), 0, LUT_N - 1).astype(int)
        vals = np.full(LUT_N, np.nan)
        for k in range(LUT_N):
            sel = lo == k
            if sel.sum() > 30:
                vals[k] = np.median(b[sel, c])
        good = ~np.isnan(vals)
        luts[c] = np.interp(np.arange(LUT_N), np.where(good)[0], vals[good]) if good.sum() >= 2 else np.linspace(0, 1, LUT_N)
        luts[c] = np.maximum.accumulate(luts[c]) if luts[c][-1] >= luts[c][0] else -np.maximum.accumulate(-luts[c])
    return luts

def _invert(luts, b):
    """Remonte la courbe : quelle entrée linéaire produirait la sortie observée ?"""
    grid = to_lin(np.linspace(0, 1, LUT_N))
    out = np.empty_like(b)
    for c in range(3):
        y, x = luts[c], grid
        if y[-1] < y[0]:
            y, x = y[::-1], x[::-1]
        out[:, c] = np.interp(b[:, c], y, x)
    return out

def fit_als(n, rounds=4):
    """Matrice et courbes ajustées tour à tour, chacune sur le résidu de l'autre."""
    a, b = pair(n)
    la = to_lin(a)
    M = np.eye(3)
    best = None
    for r in range(rounds):
        t = la @ M.T
        luts = _lut_from(t, b)
        pred = apply_mc(a, M, luts)
        err = np.abs(pred - b).mean() * 255
        if best is None or err < best[2]:
            best = (M.copy(), luts.copy(), err)
        target = _invert(luts, b)
        ok = (b.max(axis=1) < 0.985) & (b.min(axis=1) > 0.012)
        if ok.sum() < 5000:
            ok = np.ones(len(b), bool)
        M = np.linalg.lstsq(la[ok], target[ok], rcond=None)[0].T
    M, luts, err = best
    e = np.abs(apply_mc(a, M, luts) - b)
    return M, luts, e.mean() * 255, np.percentile(e, 99) * 255

def feats(l):
    """Polynôme racine : les six termes gardent l'invariance à l'exposition, contrairement
    à un polynôme classique. C'est le modèle usuel de correction colorimétrique."""
    r, g, b = l[:, 0], l[:, 1], l[:, 2]
    return np.stack([r, g, b, np.sqrt(np.maximum(r * g, 0)), np.sqrt(np.maximum(g * b, 0)), np.sqrt(np.maximum(r * b, 0))], axis=1)

def fit_poly(n):
    a, b = pair(n)
    la = to_lin(a)
    F = feats(la)
    M = np.zeros((3, F.shape[1]))
    for c in range(3):
        ok = (b[:, c] > 0.02) & (b[:, c] < 0.97)
        if ok.sum() < 5000:
            ok = np.ones(len(b), bool)
        M[c] = np.linalg.lstsq(F[ok], lb_c := to_lin(b)[ok, c], rcond=None)[0]
    t = F @ M.T
    luts = _lut_from(t, b)
    pred = apply_poly(a, M, luts)
    e = np.abs(pred - b)
    return M, luts, e.mean() * 255, np.percentile(e, 99) * 255

def apply_poly(a, M, luts):
    t = feats(to_lin(a)) @ M.T
    u = to_srgb(np.clip(t, 0, 1)) * (LUT_N - 1)
    out = np.empty_like(a)
    for c in range(3):
        k = np.clip(np.floor(u[:, c]), 0, LUT_N - 2).astype(int)
        f = u[:, c] - k
        out[:, c] = luts[c][k] * (1 - f) + luts[c][k + 1] * f
    return np.clip(out, 0, 1)
