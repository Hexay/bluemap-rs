//! Hires and lowres tiles of one map in both webroots.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;
use bm_format::grid::Tile as TilePos;
use bm_format::lowres::LowresTile;

use super::Section;
use crate::diff::RenderDiff;
use crate::{Tile, WebrootMap};

pub fn hires(golden: &Path, candidate: &Path, id: &str, section: &mut Section) -> Result<()> {
    let (g, c) = (WebrootMap::open(golden, Some(id))?, WebrootMap::open(candidate, Some(id))?);
    let (gt, ct) = (g.tiles(0)?, c.tiles(0)?);
    let names = |m: &WebrootMap| -> Result<Vec<String>> {
        Ok(crate::parse_textures(&m.textures_json()?)?.into_iter().map(|t| t.resource_path).collect())
    };
    let (gn, cn) = (names(&g)?, names(&c)?);
    let (mut identical, mut differing) = (0, Vec::new());
    let mut diff = RenderDiff::default();
    for &t in gt.union(&ct) {
        let gb = gt.contains(&t).then(|| g.tile_bytes(0, t)).transpose()?;
        let cb = ct.contains(&t).then(|| c.tile_bytes(0, t)).transpose()?;
        if gb == cb {
            identical += 1;
            continue;
        }
        differing.push(t);
        let parse = |b: Option<Vec<u8>>| b.map_or_else(|| Ok(Tile::default()), |b| crate::parse(&b));
        diff.add_tile(&parse(gb)?, &gn, &parse(cb)?, &cn, g.hires_origin(t), |_, _| false);
    }
    let total = gt.union(&ct).count();
    section.check(differing.is_empty(), format!("{identical}/{total} tiles byte-identical (decompressed PRBM)"));
    if !differing.is_empty() {
        let report = diff.finish(5);
        let pct = 100.0 * report.identical as f64 / report.original_faces.max(1) as f64;
        section.detail(format!("differing tiles: faces identical {pct:.3}% of {}", report.original_faces));
        differing.iter().take(10).for_each(|t| section.detail(format!("tile {t:?} differs")));
    }
    Ok(())
}

pub fn lowres(golden: &Path, candidate: &Path, id: &str, section: &mut Section) -> Result<()> {
    let (g, c) = (WebrootMap::open(golden, Some(id))?, WebrootMap::open(candidate, Some(id))?);
    let size = g.settings.lowres.tile_size.map(|s| s as usize);
    for lod in 1..=g.settings.lowres.lod_count {
        let (gt, ct) = (g.tiles(lod)?, c.tiles(lod)?);
        let all: BTreeSet<TilePos> = gt.union(&ct).copied().collect();
        let mut bad = Vec::new();
        for &t in &all {
            let load = |m: &WebrootMap, present: &BTreeSet<TilePos>| -> Result<Option<LowresTile>> {
                if !present.contains(&t) {
                    return Ok(None);
                }
                Ok(Some(LowresTile::decode_png(&m.tile_bytes(lod, t)?, size)?))
            };
            let (a, b) = (load(&g, &gt)?, load(&c, &ct)?);
            if a != b {
                let pixels = match (&a, &b) {
                    (Some(a), Some(b)) => count_diff(a, b, size).to_string(),
                    _ => "file missing on one side".to_owned(),
                };
                bad.push(format!("lod {lod} {t:?}: {pixels} pixels differ"));
            }
        }
        section.check(bad.is_empty(), format!("lod {lod}: {}/{} tiles pixel-identical", all.len() - bad.len(), all.len()));
        bad.iter().take(10).for_each(|b| section.detail(b.clone()));
    }
    Ok(())
}

fn count_diff(a: &LowresTile, b: &LowresTile, size: [usize; 2]) -> usize {
    let mut n = 0;
    for x in 0..=size[0] {
        for z in 0..=size[1] {
            n += usize::from(a.color(x, z) != b.color(x, z) || a.meta(x, z) != b.meta(x, z));
        }
    }
    n
}
