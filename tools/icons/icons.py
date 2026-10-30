# Jeu d'icônes « géométrique plein » pour Lumitype : 24 × 24, formes pleines et évidements, traits épais arrondis.
S = 'fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"'
def m(name, *holes):
    """Masque : blanc partout, noir sur les évidements (pour creuser une forme pleine)."""
    return f'<mask id="m-{name}" maskUnits="userSpaceOnUse" x="0" y="0" width="24" height="24"><rect width="24" height="24" fill="#fff"/>{"".join(holes)}</mask>'
MOVE = '<path d="M12 1.5l3.4 4.4H8.6zM12 22.5l-3.4-4.4h6.8zM1.5 12l4.4-3.4v6.8zM22.5 12l-4.4 3.4V8.6z"/><rect x="10.9" y="5" width="2.2" height="14" rx="1.1"/><rect x="5" y="10.9" width="14" height="2.2" rx="1.1"/><circle cx="12" cy="12" r="3"/>'
BRUSH = '<path d="M2.8 21.2c.3-3.3 1.4-6.7 5.1-7.1 2.6-.3 4.2 1.5 3.8 3.6-.6 3.4-5.4 4.1-8.9 3.5z"/><path d="M12 12.4L20.6 3.8" ' + S + ' stroke-width="3.2"/>'
ICONS = {
  # ---------------------------------------------------------------- outils
  "move": ("Déplacement", MOVE),
  "marquee": ("Sélection rectangulaire", '<path d="M3 3h6.5v2.8H5.8v3.7H3zM21 3h-6.5v2.8h3.7v3.7H21zM3 21h6.5v-2.8H5.8v-3.7H3zM21 21h-6.5v-2.8h3.7v-3.7H21z"/><circle cx="12" cy="12" r="1.6"/>'),
  "lasso": ("Lasso", '<path fill-rule="evenodd" d="M2.5 9.5a9.5 6.5 0 1 0 19 0a9.5 6.5 0 1 0-19 0zM5.4 9.5a6.6 3.8 0 1 0 13.2 0a6.6 3.8 0 1 0-13.2 0z"/><path d="M7.6 15.4L5.4 20" ' + S + ' stroke-width="2.6"/><circle cx="5" cy="20.6" r="2"/>'),
  "wand": ("Baguette magique", '<path d="M4 20L13.8 10.2" ' + S + ' stroke-width="3.4"/><path d="M17.5 1.5q.7 4.3 5 5q-4.3.7-5 5q-.7-4.3-5-5q4.3-.7 5-5z"/>'),
  "quickSelect": ("Sélection rapide", '<path d="M4.2 19.8c-.8-4.6 4.6-10.6 10.4-13.6 1.4 4.4 1.6 9.9-2.6 12.9-2.6 1.9-6.9 2.9-7.8.7z"/><path d="M19 1.5q.5 3 3.5 3.5q-3 .5-3.5 3.5q-.5-3-3.5-3.5q3-.5 3.5-3.5z"/>'),
  "crop": ("Recadrage", '<path d="M5 1.8h3.2v14h14V19H5z"/><path d="M1.8 5H19v17.2h-3.2V8.2h-14z"/>'),
  "pipette": ("Pipette", '<circle cx="17.3" cy="6.7" r="4.4"/><path d="M14.4 9.6L4.2 19.8" ' + S + ' stroke-width="3"/><circle cx="3.8" cy="20.2" r="1.3"/>'),
  "heal": ("Correcteur localisé", m("heal", '<rect x="10.2" y="10.2" width="3.6" height="3.6" rx=".8" fill="#000" transform="rotate(45 12 12)"/>', '<circle cx="8.6" cy="15.4" r=".9" fill="#000"/><circle cx="15.4" cy="8.6" r=".9" fill="#000"/>') + '<path d="M7.3 16.7L16.7 7.3" ' + S + ' stroke-width="8.4" mask="url(#m-heal)"/>'),
  "brush": ("Pinceau", BRUSH),
  "eraser": ("Gomme", m("eraser", '<path d="M8 9.8L14.2 16" stroke="#000" stroke-width="1.8"/>') + '<path mask="url(#m-eraser)" d="M2.9 14.3l8.6-8.6a2.6 2.6 0 0 1 3.7 0l4.1 4.1a2.6 2.6 0 0 1 0 3.7L12 20.8H9.4z"/><rect x="11" y="19.6" width="10.5" height="2.4" rx="1.2"/>'),
  "bucket": ("Dégradé / Pot", '<path d="M10.5 1.8l8.7 8.7-8.7 8.7L1.8 10.5z"/><path d="M20.6 13.8c1.3 2 2 3.4 2 4.3a2 2 0 0 1-4 0c0-.9.7-2.3 2-4.3z"/>'),
  "pen": ("Plume", m("pen", '<circle cx="12" cy="12.4" r="2.1" fill="#000"/>', '<rect x="11.2" y="1" width="1.6" height="10" fill="#000"/>') + '<path mask="url(#m-pen)" d="M12 1.8l6.8 9.8L12 22.2 5.2 11.6z"/>'),
  "pathSelect": ("Sélection directe", '<path d="M5 2.3v18.2l4.9-4.7 3.3 6.8 3.1-1.5-3.2-6.7 6.8-.3z"/>'),
  "type": ("Texte", '<path d="M3 3h18v5.2h-3V6.4h-4.3v12.4h2.6V21H7.7v-2.2h2.6V6.4H6v1.8H3z"/>'),
  "shape": ("Forme", m("shape", '<circle cx="17.2" cy="6.8" r="6.9" fill="#000"/>') + '<rect x="2.2" y="7.4" width="14.6" height="14.6" rx="3.2" mask="url(#m-shape)"/><circle cx="17.2" cy="6.8" r="4.9"/>'),
  "hand": ("Main", '<path d="M7 13V8.2M10.5 12V4.6M14 12V4M17.5 13V6.6" ' + S + ' stroke-width="3.1"/><path d="M5.4 11.6h13.9v4.4a6.2 6.2 0 0 1-6.2 6.2h-1.5a6.2 6.2 0 0 1-6.2-6.2z"/><path d="M5.8 15.4L3.2 11.8" ' + S + ' stroke-width="3.1"/>'),
  "zoom": ("Zoom", '<circle cx="10" cy="10" r="6.6" ' + S + ' stroke-width="3.2"/><path d="M15.2 15.2l5.6 5.6" ' + S + ' stroke-width="3.8"/>'),
  # ---------------------------------------------------------------- panneau Calques
  "eye": ("Visibilité", m("eye", '<circle cx="12" cy="12" r="5" fill="#000"/>', '<circle cx="12" cy="12" r="2.7" fill="#fff"/>') + '<path mask="url(#m-eye)" d="M1.3 12C4.2 6.8 8 4.3 12 4.3s7.8 2.5 10.7 7.7c-2.9 5.2-6.7 7.7-10.7 7.7S4.2 17.2 1.3 12z"/>'),
  "chevron": ("Déplier un groupe", '<path d="M9 4.8l7.2 7.2L9 19.2" ' + S + ' stroke-width="3.4"/>'),
  "clip": ("Flèche d'écrêtage", '<path d="M6.5 2.5v8.8a4.2 4.2 0 0 0 4.2 4.2H16" ' + S + ' stroke-width="3"/><path d="M15.4 10l6.6 5.5-6.6 5.5z"/>'),
  "textLayer": ("Calque de texte", m("textLayer", '<path d="M7 7h10v3.2h-3.4V17h-3.2v-6.8H7z" fill="#000"/>') + '<rect x="2.5" y="2.5" width="19" height="19" rx="4.5" mask="url(#m-textLayer)"/>'),
  "shapeRect": ("Forme : rectangle", '<rect x="2" y="5.5" width="20" height="13" rx="3.4"/>'),
  "shapeEllipse": ("Forme : ellipse", '<ellipse cx="12" cy="12" rx="10.2" ry="7.2"/>'),
  "shapePolygon": ("Forme : polygone", '<path d="M12 3c.8 0 1.6.4 2 1.1l7.5 13.3a2.3 2.3 0 0 1-2 3.4h-15a2.3 2.3 0 0 1-2-3.4L10 4.1C10.4 3.4 11.2 3 12 3z"/>'),
  "group": ("Groupe", '<path d="M2 6.6A2.6 2.6 0 0 1 4.6 4h5l2.3 2.6h7.5A2.6 2.6 0 0 1 22 9.2v9.2A2.6 2.6 0 0 1 19.4 21H4.6A2.6 2.6 0 0 1 2 18.4z"/>'),
  "adjustment": ("Calque de réglage", '<circle cx="12" cy="12" r="9.2" ' + S + ' stroke-width="2.6"/><path d="M12 5.4a6.6 6.6 0 0 0 0 13.2z"/>'),
  "lockTransparency": ("Verrou : transparence", '<rect x="2.5" y="2.5" width="9" height="9" rx="2"/><rect x="12.5" y="12.5" width="9" height="9" rx="2"/><rect x="13.7" y="3.7" width="6.6" height="6.6" rx="1.4" ' + S + ' stroke-width="2.2"/><rect x="3.7" y="13.7" width="6.6" height="6.6" rx="1.4" ' + S + ' stroke-width="2.2"/>'),
  "lockPixels": ("Verrou : pixels", BRUSH),
  "lockPosition": ("Verrou : position", MOVE),
  "lockAll": ("Verrou : tout", m("lockAll", '<circle cx="12" cy="15" r="2" fill="#000"/>', '<rect x="11.1" y="15" width="1.8" height="3.8" rx=".9" fill="#000"/>') + '<rect x="3.8" y="10" width="16.4" height="12" rx="3.2" mask="url(#m-lockAll)"/><path d="M7.4 10.4V7.6a4.6 4.6 0 0 1 9.2 0v2.8" ' + S + ' stroke-width="2.8"/>'),
  "link": ("Lier les calques", '<rect x="1.8" y="8.6" width="12" height="6.8" rx="3.4" transform="rotate(-45 7.8 12)" ' + S + ' stroke-width="2.6"/><rect x="10.2" y="8.6" width="12" height="6.8" rx="3.4" transform="rotate(-45 16.2 12)" ' + S + ' stroke-width="2.6"/>'),
  "fx": ("Effets (fx)", '<path d="M11.6 3.6h-1.4a2.6 2.6 0 0 0-2.6 2.6v14.2M4.6 10.2h6" ' + S + ' stroke-width="2.8"/><path d="M13.6 10.4l6.8 9.8M20.4 10.4l-6.8 9.8" ' + S + ' stroke-width="2.8"/>'),
  "mask": ("Masque de fusion", '<path fill-rule="evenodd" d="M2.5 7A4.5 4.5 0 0 1 7 2.5h10A4.5 4.5 0 0 1 21.5 7v10a4.5 4.5 0 0 1-4.5 4.5H7A4.5 4.5 0 0 1 2.5 17zM12 7.2a4.8 4.8 0 1 0 0 9.6a4.8 4.8 0 1 0 0-9.6z"/>'),
  "newLayer": ("Nouveau calque", m("newLayer", '<path d="M14 10.2v7.6M10.2 14h7.6" stroke="#000" stroke-width="2.4" stroke-linecap="round"/>') + '<rect x="7" y="7" width="14.5" height="14.5" rx="3.2" mask="url(#m-newLayer)"/><path d="M3 14.8V5.6A2.6 2.6 0 0 1 5.6 3h9.2" ' + S + ' stroke-width="2.4"/>'),
  "trash": ("Supprimer", m("trash", '<rect x="9" y="11.4" width="2" height="7.2" rx="1" fill="#000"/>', '<rect x="13" y="11.4" width="2" height="7.2" rx="1" fill="#000"/>') + '<rect x="2.8" y="4.6" width="18.4" height="2.8" rx="1.4"/><rect x="8.6" y="1.8" width="6.8" height="3.6" rx="1.4"/><path mask="url(#m-trash)" d="M4.8 9h14.4l-1.3 11.1a2.1 2.1 0 0 1-2.1 1.9H8.2a2.1 2.1 0 0 1-2.1-1.9z"/>'),
  # ---------------------------------------------------------------- divers
  "close": ("Fermer", '<path d="M5 5l14 14M19 5L5 19" ' + S + ' stroke-width="3.4"/>'),
  "caret": ("Liste déroulante", '<path d="M4.8 8.6l7.2 7.2 7.2-7.2" ' + S + ' stroke-width="3.4"/>'),
  "plus": ("Ajouter", '<path d="M12 4v16M4 12h16" ' + S + ' stroke-width="3.4"/>'),
}
GROUPS = [
  ("Outils", ["move", "marquee", "lasso", "wand", "quickSelect", "crop", "pipette", "heal", "brush", "eraser", "bucket", "pen", "pathSelect", "type", "shape", "hand", "zoom"]),
  ("Panneau Calques", ["eye", "chevron", "clip", "textLayer", "shapeRect", "shapeEllipse", "shapePolygon", "group", "adjustment", "lockTransparency", "lockPixels", "lockPosition", "lockAll", "link", "fx", "mask", "newLayer", "trash"]),
  ("Divers", ["close", "caret", "plus"]),
]
