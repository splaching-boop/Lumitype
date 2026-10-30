//! Styles de caractère : des réglages de texte nommés (police, graisse, corps, approche, couleur, casse…), rangés
//! dans le document. Appliquer un style RECOPIE ses réglages dans le texte (calque entier ou portion) et note son
//! identifiant : le rendu ne dépend jamais de la liste des styles, et un fichier reste juste même sans elle.
//! Modifier un style peut ensuite mettre à jour les textes qui l'utilisent (`Document::set_text_styles`).

use serde::{Deserialize, Serialize};

use crate::text::{StyleRun, TextCaps, TextData};

/// Un style de caractère. Un champ absent hérite du style « Basé sur » (`based_on`), puis n'est pas imposé.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharStyle {
    pub id: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub based_on: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    /// Interlignage (multiple du corps) : appliqué seulement quand le style vise le calque entier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caps: Option<TextCaps>,
    /// Raccourci (« Ctrl+Alt+1 »…), interprété par l'interface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<String>,
    /// Lien vers la bibliothèque de l'application (kit de marque) : identifiant et révision de l'entrée copiée.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library: Option<LibraryLink>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryLink {
    pub uid: String,
    pub revision: u64,
}

/// Réglages effectifs d'un style, héritage compris.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resolved {
    pub font_family: Option<String>,
    pub weight: Option<u16>,
    pub italic: Option<bool>,
    pub size: Option<f32>,
    pub line_height: Option<f32>,
    pub tracking: Option<f32>,
    pub color: Option<[u8; 3]>,
    pub caps: Option<TextCaps>,
}

/// Réglages du style `id`, en suivant « Basé sur » (sans boucle : un style déjà vu arrête la chaîne).
pub fn resolve(styles: &[CharStyle], id: u32) -> Option<Resolved> {
    let mut out = Resolved::default();
    let mut seen = Vec::new();
    let mut current = Some(id);
    let mut found = false;
    while let Some(at) = current {
        if seen.contains(&at) {
            break;
        }
        seen.push(at);
        let Some(style) = styles.iter().find(|s| s.id == at) else { break };
        found = true;
        out.font_family = out.font_family.or_else(|| style.font_family.clone());
        out.weight = out.weight.or(style.weight);
        out.italic = out.italic.or(style.italic);
        out.size = out.size.or(style.size);
        out.line_height = out.line_height.or(style.line_height);
        out.tracking = out.tracking.or(style.tracking);
        out.color = out.color.or(style.color);
        out.caps = out.caps.or(style.caps);
        current = style.based_on;
    }
    found.then_some(out)
}

/// Styles touchés par un changement de `id` : lui-même et ceux qui en héritent (directement ou non).
pub fn dependents(styles: &[CharStyle], id: u32) -> Vec<u32> {
    let mut out = vec![id];
    loop {
        let before = out.len();
        for style in styles {
            if style.based_on.is_some_and(|b| out.contains(&b)) && !out.contains(&style.id) {
                out.push(style.id);
            }
        }
        if out.len() == before {
            return out;
        }
    }
}

/// Réapplique à `text` les styles de `ids` (réglages `styles`) là où le texte les utilise. Retourne vrai si le texte
/// a changé. Calque entier : les réglages du style remplacent ceux du calque, et les portions perdent ces réglages
/// (comme quand on applique le style). Portion : ses réglages sont remis à ceux du style.
pub fn reapply(text: &mut TextData, styles: &[CharStyle], ids: &[u32]) -> bool {
    let before = text.clone();
    if let Some(id) = text.style.filter(|id| ids.contains(id)) {
        match resolve(styles, id) {
            Some(resolved) => apply_to_layer(text, &resolved, id),
            None => text.style = None,
        }
    }
    for run in &mut text.runs {
        if let Some(id) = run.style.filter(|id| ids.contains(id)) {
            match resolve(styles, id) {
                Some(resolved) => apply_to_run(run, &resolved),
                None => run.style = None,
            }
        }
    }
    *text != before
}

