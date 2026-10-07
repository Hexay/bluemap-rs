//! The per-state facts the block pass and neighbour tests read for every block, kept small and dense.

/// One byte per state so neighbour tests stay in cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Flags(u8);

impl Flags {
    const AIR: u8 = 1;
    const CULLING: u8 = 2;
    const CULLING_IDENTICAL: u8 = 4;
    const OCCLUDING: u8 = 8;
    /// Water or rendered with water: what water's `isSameLiquid` accepts.
    const WATERY: u8 = 16;

    pub fn new(air: bool, culling: bool, culling_identical: bool, occluding: bool, watery: bool) -> Self {
        let bit = |set: bool, b: u8| if set { b } else { 0 };
        Self(
            bit(air, Self::AIR)
                | bit(culling, Self::CULLING)
                | bit(culling_identical, Self::CULLING_IDENTICAL)
                | bit(occluding, Self::OCCLUDING)
                | bit(watery, Self::WATERY),
        )
    }

    pub fn is_air(self) -> bool {
        self.0 & Self::AIR != 0
    }

    pub fn culling(self) -> bool {
        self.0 & Self::CULLING != 0
    }

    pub fn culling_identical(self) -> bool {
        self.0 & Self::CULLING_IDENTICAL != 0
    }

    pub fn occluding(self) -> bool {
        self.0 & Self::OCCLUDING != 0
    }

    pub fn watery(self) -> bool {
        self.0 & Self::WATERY != 0
    }
}

/// When a block renders nothing at all (no faces, no colour) because of its neighbours alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hidden {
    Never,
    /// Every face of every variant has a cullface within one block; these are their `Offset::slot`s, as bits. The
    /// block is hidden when all of them cull.
    Cullfaces(u32),
    /// Plain water (only liquid variants, not waterlogged): hidden when the neighbour above is water and every
    /// other one is water or culls.
    Water,
}
