//! `memset` for the static musl cores. zig's compiler-rt provides libc's string functions there, and its `memset` is
//! a byte-at-a-time loop: 11% of a forced render's CPU against ~3% for glibc's (docs/23). Its symbols are weak, so
//! this strong definition replaces it at link time.
//!
//! The bulk loop uses volatile stores: LLVM turns a plain store loop back into a `memset` call, which here would recurse.

#[cfg(target_env = "musl")]
use std::ffi::{c_int, c_void};
use std::ptr::{write_unaligned, write_volatile};

#[cfg(target_arch = "x86_64")]
type Block = std::arch::x86_64::__m128i;
#[cfg(not(target_arch = "x86_64"))]
type Block = u128;

#[cfg(target_env = "musl")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(dst: *mut c_void, c: c_int, n: usize) -> *mut c_void {
    // SAFETY: the C contract: `dst` is valid for `n` bytes
    unsafe { fill(dst.cast(), c as u8, n) };
    dst
}

/// Writes `n` copies of `byte` from `dst`: straight-line overlapping stores up to 32 bytes, 16-byte blocks beyond.
///
/// # Safety
/// `dst` must be valid for writes of `n` bytes.
#[inline(always)]
pub unsafe fn fill(dst: *mut u8, byte: u8, n: usize) {
    let b8 = u64::from_ne_bytes([byte; 8]);
    // SAFETY: `Block` is 16 plain bytes on every target
    let block: Block = unsafe { std::mem::transmute([byte; 16]) };
    // SAFETY (all blocks): every store lies within `dst..dst + n`
    unsafe {
        match n {
            0 => {}
            1..=3 => {
                *dst = byte;
                *dst.add(n / 2) = byte;
                *dst.add(n - 1) = byte;
            }
            4..=7 => {
                write_unaligned(dst.cast::<u32>(), b8 as u32);
                write_unaligned(dst.add(n - 4).cast::<u32>(), b8 as u32);
            }
            8..=16 => {
                write_unaligned(dst.cast::<u64>(), b8);
                write_unaligned(dst.add(n - 8).cast::<u64>(), b8);
            }
            17..=32 => {
                write_unaligned(dst.cast::<Block>(), block);
                write_unaligned(dst.add(n - 16).cast::<Block>(), block);
            }
            _ => {
                write_unaligned(dst.cast::<Block>(), block);
                let end = dst.add(n);
                let mut p = dst.add(16 - (dst as usize & 15)).cast::<Block>();
                while (p as usize) + 64 <= end as usize {
                    write_volatile(p, block);
                    write_volatile(p.add(1), block);
                    write_volatile(p.add(2), block);
                    write_volatile(p.add(3), block);
                    p = p.add(4);
                }
                while (p as usize) + 16 <= end as usize {
                    write_volatile(p, block);
                    p = p.add(1);
                }
                write_unaligned(end.sub(16).cast::<Block>(), block);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fill;

    #[test]
    fn fills_exactly_the_range_at_every_length_and_alignment() {
        let mut buf = vec![0xAAu8; 600];
        for offset in 0..17 {
            for n in 0..520 {
                buf.fill(0xAA);
                // SAFETY: offset + n < buf.len()
                unsafe { fill(buf.as_mut_ptr().add(offset), 0x5C, n) };
                assert!(buf[..offset].iter().all(|&b| b == 0xAA), "head clobbered at {offset}+{n}");
                assert!(buf[offset..offset + n].iter().all(|&b| b == 0x5C), "range not filled at {offset}+{n}");
                assert!(buf[offset + n..].iter().all(|&b| b == 0xAA), "tail clobbered at {offset}+{n}");
            }
        }
    }
}
