//! `TileState`, `TileActionResolver` and `TileUpdateStrategy` (`core/map/renderstate/TileState.java`,
//! `TileActionResolver.java`, `common/rendermanager/TileUpdateStrategy.java`).

use std::borrow::Cow;

/// Persisted per hires tile. Keys are BlueMap registry keys (`bluemap:<name>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TileState {
    Unknown,
    Rendered,
    RenderedEdge,
    OutOfBounds,
    NotGenerated,
    MissingLight,
    LowInhabitedTime,
    ChunkError,
    RenderError,
}

/// How a tile relates to the map's render boundaries (`isInsideRenderBoundaries` with/without edge tolerance).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BoundsSituation {
    Inside,
    Edge,
    Outside,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    None,
    Render,
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ActionAndNextState {
    pub action: Action,
    pub state: TileState,
}

impl ActionAndNextState {
    pub const fn new(action: Action, state: TileState) -> Self {
        Self { action, state }
    }
}

const RENDER_RENDERED: ActionAndNextState = ActionAndNextState::new(Action::Render, TileState::Rendered);
const NONE_RENDERED: ActionAndNextState = ActionAndNextState::new(Action::None, TileState::Rendered);
const RENDER_RENDERED_EDGE: ActionAndNextState = ActionAndNextState::new(Action::Render, TileState::RenderedEdge);
const NONE_RENDERED_EDGE: ActionAndNextState = ActionAndNextState::new(Action::None, TileState::RenderedEdge);
const DELETE_OUT_OF_BOUNDS: ActionAndNextState = ActionAndNextState::new(Action::Delete, TileState::OutOfBounds);
const NONE_OUT_OF_BOUNDS: ActionAndNextState = ActionAndNextState::new(Action::None, TileState::OutOfBounds);

pub const BLUEMAP_NAMESPACE: &str = "bluemap";

impl TileState {
    /// Registry order (`TileState.REGISTRY`).
    pub const ALL: [Self; 9] = [
        Self::Unknown,
        Self::Rendered,
        Self::RenderedEdge,
        Self::OutOfBounds,
        Self::NotGenerated,
        Self::MissingLight,
        Self::LowInhabitedTime,
        Self::ChunkError,
        Self::RenderError,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Unknown => "bluemap:unknown",
            Self::Rendered => "bluemap:rendered",
            Self::RenderedEdge => "bluemap:rendered-edge",
            Self::OutOfBounds => "bluemap:out-of-bounds",
            Self::NotGenerated => "bluemap:not-generated",
            Self::MissingLight => "bluemap:missing-light",
            Self::LowInhabitedTime => "bluemap:low-inhabited-time",
            Self::ChunkError => "bluemap:chunk-error",
            Self::RenderError => "bluemap:render-error",
        }
    }

    /// `Key.parse(s, "bluemap")` + registry lookup; `None` for keys this version doesn't know.
    pub fn from_key(s: &str) -> Option<Self> {
        let formatted = bluemap_key(s);
        Self::ALL.into_iter().find(|t| t.key() == formatted)
    }

    /// `RegistryAdapter` read: keys from newer BlueMap versions fall back to [`TileState::Unknown`].
    pub fn from_key_lenient(s: &str) -> Self {
        Self::from_key(s).unwrap_or(Self::Unknown)
    }

    pub fn find_action_and_next_state(self, changed: bool, bounds: BoundsSituation) -> ActionAndNextState {
        use BoundsSituation::{Edge, Inside, Outside};
        match self {
            Self::Unknown => by_bounds(bounds),
            Self::Rendered => match bounds {
                Inside if changed => RENDER_RENDERED,
                Inside => NONE_RENDERED,
                Edge => RENDER_RENDERED_EDGE,
                Outside => DELETE_OUT_OF_BOUNDS,
            },
            Self::RenderedEdge => match bounds {
                Inside => RENDER_RENDERED,
                Edge if changed => RENDER_RENDERED_EDGE,
                Edge => NONE_RENDERED_EDGE,
                Outside => DELETE_OUT_OF_BOUNDS,
            },
            Self::OutOfBounds => match bounds {
                Inside => RENDER_RENDERED,
                Edge => RENDER_RENDERED_EDGE,
                Outside => NONE_OUT_OF_BOUNDS,
            },
            Self::RenderError => by_bounds(bounds),
            Self::NotGenerated | Self::MissingLight | Self::LowInhabitedTime | Self::ChunkError => {
                if changed {
                    by_bounds(bounds)
                } else {
                    ActionAndNextState::new(Action::None, self)
                }
            }
        }
    }
}

/// `Key.parse(s, "bluemap").getFormatted()`: a namespace needs a separator past index 0.
fn bluemap_key(s: &str) -> Cow<'_, str> {
    match s.find(':') {
        Some(i) if i > 0 => Cow::Borrowed(s),
        _ => Cow::Owned(format!("{BLUEMAP_NAMESPACE}:{s}")),
    }
}

fn by_bounds(bounds: BoundsSituation) -> ActionAndNextState {
    match bounds {
        BoundsSituation::Inside => RENDER_RENDERED,
        BoundsSituation::Edge => RENDER_RENDERED_EDGE,
        BoundsSituation::Outside => DELETE_OUT_OF_BOUNDS,
    }
}

/// Which tiles a region update re-renders regardless of chunk changes; serialized in `tasks.dat` by key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TileUpdateStrategy {
    ForceAll,
    ForceEdge,
    ForceNone,
}

impl TileUpdateStrategy {
    pub const ALL: [Self; 3] = [Self::ForceAll, Self::ForceEdge, Self::ForceNone];

    pub fn fixed(force: bool) -> Self {
        if force { Self::ForceAll } else { Self::ForceNone }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::ForceAll => "bluemap:force_all",
            Self::ForceEdge => "bluemap:force_edge",
            Self::ForceNone => "bluemap:force_none",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        let formatted = bluemap_key(s);
        Self::ALL.into_iter().find(|t| t.key() == formatted)
    }

    pub fn test(self, state: TileState) -> bool {
        match self {
            Self::ForceAll => true,
            Self::ForceEdge => state == TileState::RenderedEdge,
            Self::ForceNone => false,
        }
    }
}
