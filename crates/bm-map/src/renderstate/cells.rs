//! One render-state file each: `TileInfoRegion`, `ChunkInfoRegion`, `RegionInfoRegion`. Reading follows BlueNBT:
//! unknown fields are skipped, a missing or wrong-length array resets to defaults, a wrong tag type is an error.

use std::marker::PhantomData;

use bm_nbt::{Tag, Writer};

use super::tile_state::TileState;
use super::{CellKind, Error, Result, compress, decompress, paletted};

/// A render-state file's contents; `modified` is transient and marks cells that need saving.
pub trait Cell: Sized {
    const KIND: CellKind;

    /// A cell with every entry at its default (0 / [`TileState::Unknown`]).
    fn new() -> Self;
    fn from_nbt(nbt: &[u8]) -> Result<Self>;
    fn to_nbt(&self) -> Vec<u8>;
    fn is_modified(&self) -> bool;
    fn set_modified(&mut self, modified: bool);

    /// Parses a stored (gzip'd) cell.
    fn decode(stored: &[u8]) -> Result<Self> {
        Self::from_nbt(&decompress(stored)?)
    }

    /// The bytes to store (gzip'd NBT).
    fn encode(&self) -> Result<Vec<u8>> {
        compress(&self.to_nbt())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileInfo {
    /// Unix seconds.
    pub render_time: i32,
    pub state: TileState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileInfoRegion {
    render_times: Vec<i32>,
    states: Vec<TileState>,
    modified: bool,
}

const RENDER_TIMES: &str = "last-render-times";
const TILE_STATES: &str = "tile-states";

impl TileInfoRegion {
    pub fn get(&self, x: i32, z: i32) -> TileInfo {
        let i = Self::KIND.index(x, z);
        TileInfo { render_time: self.render_times[i], state: self.states[i] }
    }

    /// Returns the previous info; marks the cell modified if it changed.
    pub fn set(&mut self, x: i32, z: i32, info: TileInfo) -> TileInfo {
        let previous = self.get(x, z);
        let i = Self::KIND.index(x, z);
        self.render_times[i] = info.render_time;
        self.states[i] = info.state;
        self.modified |= previous != info;
        previous
    }
}

impl Cell for TileInfoRegion {
    const KIND: CellKind = CellKind::Tiles;

    fn new() -> Self {
        let n = Self::KIND.entries();
        Self { render_times: vec![0; n], states: vec![TileState::Unknown; n], modified: false }
    }

    fn from_nbt(nbt: &[u8]) -> Result<Self> {
        let mut cell = Self::new();
        for (name, tag) in bm_nbt::read_root(nbt)?.entries() {
            match name {
                n if n == RENDER_TIMES.as_bytes() => {
                    cell.render_times = sized(read_ints(tag, RENDER_TIMES)?, Self::KIND, 0);
                }
                n if n == TILE_STATES.as_bytes() => {
                    let states = paletted::read(tag, TileState::from_key_lenient)?;
                    cell.states = sized(states, Self::KIND, TileState::Unknown);
                }
                _ => {}
            }
        }
        Ok(cell)
    }

    fn to_nbt(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.int_array(RENDER_TIMES, &self.render_times);
        paletted::write(&mut w, TILE_STATES, &self.states, TileState::key);
        w.finish()
    }

    fn is_modified(&self) -> bool {
        self.modified
    }

    fn set_modified(&mut self, modified: bool) {
        self.modified = modified;
    }
}

/// Selects the grid and NBT field name of an [`IntCell`].
pub trait IntField {
    const KIND: CellKind;
    const NAME: &'static str;
}

/// Region-header chunk timestamps the tiles were last rendered from (`ChunkInfoRegion`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkHashes;

/// Unix seconds of each region's last completed update; 0 = never / region gone (`RegionInfoRegion`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionUpdateTimes;

impl IntField for ChunkHashes {
    const KIND: CellKind = CellKind::Chunks;
    const NAME: &'static str = "chunk-hashes";
}

impl IntField for RegionUpdateTimes {
    const KIND: CellKind = CellKind::Regions;
    const NAME: &'static str = "last-update-times";
}

pub type ChunkInfoRegion = IntCell<ChunkHashes>;
pub type RegionInfoRegion = IntCell<RegionUpdateTimes>;

/// A cell holding one `int` per entry under a single int-array field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntCell<F> {
    values: Vec<i32>,
    modified: bool,
    field: PhantomData<F>,
}

impl<F: IntField> IntCell<F> {
    pub fn get(&self, x: i32, z: i32) -> i32 {
        self.values[F::KIND.index(x, z)]
    }

    /// Returns the previous value; marks the cell modified if it changed.
    pub fn set(&mut self, x: i32, z: i32, value: i32) -> i32 {
        let previous = std::mem::replace(&mut self.values[F::KIND.index(x, z)], value);
        self.modified |= previous != value;
        previous
    }
}

impl<F: IntField> Cell for IntCell<F> {
    const KIND: CellKind = F::KIND;

    fn new() -> Self {
        Self { values: vec![0; F::KIND.entries()], modified: false, field: PhantomData }
    }

    fn from_nbt(nbt: &[u8]) -> Result<Self> {
        let mut cell = Self::new();
        for (name, tag) in bm_nbt::read_root(nbt)?.entries() {
            if name == F::NAME.as_bytes() {
                cell.values = sized(read_ints(tag, F::NAME)?, F::KIND, 0);
            }
        }
        Ok(cell)
    }

    fn to_nbt(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.int_array(F::NAME, &self.values);
        w.finish()
    }

    fn is_modified(&self) -> bool {
        self.modified
    }

    fn set_modified(&mut self, modified: bool) {
        self.modified = modified;
    }
}

/// `@NBTPostDeserialize init()`: an array of the wrong length is replaced by defaults.
fn sized<T: Clone>(values: Vec<T>, kind: CellKind, default: T) -> Vec<T> {
    if values.len() == kind.entries() { values } else { vec![default; kind.entries()] }
}

/// BlueNBT's `int[]` adapter: an int array, a byte array (sign-extended) or a list of numbers.
fn read_ints(tag: Tag<'_>, field: &'static str) -> Result<Vec<i32>> {
    match tag {
        Tag::IntArray(a) => Ok(a.iter().collect()),
        Tag::ByteArray(b) => Ok(b.iter().map(|&v| i32::from(v as i8)).collect()),
        Tag::List(l) => l.iter().map(|t| t.as_i64().map(|v| v as i32).ok_or(Error::WrongType(field))).collect(),
        _ => Err(Error::WrongType(field)),
    }
}