/// Le style s'applique au calque entier.
pub fn apply_to_layer(text: &mut TextData, r: &Resolved, id: u32) {
    if let Some(v) = &r.font_family {
        text.font_family.clone_from(v);
    }
    if let Some(v) = r.weight {
        text.weight = v;
    }
    if let Some(v) = r.italic {
        text.italic = v;
    }
    if let Some(v) = r.size {
        text.size = v;
    }
    if let Some(v) = r.line_height {
        text.line_height = v;
    }
    if let Some(v) = r.tracking {
        text.tracking = v;
    }
    if let Some(v) = r.color {
        text.color = v;
    }
    if let Some(v) = r.caps {
        text.caps = v;
    }
    text.style = Some(id);
    for run in &mut text.runs {
        if run.style.is_some() {
            continue; // une portion qui a son propre style le garde
        }
        if r.font_family.is_some() {
            run.font_family = None;
        }
        if r.weight.is_some() {
            run.weight = None;
        }
        if r.italic.is_some() {
            run.italic = None;
        }
        if r.size.is_some() {
            run.size = None;
        }
        if r.tracking.is_some() {
            run.tracking = None;
        }
        if r.color.is_some() {
            run.color = None;
        }
        if r.caps.is_some() {
            run.caps = None;
        }
    }
    text.runs.retain(|run| run.style.is_some() || run.font_family.is_some() || run.weight.is_some() || run.italic.is_some() || run.size.is_some() || run.tracking.is_some() || run.color.is_some() || run.caps.is_some());
}

fn apply_to_run(run: &mut StyleRun, r: &Resolved) {
    if r.font_family.is_some() {
        run.font_family.clone_from(&r.font_family);
    }
    if r.weight.is_some() {
        run.weight = r.weight;
    }
    if r.italic.is_some() {
        run.italic = r.italic;
    }
    if r.size.is_some() {
        run.size = r.size;
    }
    if r.tracking.is_some() {
        run.tracking = r.tracking;
    }
    if r.color.is_some() {
        run.color = r.color;
    }
    if r.caps.is_some() {
        run.caps = r.caps;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(id: u32, based_on: Option<u32>) -> CharStyle {
        CharStyle { id, name: format!("S{id}"), based_on, font_family: None, weight: None, italic: None, size: None, line_height: None, tracking: None, color: None, caps: None, shortcut: None, library: None }
    }

    #[test]
    fn inheritance_and_reapplication() {
        let mut body = style(1, None);
        body.font_family = Some("Serif".into());
        body.size = Some(13.0);
        let mut accent = style(2, Some(1));
        accent.weight = Some(700);
        accent.color = Some([229, 52, 62]);
        let mut styles = vec![body, accent];
        let r = resolve(&styles, 2).expect("style");
        assert_eq!((r.font_family.as_deref(), r.size, r.weight), (Some("Serif"), Some(13.0), Some(700)));
        // Boucle « Basé sur » : pas de blocage.
        styles[0].based_on = Some(2);
        assert!(resolve(&styles, 2).is_some());
        styles[0].based_on = None;
        assert_eq!(dependents(&styles, 1), vec![1, 2]);

        let mut text = TextData { content: "Bonjour".into(), ..TextData::default() };
        apply_to_layer(&mut text, &resolve(&styles, 2).expect("style"), 2);
        assert_eq!((text.weight, text.size, text.style), (700, 13.0, Some(2)));
        // Le style « Corps » change de corps : le texte qui utilise « Accent » (basé sur Corps) suit.
        styles[0].size = Some(20.0);
        assert!(reapply(&mut text, &styles, &dependents(&styles, 1)));
        assert_eq!(text.size, 20.0);
        assert!(!reapply(&mut text, &styles, &[1, 2]), "déjà à jour");
    }
}
