//! Moteur d'image en tuiles.
//!
//! Ce crate ne dépend ni de Tauri ni de l'interface : il peut être testé et
//! mesuré seul (`cargo test -p engine`, `cargo bench -p engine`).
//!
//! Conventions valables partout :
//! - pixels en RGBA 8 bits, **alpha prémultiplié**, espace sRGB non linéaire ;
//! - image découpée en tuiles carrées de [`tiles::TILE_SIZE`] pixels ;
//! - une tuile entièrement transparente n'est jamais stockée.

pub mod adjust;
pub mod blend;
pub mod brushes;
pub mod calibration;
pub mod document;
pub mod effects;
pub mod error;
pub mod filters;
pub mod heal;
pub mod io;
pub mod memory;
pub mod metadata;
pub mod paint;
pub mod path;
pub mod pixel;
pub mod plan;
pub mod png_parallel;
pub mod render;
pub mod scratch;
pub mod selection;
pub mod session;
pub mod shape;
pub mod smart;
pub mod text;
pub mod tiles;
pub mod transform;

pub use blend::BlendMode;
pub use document::{Document, Layer, LayerId};
pub use error::EngineError;
pub use memory::{memory_cap, memory_status, set_memory_cap, MemoryStatus};
pub use render::TileRenderer;
pub use session::{Dirty, Session};
pub use tiles::{MaskStore, Tile, TileCoord, TileStore, TILE_BYTES, TILE_SIZE};
