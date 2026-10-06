//! What a map stores: grids of cells and single items (`MapStorage.java`, `KeyedMapStorage.java`).

use bm_compress::Compression;

/// A per-map grid of cells, addressed by tile/cell coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GridKey {
    /// PRBM hires tiles, stored with the configured compression.
    Hires,
    /// Lowres PNG tiles for LOD `n >= 1`, never compressed.
    Lowres(u32),
    /// Render-state cells (gzip NBT), always gzip regardless of configuration.
    TileState,
    ChunkState,
    RegionState,
}

/// A single per-map item.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ItemKey {
    Settings,
    /// `textures.json`, stored with the configured compression.
    Textures,
    Markers,
    Players,
    /// Asset by name, e.g. `playerheads/<uuid>.png`; escaped like BlueMap on use.
    Asset(String),
}

impl GridKey {
    pub fn compression(self, configured: Compression) -> Compression {
        match self {
            Self::Hires => configured,
            Self::Lowres(_) => Compression::None,
            Self::TileState | Self::ChunkState | Self::RegionState => Compression::Gzip,
        }
    }

    /// `Key.formatted` used as `grid_storage.key` in SQL.
    pub fn sql_key(self) -> String {
        match self {
            Self::Hires => "bluemap:hires".into(),
            Self::Lowres(lod) => format!("bluemap:lowres/{lod}"),
            Self::TileState => "bluemap:tile-state".into(),
            Self::ChunkState => "bluemap:chunk-state".into(),
            Self::RegionState => "bluemap:region-state".into(),
        }
    }

    pub fn from_sql_key(key: &str) -> Option<Self> {
        Some(match key {
            "bluemap:hires" => Self::Hires,
            "bluemap:tile-state" => Self::TileState,
            "bluemap:chunk-state" => Self::ChunkState,
            "bluemap:region-state" => Self::RegionState,
            _ => Self::Lowres(key.strip_prefix("bluemap:lowres/")?.parse().ok()?),
        })
    }
}

pub(crate) const ASSET_KEY_PREFIX: &str = "bluemap:asset/";

impl ItemKey {
    pub fn asset(name: impl Into<String>) -> Self {
        Self::Asset(name.into())
    }

    pub fn compression(&self, configured: Compression) -> Compression {
        match self {
            Self::Textures => configured,
            _ => Compression::None,
        }
    }

    /// `Key.formatted` used as `item_storage.key` in SQL.
    pub fn sql_key(&self) -> String {
        match self {
            Self::Settings => "bluemap:settings".into(),
            Self::Textures => "bluemap:textures".into(),
            Self::Markers => "bluemap:markers".into(),
            Self::Players => "bluemap:players".into(),
            Self::Asset(name) => format!("{ASSET_KEY_PREFIX}{}", escape_asset_name(name)),
        }
    }

    /// The fixed items every map may have, i.e. everything but assets.
    pub const FIXED: [ItemKey; 4] = [Self::Settings, Self::Textures, Self::Markers, Self::Players];
}

/// `MapStorage.escapeAssetName`: `[^\w\d.\-_/]` → `_`, then `..` → `_.` (ASCII `\w`, Java default flags).
pub fn escape_asset_name(name: &str) -> String {
    let escaped: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/') { c } else { '_' })
        .collect();
    escaped.replace("..", "_.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_keys_round_trip() {
        for g in [
            GridKey::Hires,
            GridKey::Lowres(1),
            GridKey::Lowres(12),
            GridKey::TileState,
            GridKey::ChunkState,
            GridKey::RegionState,
        ] {
            assert_eq!(GridKey::from_sql_key(&g.sql_key()), Some(g));
        }
        assert_eq!(GridKey::Lowres(3).sql_key(), "bluemap:lowres/3");
        assert_eq!(GridKey::from_sql_key("bluemap:lowres/x"), None);
        assert_eq!(ItemKey::asset("playerheads/a b.png").sql_key(), "bluemap:asset/playerheads/a_b.png");
    }

    #[test]
    fn compressions_follow_bluemap() {
        let z = Compression::Zstd;
        assert_eq!(GridKey::Hires.compression(z), z);
        assert_eq!(GridKey::Lowres(1).compression(z), Compression::None);
        assert_eq!(GridKey::ChunkState.compression(Compression::None), Compression::Gzip);
        assert_eq!(ItemKey::Textures.compression(z), z);
        assert_eq!(ItemKey::Settings.compression(z), Compression::None);
    }

    #[test]
    fn asset_names_escape_like_java() {
        assert_eq!(escape_asset_name("../../etc/passwd"), "_./_./etc/passwd");
        assert_eq!(escape_asset_name("..."), "_..");
        assert_eq!(escape_asset_name("ä😀x?.png"), "__x_.png");
        assert_eq!(escape_asset_name("playerheads/0f-A_9.png"), "playerheads/0f-A_9.png");
    }
}
