//! The few `maps/<id>.conf` keys the hires mesher needs, read by hand (no HOCON parser yet), and box-only render
//! masks with `CombinedMask` semantics.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use bm_render::{RenderMask, RenderSettings};

/// `key: value` at any depth outside the mask block; the first occurrence wins.
fn value<'a>(src: &'a str, key: &str) -> Option<&'a str> {
    src.lines().map(str::trim).filter(|l| !l.starts_with('#')).find_map(|l| {
        let rest = l.strip_prefix(key)?.trim_start();
        Some(rest.strip_prefix(':').or_else(|| rest.strip_prefix('='))?.trim().trim_matches('"'))
    })
}

fn parse<T: std::str::FromStr>(src: &str, key: &str, default: T) -> Result<T> {
    match value(src, key) {
        Some(v) => v.parse().ok().with_context(|| format!("{key}: can't parse '{v}'")),
        None => Ok(default),
    }
}

pub fn render_settings(src: &str) -> Result<RenderSettings> {
    let d = RenderSettings::default();
    let hires = parse(src, "enable-hires", true)?;
    let perspective = parse(src, "enable-perspective-view", true)?;
    let free_flight = parse(src, "enable-free-flight-view", true)?;
    let mask = BoxMask::parse(src)?;
    Ok(RenderSettings {
        remove_caves_below_y: parse(src, "remove-caves-below-y", d.remove_caves_below_y)?,
        cave_detection_ocean_floor: parse(src, "cave-detection-ocean-floor", d.cave_detection_ocean_floor)?,
        cave_detection_uses_block_light: parse(src, "cave-detection-uses-block-light", false)?,
        ambient_light: parse(src, "ambient-light", d.ambient_light)?,
        render_edges: parse(src, "render-edges", d.render_edges)?,
        edge_light_strength: parse(src, "edge-light-strength", d.edge_light_strength)?,
        ignore_missing_light_data: parse(src, "ignore-missing-light-data", false)?,
        render_top_only: !hires || (!perspective && !free_flight),
        mask: (!mask.layers.is_empty()).then(|| Arc::new(mask) as Arc<dyn RenderMask>),
    })
}

struct Layer {
    min: [i32; 3],
    max: [i32; 3],
    value: bool,
}

struct BoxMask {
    layers: Vec<Layer>,
}

impl BoxMask {
    /// `render-mask: [ {..} {..} ]` of box entries.
    fn parse(src: &str) -> Result<Self> {
        let Some(start) = src.find("render-mask") else { return Ok(Self { layers: Vec::new() }) };
        let block = &src[start..];
        let open = block.find('[').context("render-mask: expected a list")?;
        let close = block.find(']').context("render-mask: unterminated list")?;
        let mut layers = Vec::new();
        for entry in block[open + 1..close].split('}').filter(|e| e.contains('{')) {
            let entry = &entry[entry.find('{').unwrap_or(0) + 1..];
            if value(entry, "type").is_some_and(|t| t != "box") {
                bail!("render-mask: only box masks are supported here");
            }
            let get = |k, d| parse(entry, k, d);
            let (min, max) = (i32::MIN, i32::MAX);
            let layer = Layer {
                min: [get("min-x", min)?, get("min-y", min)?, get("min-z", min)?],
                max: [get("max-x", max)?, get("max-y", max)?, get("max-z", max)?],
                value: !parse(entry, "subtract", false)?,
            };
            // `CombinedMask.add`: a leading subtraction starts from everything
            if !layer.value && layers.is_empty() {
                layers.push(Layer { min: [min; 3], max: [max; 3], value: true });
            }
            layers.push(layer);
        }
        Ok(Self { layers })
    }
}

impl Layer {
    fn contains(&self, p: [i32; 3]) -> bool {
        (0..3).all(|i| self.min[i] <= p[i] && p[i] <= self.max[i])
    }
}

impl RenderMask for BoxMask {
    fn test(&self, x: i32, y: i32, z: i32) -> bool {
        match self.layers.iter().rev().find(|l| l.contains([x, y, z])) {
            Some(l) => l.value,
            None => self.layers.is_empty(),
        }
    }

    fn test_area(&self, min: [i32; 2], max: [i32; 2]) -> Option<bool> {
        for l in self.layers.iter().rev() {
            let overlaps = l.min[0] <= max[0] && min[0] <= l.max[0] && l.min[2] <= max[1] && min[1] <= l.max[2];
            if !overlaps {
                continue;
            }
            let covers = l.min[0] <= min[0] && max[0] <= l.max[0] && l.min[2] <= min[1] && max[1] <= l.max[2];
            return (covers && l.min[1] == i32::MIN && l.max[1] == i32::MAX).then_some(l.value);
        }
        Some(self.layers.is_empty())
    }

    fn test_column(&self, x: i32, z: i32) -> bool {
        for l in self.layers.iter().rev() {
            if !(l.min[0] <= x && x <= l.max[0] && l.min[2] <= z && z <= l.max[2]) {
                continue;
            }
            // a layer covering only part of the column makes the result undefined, which counts as inside
            return l.value || l.min[1] != i32::MIN || l.max[1] != i32::MAX;
        }
        self.layers.is_empty()
    }
}
