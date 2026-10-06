//! flow-math 1.0.3 `TrigMath`: sin/cos read a 2^22-entry float table with no interpolation; the arc functions are
//! Cephes-style rational approximations, not `std`'s.
#![allow(clippy::excessive_precision)] // literals copied verbatim from TrigMath.java

use std::f64::consts::PI;

pub const HALF_PI: f64 = PI / 2.0;
pub const TWO_PI: f64 = 2.0 * PI;
pub const DEG_TO_RAD: f64 = PI / 180.0;
pub const RAD_TO_DEG: f64 = 180.0 / PI;

const SIN_BITS: u32 = 22;
const SIN_SIZE: i32 = 1 << SIN_BITS;
const SIN_MASK: i32 = SIN_SIZE - 1;
const SIN_CONVERSION_FACTOR: f64 = SIN_SIZE as f64 / TWO_PI;
const COS_OFFSET: i32 = SIN_SIZE / 4;

const SQ2P1: f64 = 2.414213562373095048802;
const SQ2M1: f64 = 0.414213562373095048802;
const P4: f64 = 0.161536412982230228262E2;
const P3: f64 = 0.26842548195503973794141E3;
const P2: f64 = 0.11530293515404850115428136E4;
const P1: f64 = 0.178040631643319697105464587E4;
const P0: f64 = 0.89678597403663861959987488E3;
const Q4: f64 = 0.5895697050844462222791E2;
const Q3: f64 = 0.536265374031215315104235E3;
const Q2: f64 = 0.16667838148816337184521798E4;
const Q1: f64 = 0.207933497444540981287275926E4;
const Q0: f64 = 0.89678597403663861962481162E3;

/// `TrigMath.sin`: table lookup at `floor(angle * 2^22 / 2π)`.
pub fn sin(angle: f64) -> f32 {
    sin_raw(floor(angle * SIN_CONVERSION_FACTOR))
}

/// `TrigMath.cos`: the sine table shifted by a quarter turn.
pub fn cos(angle: f64) -> f32 {
    cos_raw(floor(angle * SIN_CONVERSION_FACTOR))
}

pub fn asin(value: f64) -> f64 {
    if value > 1.0 {
        f64::NAN
    } else if value < 0.0 {
        -asin(-value)
    } else {
        let temp = (1.0 - value * value).sqrt();
        if value > 0.7 { HALF_PI - msatan(temp / value) } else { msatan(value / temp) }
    }
}

#[allow(clippy::manual_range_contains)] // NaN must fall through to asin, as in Java
pub fn acos(value: f64) -> f64 {
    if value > 1.0 || value < -1.0 { f64::NAN } else { HALF_PI - asin(value) }
}

pub fn atan(value: f64) -> f64 {
    if value > 0.0 { msatan(value) } else { -msatan(-value) }
}

pub fn atan2(y: f64, x: f64) -> f64 {
    if y + x == y {
        return if y >= 0.0 { HALF_PI } else { -HALF_PI };
    }
    let y = atan(y / x);
    if x < 0.0 {
        return if y <= 0.0 { y + PI } else { y - PI };
    }
    y
}

/// `GenericMath.floor(double)`: truncating cast, then step down for negatives.
fn floor(a: f64) -> i32 {
    let y = a as i32;
    if a < y as f64 { y.wrapping_sub(1) } else { y }
}

// Computes the table entry instead of storing it: identical to `SIN_TABLE[i]` without the 16 MiB table.
fn table(idx: i32) -> f32 {
    ((((idx & SIN_MASK) as f64) * TWO_PI) / SIN_SIZE as f64).sin() as f32
}

fn sin_raw(idx: i32) -> f32 {
    table(idx)
}

fn cos_raw(idx: i32) -> f32 {
    table(idx.wrapping_add(COS_OFFSET))
}

fn mxatan(arg: f64) -> f64 {
    let argsq = arg * arg;
    let mut value = (((P4 * argsq + P3) * argsq + P2) * argsq + P1) * argsq + P0;
    value /= ((((argsq + Q4) * argsq + Q3) * argsq + Q2) * argsq + Q1) * argsq + Q0;
    value * arg
}

fn msatan(arg: f64) -> f64 {
    if arg < SQ2M1 {
        return mxatan(arg);
    }
    if arg > SQ2P1 {
        return HALF_PI - mxatan(1.0 / arg);
    }
    HALF_PI / 2.0 + mxatan((arg - 1.0) / (arg + 1.0))
}

/// FNV-1a over the raw bits of every table entry, for checking the whole table against Java's.
#[doc(hidden)]
pub fn table_fingerprint() -> u64 {
    (0..SIN_SIZE).fold(0xcbf2_9ce4_8422_2325, |h, i| (h ^ table(i).to_bits() as u64).wrapping_mul(0x100_0000_01b3))
}
