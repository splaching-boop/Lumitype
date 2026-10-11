"""Export final : v2 et v3, tables lissées."""
from common import *
from smooth import whittaker, roughness, isotonic, banding
from v3s import build, binned, apply_lut, FEATS
import numpy as np, json

LAM = (5000, 1500, 1500)

def build_v2(n, lam):
    """v2 : le premier modèle — matrice ou polynôme en lumière linéaire, courbe de 33 points."""
    a, b = pair(n)
    best = None
    for terms in (3, 6):
        F = FEATS[terms](to_lin(a))
        tgt = to_lin(b)
        M = np.zeros((3, terms))
        for c in range(3):
            ok = (b[:, c] > 0.02) & (b[:, c] < 0.97)
            if ok.sum() < 5000: ok = np.ones(len(b), bool)
            M[c] = np.linalg.lstsq(F[ok], tgt[ok, c], rcond=None)[0]
        u = to_srgb(np.clip(F @ M.T, 0, 1)) * 32
        luts = np.zeros((3, 33)); pred = np.empty_like(a)
        for c in range(3):
            med, w = binned(u[:, c], b[:, c], 33)
            fill = np.where(w > 0)[0]
            med = np.interp(np.arange(33), fill, med[fill]) if len(fill) >= 2 else np.linspace(0, 1, 33)
            lut = whittaker(med, np.maximum(w, 1e-3), lam)
            lut = isotonic(lut, np.maximum(w, 1e-3))
            lut = whittaker(lut, np.ones(33), 8)
            luts[c] = isotonic(lut, np.ones(33))
            pred[:, c] = apply_lut(u[:, c], luts[c])
        pred = np.clip(pred, 0, 1)
        e = np.abs(pred - b).mean()
        if best is None or e < best[0]: best = (e, pred, M, luts, terms)
    e, pred, M, luts, terms = best
    return {'err': e * 255, 'band': banding(pred, len(b) // 900), 'M': M, 'luts': luts, 'terms': terms}

SHOTS = [('21','control',None),('1','temperature',-100),('3','temperature',100),('4','tint',-100),('5','tint',100),
 ('6','exposure',-5),('7','exposure',-3.24),('8','exposure',1.32),('9','exposure',5),
 ('10','highlights',-100),('11','highlights',100),('12','shadows',-100),('13','shadows',100),
 ('14','whites',-100),('15','whites',100),('16','blacks',-100),('17','blacks',100),
 ('18','dehaze',-100),('19','dehaze',100),('20','vibrance',-100),('22','vibrance',100),
 ('23','saturation',-100),('24','saturation',100),
 ('31','redHue',-100),('28','greenHue',-100),('30','greenHue',100),('25','blueHue',-100),('27','blueHue',100)]

r4 = lambda x, d: [round(float(v), d) for v in np.ravel(x)]
v2, v3 = {}, {}
e2s, e3s, r2s, r3s = [], [], [], []
for n, key, value in SHOTS:
    a = build_v2(n, 20)
    v2[n] = {'key': key, 'value': value, 'kind': 'p' if a['terms'] == 6 else 'm',
             'm': [r4(row, 6) for row in a['M']], 'l': [r4(row, 4) for row in a['luts']]}
    c = build(n, *LAM)
    v3[n] = {'key': key, 'value': value, 'terms': c['terms'], 'space': c['space'],
             'm': [r4(row, 6) for row in c['M']], 'l': [r4(row, 4) for row in c['luts']],
             'hue': [r4(row, 4) for row in c['hue']], 'lum': [r4(row, 4) for row in c['lum']]}
    e2s.append(a['err']); e3s.append(c['err']); r2s.append(a['band']); r3s.append(c['band'])
    print(f"#{n:3s} {key:12s} {str(value):>7s}  v2 {a['err']:5.2f} (bandes {a['band']:.2f})   v3 {c['err']:5.2f} (bandes {c['band']:.2f})")

json.dump(v2, open('v2s.json', 'w'), separators=(',', ':'))
json.dump(v3, open('v3s.json', 'w'), separators=(',', ':'))
import os
print(f"\nv2 moyenne {np.mean(e2s):.2f} bandes {np.mean(r2s):.2f}   |   v3 moyenne {np.mean(e3s):.2f} bandes {np.mean(r3s):.2f}")
print(f"référence bandes {banding(REF.reshape(-1,3), REF.shape[0]):.2f}")
print(f"{os.path.getsize('v2s.json')/1024:.0f} Ko + {os.path.getsize('v3s.json')/1024:.0f} Ko")
