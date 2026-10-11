# winit-x11 0.31.0-beta.2 — copie locale de Lumitype

Copie du crate publié `winit-x11 0.31.0-beta.2`, branchée par `[patch.crates-io]` dans le `Cargo.toml` racine.
Elle ne sert qu'au runtime Chromium sous Linux/X11 (`tauri-runtime-cef`, qui fige `winit =0.31.0-beta.2`).

## Pourquoi

Sous GNOME (Wayland + XWayland), un glisser venu de Nautilus ou d'un navigateur est remis par Mutter à la fenêtre
de premier niveau, celle de winit (`XdndAware`). La page (Chromium) ne le voit pas : Lumitype l'écoute par les
événements Tauri `tauri://drag-*` (`onDragDropEvent`). Or winit d'origine ne demande que `text/uri-list` et rejette
toute adresse qui n'est pas `file://` : une image glissée depuis un site était perdue.

## Changements (repérés par « Lumitype » dans le code)

- `src/atoms.rs` : atome `TextHtml` (`text/html`).
- `src/dnd.rs` :
  - `parse_data` laisse passer les adresses `http(s)://`, telles qu'envoyées (non décodées), et ignore les
    commentaires `#` ;
  - `target_type` / `requested_type` : type demandé à la source (`text/uri-list`, puis `text/html`) ;
  - `read_data` lit le HTML quel que soit son type de propriété ;
  - `parse_html_image` : première `<img src>` (UTF-8 ou UTF-16 avec BOM).
- `src/event_processor.rs` :
  - un glisser qui ne propose que du HTML est accepté ;
  - après une liste d'adresses sans fichier local, les positions suivantes demandent le HTML. Son image passe en
    tête des chemins, devant l'adresse de la page.

## Mise à jour de winit

Si `tauri-runtime-cef` passe à une autre version de winit, recopier le nouveau `winit-x11` et reporter ces
changements (ou retirer le `[patch]` si winit gère enfin les adresses web).
