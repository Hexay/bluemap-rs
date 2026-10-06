use super::*;
use Action::{Delete as D, None as N, Render as R};
use BoundsSituation::{Edge, Inside, Outside};
use TileState::*;

mod store_tests;

/// `TileState.java` transcribed: (state, changed) → [INSIDE, EDGE, OUTSIDE] outcomes.
type Row = (TileState, bool, [(Action, TileState); 3]);

const TABLE: &[Row] = &[
    (Unknown, false, [(R, Rendered), (R, RenderedEdge), (D, OutOfBounds)]),
    (Unknown, true, [(R, Rendered), (R, RenderedEdge), (D, OutOfBounds)]),
    (Rendered, false, [(N, Rendered), (R, RenderedEdge), (D, OutOfBounds)]),
    (Rendered, true, [(R, Rendered), (R, RenderedEdge), (D, OutOfBounds)]),
    (RenderedEdge, false, [(R, Rendered), (N, RenderedEdge), (D, OutOfBounds)]),
    (RenderedEdge, true, [(R, Rendered), (R, RenderedEdge), (D, OutOfBounds)]),
    (OutOfBounds, false, [(R, Rendered), (R, RenderedEdge), (N, OutOfBounds)]),
    (OutOfBounds, true, [(R, Rendered), (R, RenderedEdge), (N, OutOfBounds)]),
    (NotGenerated, false, [(N, NotGenerated), (N, NotGenerated), (N, NotGenerated)]),
    (NotGenerated, true, [(R, Rendered), (R, RenderedEdge), (D, OutOfBounds)]),
    (MissingLight, false, [(N, MissingLight), (N, MissingLight), (N, MissingLight)]),
    (MissingLight, true, [(R, Rendered), (R, RenderedEdge), (D, OutOfBounds)]),
    (LowInhabitedTime, false, [(N, LowInhabitedTime), (N, LowInhabitedTime), (N, LowInhabitedTime)]),
    (LowInhabitedTime, true, [(R, Rendered), (R, RenderedEdge), (D, OutOfBounds)]),
    (ChunkError, false, [(N, ChunkError), (N, ChunkError), (N, ChunkError)]),
    (ChunkError, true, [(R, Rendered), (R, RenderedEdge), (D, OutOfBounds)]),
    (RenderError, false, [(R, Rendered), (R, RenderedEdge), (D, OutOfBounds)]),
    (RenderError, true, [(R, Rendered), (R, RenderedEdge), (D, OutOfBounds)]),
];

#[test]
fn transition_table_matches_java() {
    assert_eq!(TABLE.len(), TileState::ALL.len() * 2);
    for &(state, changed, outcomes) in TABLE {
        for (bounds, (action, next)) in [Inside, Edge, Outside].into_iter().zip(outcomes) {
            let got = state.find_action_and_next_state(changed, bounds);
            assert_eq!(got, ActionAndNextState::new(action, next), "{state:?} changed={changed} {bounds:?}");
        }
    }
}

#[test]
fn keys_round_trip_and_parse_like_key_parse() {
    for s in TileState::ALL {
        assert_eq!(TileState::from_key(s.key()), Some(s));
    }
    assert_eq!(TileState::from_key("rendered-edge"), Some(RenderedEdge));
    assert_eq!(TileState::from_key("minecraft:rendered"), None);
    assert_eq!(TileState::from_key(":rendered"), None);
    assert_eq!(TileState::from_key_lenient("bluemap:some-future-state"), Unknown);
    assert_eq!(TileUpdateStrategy::from_key("force_edge"), Some(TileUpdateStrategy::ForceEdge));
}

#[test]
fn update_strategies() {
    use TileUpdateStrategy::*;
    assert!(TileState::ALL.iter().all(|&s| ForceAll.test(s) && !ForceNone.test(s)));
    assert_eq!(TileState::ALL.iter().filter(|&&s| ForceEdge.test(s)).count(), 1);
    assert!(ForceEdge.test(RenderedEdge));
    assert_eq!(TileUpdateStrategy::fixed(true), ForceAll);
}

#[test]
fn cell_index_and_paths() {
    assert_eq!(CellKind::Tiles.index(-1, 0), 31);
    assert_eq!(CellKind::Tiles.index(0, -1), 31 << 5);
    assert_eq!(CellKind::Chunks.index(130, 1), (1 << 7) | 2);
    assert_eq!(CellKind::Regions.cell_of(-1, 64), (-1, 1));
    assert_eq!(CellKind::Regions.relative_path((-1, 0)), "rstate/regions/x-1/z0.regions.dat");
    assert_eq!(CellKind::Tiles.relative_path((12, -3)), "rstate/x1/2/z-3.tiles.dat");
    assert_eq!(CellKind::Chunks.entries(), 16384);
}

