//! Why a [`super::RenderQueue`] is paused. Each source pauses and resumes on its own; the queue runs only when
//! none holds it.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    /// `/bluemap stop`, `RenderManager.stop()`, or render threads persisted as stopped.
    Stopped,
    /// `player-render-limit` reached.
    PlayerLimit,
    /// Core RSS above `memory-limit`.
    Memory,
    /// Server tick time above `render-pause-mspt`.
    ServerLoad,
}

impl PauseReason {
    pub const ALL: [PauseReason; 4] = [Self::Stopped, Self::PlayerLimit, Self::Memory, Self::ServerLoad];

    fn bit(self) -> u8 {
        1 << self as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PauseReasons(u8);

impl PauseReasons {
    pub fn contains(self, reason: PauseReason) -> bool {
        self.0 & reason.bit() != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn iter(self) -> impl Iterator<Item = PauseReason> {
        PauseReason::ALL.into_iter().filter(move |r| self.contains(*r))
    }

    pub(super) fn insert(&mut self, reason: PauseReason) {
        self.0 |= reason.bit();
    }

    pub(super) fn remove(&mut self, reason: PauseReason) {
        self.0 &= !reason.bit();
    }
}
