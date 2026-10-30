"""v4 : même modèle que v3, mais la correction résiduelle devient neutre.

v3 ajustait sa dernière courbe sur *tous* les pixels d'une même luminance — gris et couleurs
confondus. La mire étant à 80 % faite de couleurs saturées, les gris se faisaient écraser :
jusqu'à 19,7 d'écart sur les gris quand la moyenne affichait 5,3. Or un gris qui part en
couleur se voit tout de suite, contrairement à une teinte saturée légèrement fausse.

La correction est donc scindée :
  - par teinte, pondérée par la chroma  -> nulle sur un gris, pleine sur une couleur vive ;
  - par luminance, pondérée à l'inverse -> pleine sur un gris, nulle dès que ça sature.
"""
from common import *
from smooth import whittaker, isotonic, banding
from v3s import binned, apply_lut, FEATS
import numpy as np

LUT, SECT, LUMA = 129, 96, 65
CHROMA_FULL = 0.25          # au-delà, la correction neutre ne s'applique plus du tout
CHROMA_GREY = 0.08          # en deçà, le pixel sert à ajuster la correction neutre

def hue_of(a):
    mx, mn = a.max(1), a.min(1); chroma = mx - mn
    r, g, b = a.T; ismax = a.argmax(1); d = np.maximum(chroma, 1e-9)
    h = np.where(ismax == 0, ((g - b) / d) % 6, np.where(ismax == 1, (b - r) / d + 2, (r - g) / d + 4)) / 6
    return np.where(chroma > 1e-6, h % 1.0, 0.0), chroma

def build(n, lam_lut=5000, lam_hue=1500, lam_lum=1500):
    a, b = pair(n)
    best = None
    for terms in (3, 6):
        for space in ('lin', 'srgb'):
            base = to_lin(a) if space == 'lin' else a
            F = FEATS[terms](base); tgt = to_lin(b) if space == 'lin' else b
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
                med = np.interp(np.arange(LUT), fill, med[fill]) if len(fill) >= 2 else np.linspace(0, 1, LUT)
                lut = whittaker(med, np.maximum(w, 1e-3), lam_lut)
                lut = isotonic(lut, np.maximum(w, 1e-3))
                lut = whittaker(lut, np.ones(LUT), 200)
                luts[c] = isotonic(lut, np.ones(LUT))
                pred[:, c] = apply_lut(u[:, c], luts[c])
            pred = np.clip(pred, 0, 1)
            e = np.abs(pred - b).mean()
            if best is None or e < best[0]: best = (e, pred, M, luts, terms, space)
    _, pred, M, luts, terms, space = best

    hue, chroma = hue_of(a)
    neutral = np.maximum(0.0, 1.0 - chroma / CHROMA_FULL)

    # 1) Correction neutre, ajustée sur les gris seuls.
    res = b - pred
    grey = chroma < CHROMA_GREY
    L = 0.2126 * a[:, 0] + 0.7152 * a[:, 1] + 0.0722 * a[:, 2]
    lidx = np.clip(L, 0, 1) * (LUMA - 1)
    lum = np.zeros((LUMA, 3))
    for c in range(3):
        med, w = binned(lidx[grey], res[grey, c], LUMA, minimum=30)
        fill = np.where(w > 0)[0]
        med = np.interp(np.arange(LUMA), fill, med[fill]) if len(fill) >= 2 else np.zeros(LUMA)
        lum[:, c] = whittaker(med, np.maximum(w, 1e-3), lam_lum) if len(fill) >= 2 else med
    k = np.clip(np.floor(lidx), 0, LUMA - 2).astype(int); fr = lidx - k
    pred = np.clip(pred + (lum[k] * (1 - fr[:, None]) + lum[k + 1] * fr[:, None]) * neutral[:, None], 0, 1)

    # 2) Correction par teinte, ajustée sur les couleurs, pondérée par la chroma.
    res = b - pred
    idx = hue * SECT
    table = np.zeros((SECT, 3))
    sel = chroma > CHROMA_GREY
    for c in range(3):
        med, w = binned(idx[sel], res[sel, c] / np.maximum(chroma[sel], 1e-6), SECT, minimum=40)
        fill = np.where(w > 0)[0]
        med = np.interp(np.arange(SECT), fill, med[fill]) if len(fill) >= 2 else np.zeros(SECT)
        table[:, c] = whittaker(med, np.maximum(w, 1e-3), lam_hue, circular=True) if len(fill) >= 2 else med
    k = np.floor(idx).astype(int) % SECT; fr = idx - np.floor(idx)
    corr = table[k] * (1 - fr[:, None]) + table[(k + 1) % SECT] * fr[:, None]
    pred = np.clip(pred + corr * chroma[:, None], 0, 1)

    err = np.abs(pred - b)
    hgt = len(b) // 900
    return {'pred': pred, 'err': err.mean() * 255, 'grey': err[grey].mean() * 255,
            'band': banding(pred, hgt), 'M': M, 'luts': luts, 'hue': table, 'lum': lum,
            'terms': terms, 'space': space}
