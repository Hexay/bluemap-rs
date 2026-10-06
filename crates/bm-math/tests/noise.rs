//! Simplex noise against Java BlueMap (see `data/noise.rs`).

#[rustfmt::skip]
#[path = "data/noise.rs"]
mod data;

use bm_java::JavaRandom;
use bm_math::SimplexNoise;

fn swamp() -> SimplexNoise {
    SimplexNoise::new(&mut JavaRandom::new(2345))
}

#[test]
fn permutation() {
    for (seed, p) in data::PERMUTATION {
        let noise = SimplexNoise::new(&mut JavaRandom::new(*seed));
        assert_eq!(noise.permutation(), p, "seed {seed}");
    }
}

#[test]
fn swamp_grass_noise() {
    let noise = swamp();
    for &(x, z, expected) in data::SWAMP {
        assert_eq!(noise.get_value(x as f64 * 0.0225, z as f64 * 0.0225).to_bits(), expected, "block {x} {z}");
    }
}

#[test]
fn raw_points() {
    let noise = swamp();
    for &(x, y, expected) in data::RAW {
        let (x, y) = (f64::from_bits(x), f64::from_bits(y));
        assert_eq!(noise.get_value(x, y).to_bits(), expected, "({x}, {y})");
    }
}
