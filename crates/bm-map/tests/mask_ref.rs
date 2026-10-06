//! Masks built from config structs against BlueMap 5.28's own mask classes (`tools/javaref/MaskRef.java`).

#[rustfmt::skip]
#[path = "data/mask.rs"]
mod data;

use bm_map::mask::{Mask, MaskConfig, MaskShape, Tristate, build_render_mask};

/// Parses MaskRef's spec grammar into config entries.
fn parse(tokens: &mut std::iter::Peekable<std::str::SplitWhitespace<'_>>) -> Vec<MaskConfig> {
    let mut layers = Vec::new();
    while let Some(t) = tokens.next() {
        if t == ")" {
            break;
        }
        let subtract = t.starts_with('-');
        let mut i = || tokens.next().unwrap().parse::<i32>().unwrap();
        let shape = match &t[1..] {
            "box" => MaskShape::Box { min: [i(), i(), i()], max: [i(), i(), i()] },
            "circle" | "ellipse" | "poly" => {
                let mut nums = Vec::new();
                while let Some(n) = tokens.peek().filter(|n| n.parse::<f64>().is_ok()) {
                    nums.push(n.parse::<f64>().unwrap());
                    tokens.next();
                }
                match &t[1..] {
                    "circle" => MaskShape::Circle {
                        center_x: nums[0],
                        center_z: nums[1],
                        radius: nums[2],
                        min_y: nums[3] as i32,
                        max_y: nums[4] as i32,
                    },
                    "ellipse" => MaskShape::Ellipse {
                        center_x: nums[0],
                        center_z: nums[1],
                        radius_x: nums[2],
                        radius_z: nums[3],
                        min_y: nums[4] as i32,
                        max_y: nums[5] as i32,
                    },
                    _ => MaskShape::Polygon {
                        min_y: nums[0] as i32,
                        max_y: nums[1] as i32,
                        shape: nums[2..].chunks(2).map(|p| [p[0], p[1]]).collect(),
                    },
                }
            }
            "blur" => {
                let size = i();
                assert_eq!(tokens.next(), Some("("));
                MaskShape::Blur { size, masks: parse(tokens) }
            }
            other => panic!("unknown mask {other}"),
        };
        layers.push(MaskConfig { subtract, shape });
    }
    layers
}

fn tristate(t: Tristate) -> i8 {
    match t {
        Tristate::True => 1,
        Tristate::Undefined => 0,
        Tristate::False => -1,
    }
}

#[test]
fn masks_match_java() {
    let mut checked = 0;
    for s in data::SCENARIOS {
        let mask = Mask::Combined(build_render_mask(&parse(&mut s.spec.split_whitespace().peekable())).unwrap());
        for &(x, y, z, want) in s.points {
            assert_eq!(mask.test(x, y, z), want, "{:?} test({x}, {y}, {z})", s.spec);
        }
        for &([a, b, c, d, e, f], want) in s.boxes {
            assert_eq!(
                tristate(mask.test_area(a, b, c, d, e, f)),
                want,
                "{:?} test_area{:?}",
                s.spec,
                [a, b, c, d, e, f]
            );
        }
        for &([a, b, c, d], want) in s.edges {
            assert_eq!(mask.is_edge(a, b, c, d), want, "{:?} is_edge{:?}", s.spec, [a, b, c, d]);
        }
        for &([a, b, c, d, e, f], kind, layers, points) in s.subs {
            let sub = mask.submask(a, b, c, d, e, f);
            let (got_kind, got_layers) = match &sub {
                Mask::All => ('A', -1),
                Mask::None => ('N', -1),
                Mask::Combined(m) => ('C', m.len() as i32),
                _ => ('O', -1),
            };
            assert_eq!((got_kind, got_layers), (kind, layers), "{:?} submask{:?}", s.spec, [a, b, c, d, e, f]);
            for &(x, y, z, want) in points {
                assert_eq!(sub.test(x, y, z), want, "{:?} submask test({x}, {y}, {z})", s.spec);
            }
        }
        checked += s.points.len() + s.boxes.len() + s.edges.len() + s.subs.len();
    }
    assert!(checked > 5000, "only {checked} reference queries");
}