/// The exact bytes BlueNBT writes for a tile cell.
#[test]
fn tile_cell_nbt_layout() {
    let mut cell = TileInfoRegion::new();
    cell.set(1, 0, TileInfo { render_time: 7, state: MissingLight });
    cell.set(2, 0, TileInfo { render_time: 7, state: Rendered });
    let nbt = cell.to_nbt();
    let mut expected = vec![10, 0, 0, 11, 0, 17];
    expected.extend(b"last-render-times");
    expected.extend(1024i32.to_be_bytes());
    expected.extend([0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 7]);
    expected.extend(std::iter::repeat_n(0, 4 * 1021));
    expected.extend([10, 0, 11]);
    expected.extend(b"tile-states");
    expected.extend([9, 0, 7]);
    expected.extend(b"palette");
    expected.extend([8, 0, 0, 0, 3]);
    for k in ["bluemap:unknown", "bluemap:missing-light", "bluemap:rendered"] {
        expected.extend((k.len() as u16).to_be_bytes());
        expected.extend(k.as_bytes());
    }
    expected.extend([7, 0, 4]);
    expected.extend(b"data");
    expected.extend(1024i32.to_be_bytes());
    expected.extend([0, 1, 2]);
    expected.extend(std::iter::repeat_n(0, 1021));
    expected.extend([0, 0]);
    assert_eq!(nbt, expected);
    assert_eq!(TileInfoRegion::from_nbt(&nbt).unwrap().get(2, 0).state, Rendered);
}

#[test]
fn int_cells_round_trip_through_gzip() {
    let mut cell = ChunkInfoRegion::new();
    assert_eq!(cell.set(-1, -1, 0x6ab8_248f), 0);
    assert!(cell.is_modified());
    let decoded = ChunkInfoRegion::decode(&cell.encode().unwrap()).unwrap();
    assert_eq!(decoded.get(127, 127), 0x6ab8_248f);
    assert!(!decoded.is_modified());
    assert_eq!(&ChunkInfoRegion::new().to_nbt()[3..18], b"\x0b\0\x0cchunk-hashes");
    assert_eq!(&RegionInfoRegion::new().to_nbt()[3..23], b"\x0b\0\x11last-update-times");
}

#[test]
fn set_same_value_is_not_a_modification() {
    let mut cell = RegionInfoRegion::new();
    cell.set(3, 4, 0);
    assert!(!cell.is_modified());
    let mut tiles = TileInfoRegion::new();
    tiles.set(0, 0, TileInfo { render_time: 0, state: Unknown });
    assert!(!tiles.is_modified());
}

fn tile_nbt(build: impl FnOnce(&mut bm_nbt::Writer)) -> Vec<u8> {
    let mut w = bm_nbt::Writer::new();
    build(&mut w);
    w.finish()
}

#[test]
fn lenient_reads_like_bluenbt() {
    // unknown fields and unknown state keys are fine; wrong-length arrays reset
    let nbt = tile_nbt(|w| {
        w.string("future-field", "x").int_array("last-render-times", &[1, 2, 3]);
        w.begin_compound("tile-states").string_list("palette", &["rendered", "bluemap:from-the-future"]);
        let mut data = vec![0u8; 1024];
        data[5] = 1;
        w.byte_array("data", &data).end_compound();
    });
    let cell = TileInfoRegion::from_nbt(&nbt).unwrap();
    assert_eq!(cell.get(0, 0), TileInfo { render_time: 0, state: Rendered });
    assert_eq!(cell.get(5, 0).state, Unknown);

    let short = tile_nbt(|w| {
        w.begin_compound("tile-states").string_list("palette", &["bluemap:rendered"]).byte_array("data", &[0; 3]);
        w.end_compound();
    });
    assert_eq!(TileInfoRegion::from_nbt(&short).unwrap().get(0, 0).state, Unknown);
    assert_eq!(TileInfoRegion::from_nbt(&tile_nbt(|_| {})).unwrap(), TileInfoRegion::new());

    let bytes = tile_nbt(|w| {
        w.byte_array("chunk-hashes", &[0xff; 16384]);
    });
    assert_eq!(ChunkInfoRegion::from_nbt(&bytes).unwrap().get(0, 0), -1);
}

#[test]
fn malformed_cells_are_errors() {
    let wrong_type = tile_nbt(|w| {
        w.string("last-render-times", "nope");
    });
    assert!(matches!(TileInfoRegion::from_nbt(&wrong_type), Err(Error::WrongType(_))));
    let empty_palette = tile_nbt(|w| {
        w.begin_compound("tile-states").string_list("palette", &[]).byte_array("data", &[0]).end_compound();
    });
    assert!(matches!(TileInfoRegion::from_nbt(&empty_palette), Err(Error::EmptyPalette)));
    for index in [1u8, 200] {
        let bad_index = tile_nbt(|w| {
            w.begin_compound("tile-states").string_list("palette", &["rendered"]).byte_array("data", &[index]);
            w.end_compound();
        });
        assert!(matches!(TileInfoRegion::from_nbt(&bad_index), Err(Error::PaletteIndex { .. })));
    }
    assert!(ChunkInfoRegion::decode(b"not gzip").is_err());
}
