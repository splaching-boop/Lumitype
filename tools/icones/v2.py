from icons import ICONS, S, m
# ---- Lier les calques : deux maillons de chaîne qui s'entrelacent (chaque anneau passe sous l'autre d'un côté).
LINK = (m("link", '<circle cx="10.2" cy="10.2" r="1.9" fill="#000"/>')
        + '<rect x="2.5" y="12" width="12" height="7" rx="3.5" transform="rotate(-45 8.5 15.5)" ' + S + ' stroke-width="2.6" mask="url(#m-link)"/>'
        + m("link2", '<circle cx="13.8" cy="13.8" r="1.9" fill="#000"/>')
        + '<rect x="9.5" y="5" width="12" height="7" rx="3.5" transform="rotate(-45 15.5 8.5)" ' + S + ' stroke-width="2.6" mask="url(#m-link2)"/>')
# ---- Pipette, façon Photoshop : poire en haut à droite, bague, tube de verre effilé vers la pointe.
PIPETTE = ('<g transform="rotate(45 12 12)"><rect x="9.4" y="0.8" width="5.2" height="6.6" rx="2.6"/>'
           '<rect x="7.4" y="8" width="9.2" height="3" rx="1.5"/><path d="M10 11.8h4v5.8l-2 4.6-2-4.6z"/></g>')
# ---- Plume, simplifiée : une pointe pleine et un seul œil ; les variantes portent un signe en bas à droite.
# Pointe de plume : bord supérieur plat (le corps du stylo), épaules, pointe en bas ; un seul œil.
# Pointe de plume : bord supérieur plat (le corps du stylo), épaules, pointe en bas ; un seul œil. Penchée à 45°,
# pointe vers le bas à gauche, comme la plume de Photoshop (le masque, défini dans le repère de l'icône, est
# tourné avec elle).
NIB = ('<mask id="m-nib" maskUnits="userSpaceOnUse" x="0" y="0" width="24" height="24"><rect width="24" height="24" fill="#fff"/>'
       '<circle cx="10" cy="11.4" r="1.8" fill="#000" transform="rotate(45 10 12)"/></mask>'
       '<path mask="url(#m-nib)" transform="rotate(45 10 12)" d="M6.4 2.2h7.2l3.4 7.6L10 22 3 9.8z"/>')
BADGE = lambda d: '<path d="' + d + '" ' + S + ' stroke-width="2.4"/>'
PEN = '<g transform="translate(2 1)">' + NIB + '</g>'
PEN_ADD = '<g transform="translate(-1.5 -1.5)">' + NIB.replace('m-nib', 'm-nibA') + '</g>' + BADGE("M19.5 15.5v6.5M16.25 18.75h6.5")
PEN_DELETE = '<g transform="translate(-1.5 -1.5)">' + NIB.replace('m-nib', 'm-nibD') + '</g>' + BADGE("M16.25 18.75h6.5")
PEN_CONVERT = '<path d="M4 20l8-15 8 15" ' + S + ' stroke-width="2.8"/><rect x="9.6" y="2.6" width="4.8" height="4.8" rx="1" />'
PEN_FREE = '<g transform="translate(-1.5 -1.5)">' + NIB.replace('m-nib', 'm-nibF') + '</g>' + BADGE("M15 21.5c1.4-2.8 2.8-2.8 3.8 0s2.4 2.8 3.8 0")
# ---- Verrou, simplifié : corps plein et anse, sans serrure.
LOCK = '<rect x="4" y="10.4" width="16" height="11.6" rx="3.2"/><path d="M7.5 10.6V7.8a4.5 4.5 0 0 1 9 0v2.8" ' + S + ' stroke-width="3"/>'

NEW = {
  "link": ("Lier les calques", LINK),
  "pipette": ("Pipette", PIPETTE),
  "pen": ("Plume", PEN),
  "lockAll": ("Verrou : tout", LOCK),
  "penAdd": ("Ajouter un point", PEN_ADD),
  "penDelete": ("Supprimer un point", PEN_DELETE),
  "penConvert": ("Convertir le point", PEN_CONVERT),
  "penFree": ("Plume libre", PEN_FREE),
}
