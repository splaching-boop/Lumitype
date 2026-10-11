use std::io;
use std::os::raw::*;
use std::path::{Path, PathBuf};
use std::str::Utf8Error;
use std::sync::Arc;

use dpi::PhysicalPosition;
use percent_encoding::percent_decode;
use x11rb::protocol::xproto::{self, ConnectionExt};

use crate::atoms::AtomName::None as DndNone;
use crate::atoms::*;
use crate::event_loop::{CookieResultExt, X11Error};
use crate::util;
use crate::xdisplay::XConnection;

#[derive(Debug, Clone, Copy)]
pub enum DndState {
    Accepted,
    Rejected,
}

#[derive(Debug)]
pub enum DndDataParseError {
    EmptyData,
    InvalidUtf8(#[allow(dead_code)] Utf8Error),
    HostnameSpecified(#[allow(dead_code)] String),
    UnexpectedProtocol(#[allow(dead_code)] String),
    UnresolvablePath(#[allow(dead_code)] io::Error),
}

impl From<Utf8Error> for DndDataParseError {
    fn from(e: Utf8Error) -> Self {
        DndDataParseError::InvalidUtf8(e)
    }
}

impl From<io::Error> for DndDataParseError {
    fn from(e: io::Error) -> Self {
        DndDataParseError::UnresolvablePath(e)
    }
}

#[derive(Debug)]
pub struct Dnd {
    xconn: Arc<XConnection>,
    // Populated by XdndEnter event handler
    pub version: Option<c_long>,
    pub type_list: Option<Vec<xproto::Atom>>,
    // Populated by XdndPosition event handler
    pub source_window: Option<xproto::Window>,
    // Populated by XdndPosition event handler
    pub position: PhysicalPosition<f64>,
    // Populated by SelectionNotify event handler (triggered by XdndPosition event handler)
    pub result: Option<Result<Vec<PathBuf>, DndDataParseError>>,
    // Populated by SelectionNotify event handler (triggered by XdndPosition event handler)
    pub dragging: bool,
    // Lumitype : type demandé à la source (`text/uri-list`, puis `text/html` pour un glisser venu du web).
    pub target_type: Option<xproto::Atom>,
}

impl Dnd {
    pub fn new(xconn: Arc<XConnection>) -> Result<Self, X11Error> {
        Ok(Dnd {
            xconn,
            version: None,
            type_list: None,
            source_window: None,
            position: PhysicalPosition::default(),
            result: None,
            dragging: false,
            target_type: None,
        })
    }

    pub fn reset(&mut self) {
        self.version = None;
        self.type_list = None;
        self.source_window = None;
        self.result = None;
        self.dragging = false;
        self.target_type = None;
    }

    /// Lumitype : type actuellement demandé à la source (la liste d'adresses par défaut).
    pub fn requested_type(&self) -> xproto::Atom {
        self.target_type.unwrap_or(self.xconn.atoms()[TextUriList])
    }

    /// Lumitype : vrai si la source propose aussi du HTML (glisser venu d'un navigateur).
    pub fn offers_html(&self) -> bool {
        let html = self.xconn.atoms()[TextHtml];
        self.type_list.as_ref().is_some_and(|list| list.contains(&html))
    }

    pub unsafe fn send_status(
        &self,
        this_window: xproto::Window,
        target_window: xproto::Window,
        state: DndState,
    ) -> Result<(), X11Error> {
        let atoms = self.xconn.atoms();
        let (accepted, action) = match state {
            DndState::Accepted => (1, atoms[XdndActionPrivate]),
            DndState::Rejected => (0, atoms[DndNone]),
        };
        self.xconn
            .send_client_msg(target_window, target_window, atoms[XdndStatus] as _, None, [
                this_window,
                accepted,
                0,
                0,
                action as _,
            ])?
            .ignore_error();

        Ok(())
    }

    pub unsafe fn send_finished(
        &self,
        this_window: xproto::Window,
        target_window: xproto::Window,
        state: DndState,
    ) -> Result<(), X11Error> {
        let atoms = self.xconn.atoms();
        let (accepted, action) = match state {
            DndState::Accepted => (1, atoms[XdndActionPrivate]),
            DndState::Rejected => (0, atoms[DndNone]),
        };
        self.xconn
            .send_client_msg(target_window, target_window, atoms[XdndFinished] as _, None, [
                this_window,
                accepted,
                action as _,
                0,
                0,
            ])?
            .ignore_error();

        Ok(())
    }

    pub unsafe fn get_type_list(
        &self,
        source_window: xproto::Window,
    ) -> Result<Vec<xproto::Atom>, util::GetPropertyError> {
        let atoms = self.xconn.atoms();
        self.xconn.get_property(
            source_window,
            atoms[XdndTypeList],
            xproto::Atom::from(xproto::AtomEnum::ATOM),
        )
    }

    pub unsafe fn convert_selection(&self, window: xproto::Window, time: xproto::Timestamp) {
        let atoms = self.xconn.atoms();
        self.xconn
            .xcb_connection()
            .convert_selection(
                window,
                atoms[XdndSelection],
                self.requested_type(),
                atoms[XdndSelection],
                time,
            )
            .expect_then_ignore_error("Failed to send XdndSelection event")
    }

    pub unsafe fn read_data(
        &self,
        window: xproto::Window,
    ) -> Result<Vec<c_uchar>, util::GetPropertyError> {
        let atoms = self.xconn.atoms();
        if self.requested_type() == atoms[TextUriList] {
            return self.xconn.get_property(window, atoms[XdndSelection], atoms[TextUriList]);
        }
        // Lumitype : le HTML arrive sous le type annoncé par la source (souvent `text/html`, parfois `UTF8_STRING`) :
        // lu tel quel, quel que soit son type.
        let reply = self
            .xconn
            .xcb_connection()
            .get_property(false, window, atoms[XdndSelection], xproto::AtomEnum::ANY, 0, u32::MAX / 4)?
            .reply()?;
        Ok(reply.value)
    }

    /// Lumitype : adresse de la première image (`<img src="…">`) d'un fragment HTML glissé depuis un navigateur.
    /// Le HTML peut être en UTF-16 (Firefox, avec marque d'ordre des octets) ou en UTF-8.
    pub fn parse_html_image(data: &[c_uchar]) -> Option<String> {
        let html = if data.starts_with(&[0xFF, 0xFE]) {
            let units: Vec<u16> = data[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        } else {
            String::from_utf8_lossy(data).into_owned()
        };
        let lower = html.to_ascii_lowercase();
        let tag = lower.find("<img")?;
        let attr = tag + lower[tag..].find("src=")? + 4;
        let rest = &html[attr..];
        let (quote, rest) = match rest.chars().next()? {
            q @ ('"' | '\'') => (q, &rest[1..]),
            _ => (' ', rest),
        };
        let end = rest.find(|c: char| c == quote || (quote == ' ' && c == '>')).unwrap_or(rest.len());
        let url = rest[..end].trim().replace("&amp;", "&");
        (url.starts_with("http://") || url.starts_with("https://")).then_some(url)
    }

    pub fn parse_data(&self, data: &mut [c_uchar]) -> Result<Vec<PathBuf>, DndDataParseError> {
        if !data.is_empty() {
            let mut path_list = Vec::new();
            let decoded = percent_decode(data).decode_utf8()?.into_owned();
            for uri in decoded.split("\r\n").filter(|u| !u.is_empty()) {
                // The format is specified as protocol://host/path
                // However, it's typically simply protocol:///path
                let path_str = if uri.starts_with("file://") {
                    let path_str = uri.replace("file://", "");
                    if !path_str.starts_with('/') {
                        // A hostname is specified
                        // Supporting this case is beyond the scope of my mental health
                        return Err(DndDataParseError::HostnameSpecified(path_str));
                    }
                    path_str
                } else if uri.starts_with("http://") || uri.starts_with("https://") {
                    // Lumitype : adresse web (image glissée depuis un navigateur), transmise telle quelle (non
                    // décodée : les `%20` de l'adresse sont à garder pour la télécharger).
                    let raw = data_line(data, uri);
                    path_list.push(PathBuf::from(raw));
                    continue;
                } else if uri.starts_with('#') {
                    continue; // commentaire de la liste d'adresses (RFC 2483)
                } else {
                    // Only the file protocol is supported
                    return Err(DndDataParseError::UnexpectedProtocol(uri.to_owned()));
                };

                let path = Path::new(&path_str).canonicalize()?;
                path_list.push(path);
            }
            Ok(path_list)
        } else {
            Err(DndDataParseError::EmptyData)
        }
    }
}

/// Lumitype : la ligne d'origine (avant décodage `%xx`) qui correspond à `decoded` dans la liste d'adresses.
fn data_line(data: &[c_uchar], decoded: &str) -> String {
    String::from_utf8_lossy(data)
        .split("\r\n")
        .map(str::trim)
        .find(|line| percent_decode(line.as_bytes()).decode_utf8().is_ok_and(|d| d == decoded))
        .unwrap_or(decoded)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{Dnd, data_line};

    #[test]
    fn image_from_html() {
        let html = br#"<meta charset="utf-8"><a href="https://site.fr/page"><img alt="x" SRC="https://i.site.fr/a.jpg?w=1&amp;h=2"></a>"#;
        assert_eq!(Dnd::parse_html_image(html).as_deref(), Some("https://i.site.fr/a.jpg?w=1&h=2"));
        let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain("<img src='https://x.fr/b.png'>".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(Dnd::parse_html_image(&utf16).as_deref(), Some("https://x.fr/b.png"));
        assert_eq!(Dnd::parse_html_image(b"<img src=data:image/png;base64,AAA>"), None);
        assert_eq!(Dnd::parse_html_image(b"<p>texte</p>"), None);
    }

    #[test]
    fn web_address_kept_encoded() {
        let data = b"https://x.fr/mon%20image.jpg\r\n";
        assert_eq!(data_line(data, "https://x.fr/mon image.jpg"), "https://x.fr/mon%20image.jpg");
    }
}
