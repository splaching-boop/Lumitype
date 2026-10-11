"""Lissage des tables ajustées.

Une transformation couleur est une fonction : appliquée à un dégradé lisse, elle ne peut
ressortir lisse que si elle est elle-même lisse. Les médianes par tranche ne le sont pas —
d'où les stries. On les remplace par un lissage de Whittaker : on cherche la courbe qui
suit les données tout en pénalisant sa propre courbure.
"""
import numpy as np

def whittaker(y, w, lam, circular=False):
    n = len(y)
    if circular:
        D = np.zeros((n, n))
        for i in range(n):
            D[i, (i - 1) % n] = 1; D[i, i] = -2; D[i, (i + 1) % n] = 1
    else:
        D = np.zeros((n - 2, n))
        for i in range(n - 2):
            D[i, i] = 1; D[i, i + 1] = -2; D[i, i + 2] = 1
    W = np.diag(w)
    return np.linalg.solve(W + lam * D.T @ D, W @ y)

def roughness(img, h):
    """Dérivée seconde moyenne le long de l'axe horizontal, dans la bande arc-en-ciel.
    C'est la mesure de « strie » : plus c'est bas, plus le dégradé est lisse."""
    a = img.reshape(h, 900, 3)
    band = a[int(h * 0.15):int(h * 0.65)]
    d2 = band[:, 2:] - 2 * band[:, 1:-1] + band[:, :-2]
    return np.abs(d2).mean() * 255

def banding(img, h):
    """Ondulation de basse fréquence le long du dégradé : c'est elle qu'on voit comme des
    bandes verticales. On moyenne la bande arc-en-ciel verticalement, on lisse sur 9 pixels
    pour écarter le bruit fin, puis on mesure la dérivée seconde de ce qui reste."""
    a = img.reshape(h, 900, 3)
    row = a[int(h * 0.15):int(h * 0.65)].mean(axis=0)          # 900 x 3
    k = np.ones(9) / 9
    sm = np.stack([np.convolve(row[:, c], k, mode='valid') for c in range(3)], 1)
    d2 = sm[2:] - 2 * sm[1:-1] + sm[:-2]
    return np.abs(d2).mean() * 255 * 100                        # x100 : valeurs lisibles

def isotonic(y, w):
    """Régression isotone (pool adjacent violators) : la plus proche courbe croissante.
    Une courbe de transfert ne peut pas redescendre quand l'entrée monte — un creux
    se voit immédiatement comme une bande dans un dégradé."""
    y = np.asarray(y, float).copy(); w = np.asarray(w, float).copy()
    n = len(y)
    val = y.copy(); wt = np.maximum(w, 1e-9); size = np.ones(n, int)
    i = 0
    idx = list(range(n))
    stack_v, stack_w, stack_n = [], [], []
    for k in range(n):
        v, ww, nn = val[k], wt[k], 1
        while stack_v and stack_v[-1] > v:
            pv, pw, pn = stack_v.pop(), stack_w.pop(), stack_n.pop()
            v = (pv * pw + v * ww) / (pw + ww); ww = pw + ww; nn = pn + nn
        stack_v.append(v); stack_w.append(ww); stack_n.append(nn)
    out = np.empty(n); pos = 0
    for v, nn in zip(stack_v, stack_n):
        out[pos:pos + nn] = v; pos += nn
    return out
