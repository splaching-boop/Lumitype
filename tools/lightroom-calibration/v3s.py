from common import *
from smooth import whittaker, roughness, isotonic, banding
import numpy as np

LUT, SECT, LUMA = 129, 96, 65

def f3(l): return l
def f6(l):
    r, g, b = l.T
    return np.stack([r, g, b, np.sqrt(np.maximum(r*g, 0)), np.sqrt(np.maximum(g*b, 0)), np.sqrt(np.maximum(r*b, 0))], 1)
FEATS = {3: f3, 6: f6}

def binned(key, values, n, minimum=8):
    """Médiane et effectif par tranche — la matière première du lissage."""
    k = np.clip(np.floor(key), 0, n - 1).astype(int)
    order = np.argsort(k, kind='stable')
    counts = np.bincount(k, minlength=n)
    edges = np.concatenate(([0], np.cumsum(counts)))
    sv = values[order]
    med = np.zeros(n); w = np.zeros(n)
    for i in range(n):
        if counts[i] >= minimum:
            med[i] = np.median(sv[edges[i]:edges[i+1]])
            w[i] = min(counts[i], 4000)
    return med, w

def apply_lut(u, lut):
    k = np.clip(np.floor(u), 0, len(lut) - 2).astype(int)
    fr = u - k
    return lut[k] * (1 - fr) + lut[k + 1] * fr

def build(n, lam_lut, lam_hue, lam_lum, mono=True):
    a, b = pair(n)
    best = None
    for terms in (3, 6):
        for space in ('lin', 'srgb'):
            base = to_lin(a) if space == 'lin' else a
            F = FEATS[terms](base)
            tgt = to_lin(b) if space == 'lin' else b
            M = np.zeros((3, terms))
            for c in range(3):
                ok = (b[:, c] > 0.02) & (b[:, c] < 0.97)
                if ok.sum() < 5000: ok = np.ones(len(b), bool)
                M[c] = np.linalg.lstsq(F[ok], tgt[ok, c], rcond=None)[0]
            t = F @ M.T
            u = (to_srgb(np.clip(t, 0, 1)) if space == 'lin' else np.clip(t, 0, 1)) * (LUT - 1)
            luts = np.zeros((3, LUT)); pred = np.empty_like(a)
            for c in range(3):
                med, w = binned(u[:, c], b[:, c], LUT)
                fill = np.where(w > 0)[0]
                if len(fill) >= 2:
                    med = np.interp(np.arange(LUT), fill, med[fill])
                    w = np.maximum(w, 1e-3)
                else:
                    med, w = np.linspace(0, 1, LUT), np.ones(LUT)
                lut = whittaker(med, w, lam_lut) if lam_lut > 0 else med
                if mono:
                    # Une courbe de transfert ne redescend pas : un creux se verrait comme une bande.
                    lut = isotonic(lut, w)
                    lut = whittaker(lut, np.ones(LUT), 200)
                    lut = isotonic(lut, np.ones(LUT))
                luts[c] = lut
                pred[:, c] = apply_lut(u[:, c], luts[c])
            pred = np.clip(pred, 0, 1)
            e = np.abs(pred - b).mean()
            if best is None or e < best[0]:
                best = (e, pred, M, luts, terms, space, u)
    _, pred, M, luts, terms, space, _ = best

    # Correction par teinte, pondérée par la chroma : nulle sur un gris.
    mx, mn = a.max(1), a.min(1); chroma = mx - mn
    r, g, bb = a.T; ismax = a.argmax(1); d = np.maximum(chroma, 1e-9)
    hue = np.where(ismax == 0, ((g - bb) / d) % 6, np.where(ismax == 1, (bb - r) / d + 2, (r - g) / d + 4)) / 6
    hue = np.where(chroma > 1e-6, hue % 1.0, 0.0)
    res = b - pred
    sel = chroma > 0.08
    table = np.zeros((SECT, 3))
    idx = hue * SECT
    for c in range(3):
        med, w = binned(idx[sel], (res[sel, c] / np.maximum(chroma[sel], 1e-6)), SECT, minimum=40)
        table[:, c] = whittaker(med, np.maximum(w, 1e-3), lam_hue, circular=True) if lam_hue > 0 else med
    k = np.floor(idx).astype(int) % SECT; fr = idx - np.floor(idx)
    corr = table[k] * (1 - fr[:, None]) + table[(k + 1) % SECT] * fr[:, None]
    pred = np.clip(pred + corr * chroma[:, None], 0, 1)

    # Courbe résiduelle indexée par la luminance d'entrée.
    L = 0.2126 * a[:, 0] + 0.7152 * a[:, 1] + 0.0722 * a[:, 2]
    res = b - pred
    lidx = np.clip(L, 0, 1) * (LUMA - 1)
    lum = np.zeros((LUMA, 3))
    for c in range(3):
        med, w = binned(lidx, res[:, c], LUMA, minimum=40)
        lum[:, c] = whittaker(med, np.maximum(w, 1e-3), lam_lum) if lam_lum > 0 else med
    kk = np.clip(np.floor(lidx), 0, LUMA - 2).astype(int); fr2 = lidx - kk
    pred = np.clip(pred + lum[kk] * (1 - fr2[:, None]) + lum[kk + 1] * fr2[:, None], 0, 1)

    hgt = len(b) // 900
    return {'pred': pred, 'err': np.abs(pred - b).mean() * 255, 'rough': roughness(pred, hgt),
            'band': banding(pred, hgt),
            'M': M, 'luts': luts, 'hue': table, 'lum': lum, 'terms': terms, 'space': space}
