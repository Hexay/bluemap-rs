//! Allocator glue. Static musl builds use mimalloc (glibc/Windows/macOS keep the system one): musl's malloc costs
//! ~25% more CPU on a forced render; mimalloc with its defaults holds ~60% more peak RSS than glibc, which matters next
//! to a JVM in one container. Purging freed memory after 3 ms without eager arena commit beats musl on both
//! (docs/13 §5). [`release_free_memory`] is for long-running modes going idle (docs/23).

#[cfg(target_env = "musl")]
mod tuned {
    use std::ffi::{c_int, c_long};

    #[global_allocator]
    static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

    unsafe extern "C" {
        fn mi_option_set(option: c_int, value: c_long);
    }

    // `mi_option_t` indices of the bundled mimalloc v3 (include/mimalloc.h); libmimalloc-sys doesn't bind these two
    const ARENA_EAGER_COMMIT: c_int = 4;
    const PURGE_DELAY: c_int = 15;

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
}

/// Applies the tuned mimalloc defaults on musl; `MIMALLOC_*` environment variables still override them.
pub fn tune() {
    #[cfg(target_env = "musl")]
    tuned::tune();
}

/// Hands memory the allocator keeps cached after a burst (a finished render) back to the OS. glibc only returns the
/// top of the main heap on `free`; the render threads' arenas keep their peak until `malloc_trim` walks them.
/// mimalloc needs nothing: its 3 ms purge already returns it (`mi_collect` at idle measured no gain, docs/23).
pub fn release_free_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> std::ffi::c_int;
        }
        // SAFETY: glibc entry point, no preconditions
        unsafe { malloc_trim(0) };
    }
}
