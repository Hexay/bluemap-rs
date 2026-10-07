//! `Double.toString` / `Float.toString` and `(Concurrent)HashMap` order against values printed by Java (`tools/javaref/JdkRef.java`).

#[rustfmt::skip]
#[path = "data/fmt.rs"]
mod data;

#[rustfmt::skip]
#[path = "data/hash_map.rs"]
mod orders;

#[rustfmt::skip]
#[path = "data/concurrent_hash_map.rs"]
mod chm_orders;

use bm_java::fmt::{double_to_string, float_to_string};

#[test]
fn doubles_match_java() {
    let bad: Vec<_> = data::DOUBLES
        .iter()
        .map(|&(bits, java)| (java, double_to_string(f64::from_bits(bits))))
        .filter(|(java, ours)| java != ours)
        .collect();
    assert!(bad.is_empty(), "{} of {} differ: {:?}", bad.len(), data::DOUBLES.len(), &bad[..bad.len().min(20)]);
}

#[test]
fn floats_match_java() {
    let bad: Vec<_> = data::FLOATS
        .iter()
        .map(|&(bits, java)| (java, float_to_string(f32::from_bits(bits))))
        .filter(|(java, ours)| java != ours)
        .collect();
    assert!(bad.is_empty(), "{} of {} differ: {:?}", bad.len(), data::FLOATS.len(), &bad[..bad.len().min(20)]);
}

#[test]
fn hash_map_order_matches_java() {
    for &(inserted, iterated) in orders::ORDERS {
        assert_eq!(bm_java::hash_map_order(inserted), iterated);
    }
}

#[test]
fn concurrent_hash_map_order_matches_java() {
    for &(inserted, put_all, iterated) in chm_orders::ORDERS {
        assert_eq!(bm_java::concurrent_hash_map_order(inserted), iterated, "putAll: {put_all}");
    }
}
