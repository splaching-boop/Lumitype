"""v5 : les gris pèsent leur vrai poids dans l'ajustement de l'étage 1.

La mire n'est faite qu'à 20 % de gris. Un ajustement au sens des moindres carrés les
sacrifie donc volontiers : sur `Rouge primaire -100`, un gris à 63 ressortait avec 60 niveaux
d'écart entre ses canaux — franchement coloré. La correction des neutres rattrapait ensuite,
mais avec de gros coefficients, et seulement là où la mire avait des gris.

Ici les pixels proches du neutre reçoivent un poids `1 + K` dans la matrice **et** dans les
courbes. L'étage 1 sort alors déjà juste sur les gris, et la correction des neutres n'a plus
qu'un résidu à traiter.
"""
from common import *
from smooth import whittaker, isotonic, banding
from v3s import FEATS, apply_lut
import numpy as np

LUT, SECT, LUMA = 129, 96, 65
CHROMA_FULL, CHROMA_GREY = 0.25, 0.08

def weighted_median(values, weights):
    order = np.argsort(values)
    v, w = values[order], weights[order]
    c = np.cumsum(w)
    return v[np.searchsorted(c, c[-1] / 2)]

def binned_w(key, values, weights, n, minimum=8):
    k = np.clip(np.floor(key), 0, n - 1).astype(int)
    med = np.zeros(n); tot = np.zeros(n)
    for i in range(n):
        sel = k == i
        if sel.sum() >= minimum:
            med[i] = weighted_median(values[sel], weights[sel])
            tot[i] = min(weights[sel].sum(), 8000)
    return med, tot

def hue_of(a):
    mx, mn = a.max(1), a.min(1); chroma = mx - mn
    r, g, b = a.T; ismax = a.argmax(1); d = np.maximum(chroma, 1e-9)
    h = np.where(ismax == 0, ((g - b) / d) % 6, np.where(ismax == 1, (b - r) / d + 2, (r - g) / d + 4)) / 6
    return np.where(chroma > 1e-6, h % 1.0, 0.0), chroma

def build(n, grey_weight=6.0, lam_lut=5000, lam_hue=1500, lam_lum=1500):
    a, b = pair(n)
    hue, chroma = hue_of(a)
    neutral = np.maximum(0.0, 1.0 - chroma / CHROMA_FULL)
    weight = 1.0 + grey_weight * neutral

    best = None
    for terms in (3, 6):
        for space in ('lin', 'srgb'):
            base = to_lin(a) if space == 'lin' else a
            F = FEATS[terms](base); tgt = to_lin(b) if space == 'lin' else b
            M = np.zeros((3, terms))
            for c in range(3):
                ok = (b[:, c] > 0.02) & (b[:, c] < 0.97)
                if ok.sum() < 5000: ok = np.ones(len(b), bool)
                s = np.sqrt(weight[ok])[:, None]
                M[c] = np.linalg.lstsq(F[ok] * s, tgt[ok, c] * s[:, 0], rcond=None)[0]
            t = F @ M.T
            u = (to_srgb(np.clip(t, 0, 1)) if space == 'lin' else np.clip(t, 0, 1)) * (LUT - 1)
            luts = np.zeros((3, LUT)); pred = np.empty_like(a)
            for c in range(3):
                med, w = binned_w(u[:, c], b[:, c], weight, LUT)
                fill = np.where(w > 0)[0]
                med = np.interp(np.arange(LUT), fill, med[fill]) if len(fill) >= 2 else np.linspace(0, 1, LUT)
                lut = whittaker(med, np.maximum(w, 1e-3), lam_lut)
                lut = isotonic(lut, np.maximum(w, 1e-3))
                lut = whittaker(lut, np.ones(LUT), 200)
                luts[c] = isotonic(lut, np.ones(LUT))
                pred[:, c] = apply_lut(u[:, c], luts[c])
            pred = np.clip(pred, 0, 1)
            e = np.abs(pred - b) * weight[:, None]
            score = e.mean()
            if best is None or score < best[0]: best = (score, pred, M, luts, terms, space)
    _, pred, M, luts, terms, space = best

    grey = chroma < CHROMA_GREY
    L = 0.2126 * a[:, 0] + 0.7152 * a[:, 1] + 0.0722 * a[:, 2]
    lidx = np.clip(L, 0, 1) * (LUMA - 1)
    res = b - pred
    lum = np.zeros((LUMA, 3))
    for c in range(3):
        med, w = binned_w(lidx[grey], res[grey, c], np.ones(grey.sum()), LUMA, minimum=30)
        fill = np.where(w > 0)[0]
        med = np.interp(np.arange(LUMA), fill, med[fill]) if len(fill) >= 2 else np.zeros(LUMA)
        lum[:, c] = whittaker(med, np.maximum(w, 1e-3), lam_lum) if len(fill) >= 2 else med
    k = np.clip(np.floor(lidx), 0, LUMA - 2).astype(int); fr = lidx - k
    pred = np.clip(pred + (lum[k] * (1 - fr[:, None]) + lum[k + 1] * fr[:, None]) * neutral[:, None], 0, 1)

    res = b - pred
    idx = hue * SECT
    table = np.zeros((SECT, 3))
    sel = chroma > CHROMA_GREY
    for c in range(3):
        med, w = binned_w(idx[sel], res[sel, c] / np.maximum(chroma[sel], 1e-6), np.ones(sel.sum()), SECT, minimum=40)
        fill = np.where(w > 0)[0]
        med = np.interp(np.arange(SECT), fill, med[fill]) if len(fill) >= 2 else np.zeros(SECT)
        table[:, c] = whittaker(med, np.maximum(w, 1e-3), lam_hue, circular=True) if len(fill) >= 2 else med
    k = np.floor(idx).astype(int) % SECT; fr = idx - np.floor(idx)
    corr = table[k] * (1 - fr[:, None]) + table[(k + 1) % SECT] * fr[:, None]
    pred = np.clip(pred + corr * chroma[:, None], 0, 1)

    err = np.abs(pred - b)
    return {'pred': pred, 'err': err.mean() * 255, 'grey': err[grey].mean() * 255,
            'band': banding(pred, len(b) // 900), 'M': M, 'luts': luts, 'hue': table, 'lum': lum,
            'terms': terms, 'space': space}
