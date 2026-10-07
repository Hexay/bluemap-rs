//! Global allocator of the static musl builds (glibc/Windows/macOS keep the system one). musl's malloc costs ~25%
//! more CPU on a forced render; mimalloc with its defaults holds ~60% more peak RSS than glibc, which matters next to
//! a JVM in one container. Purging freed memory after 3 ms without eager arena commit beats musl on both (docs/13 §5).

use std::ffi::{c_int, c_long};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

unsafe extern "C" {
    fn mi_option_set(option: c_int, value: c_long);
}

// `mi_option_t` indices of the bundled mimalloc v3 (include/mimalloc.h); libmimalloc-sys doesn't bind these two
const ARENA_EAGER_COMMIT: c_int = 4;
const PURGE_DELAY: c_int = 15;

/// Applies the tuned defaults; `MIMALLOC_*` environment variables still override them.
pub fn tune() {
    for (option, env, value) in
        [(PURGE_DELAY, "MIMALLOC_PURGE_DELAY", 3), (ARENA_EAGER_COMMIT, "MIMALLOC_ARENA_EAGER_COMMIT", 0)]
    {
        if std::env::var_os(env).is_none() {
            // SAFETY: plain option store in mimalloc; valid index for the linked version
            unsafe { mi_option_set(option, value) };
        }
    }
}
