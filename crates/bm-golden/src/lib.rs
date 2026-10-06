//! Test oracle, not product code: strict parsers for BlueMap's output and a face-level render diff, used to
//! compare bluemap-rs renders against Java BlueMap's golden output (tools/render_golden.py).

pub mod diff;
pub mod lowres;
mod parse;
mod reader;
pub mod roundtrip;
pub mod settings;
#[cfg(test)]
mod test_prbm;
pub mod textures;
pub mod tile;
pub mod webroot;

pub use parse::parse;
pub use textures::{Texture, parse_texture_names, parse_textures};
pub use tile::{Face, Group, Tile};
pub use webroot::WebrootMap;
