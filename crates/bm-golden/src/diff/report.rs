use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;

use super::ASPECTS;

#[derive(Debug, Serialize)]
pub struct AspectReport {
    /// Paired faces where this aspect differs.
    pub faces: u64,
    /// Mean |Δ| over those faces (AO per vertex, 0..255; light 0..15).
    pub mean_delta: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct CellReport {
    pub pos: [i32; 3],
    pub missing: u32,
    pub extra: u32,
    pub differing: u32,
    pub aspects: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    /// Faces (both renders) skipped because they are in or look into a column outside the compared area.
    pub edge_faces: u64,
    pub original_faces: u64,
    pub candidate_faces: u64,
    /// Faces with identical geometry in both renders.
    pub paired: u64,
    /// Paired faces that also agree on every aspect.
    pub identical: u64,
    pub aspects: BTreeMap<&'static str, AspectReport>,
    /// Original faces without a partner, by texture (top N).
    pub missing: Vec<(String, u64)>,
    /// Reconstructed faces without a partner, by texture (top N).
    pub extra: Vec<(String, u64)>,
    /// Block cells with the most unpaired or differing faces (top N).
    pub cells: Vec<CellReport>,
}

fn pct(n: u64, of: u64) -> f64 {
    100.0 * n as f64 / of.max(1) as f64
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let (o, p) = (self.original_faces, self.paired);
        writeln!(
            f,
            "faces       original {o}, candidate {} ({} at the edge skipped)",
            self.candidate_faces, self.edge_faces
        )?;
        writeln!(f, "identical   {:.2}% of original faces ({})", pct(self.identical, o), self.identical)?;
        writeln!(f, "paired      {:.2}% of original faces ({p}), same geometry", pct(p, o))?;
        writeln!(f, "missing     {} faces, extra {}", o - p, self.candidate_faces - p)?;
        writeln!(f, "differing aspects of paired faces:")?;
        for name in ASPECTS {
            let a = &self.aspects[name];
            let delta = a.mean_delta.map(|d| format!("  mean |Δ| {d:.1}")).unwrap_or_default();
            writeln!(f, "  {name:<11} {:>8} ({:.2}%){delta}", a.faces, pct(a.faces, p))?;
        }
        for (label, list) in [("missing", &self.missing), ("extra", &self.extra)] {
            if !list.is_empty() {
                writeln!(f, "{label} by texture:")?;
            }
            for (texture, n) in list {
                writeln!(f, "  {n:>8}  {texture}")?;
            }
        }
        if !self.cells.is_empty() {
            writeln!(f, "worst cells (missing/extra/differing):")?;
        }
        for c in &self.cells {
            let [x, y, z] = c.pos;
            writeln!(f, "  {x},{y},{z}  {}/{}/{}  {}", c.missing, c.extra, c.differing, c.aspects.join(" "))?;
        }
        Ok(())
    }
}
