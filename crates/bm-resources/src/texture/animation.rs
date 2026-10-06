//! `.png.mcmeta` animation data (`RP/texture/AnimationMeta.java`).

use serde_json::Value;

use super::Error;
use super::gson::{self, GResult, GsonError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimationMeta {
    pub interpolate: bool,
    pub width: i32,
    pub height: i32,
    pub frametime: i32,
    /// `None` when absent or empty.
    pub frames: Option<Vec<FrameMeta>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameMeta {
    pub index: i32,
    pub time: i32,
}

impl Default for AnimationMeta {
    fn default() -> Self {
        Self { interpolate: false, width: 1, height: 1, frametime: 1, frames: None }
    }
}

impl AnimationMeta {
    /// A `.png.mcmeta` file; `None` for an empty file (Gson's `fromJson` yields null).
    pub fn parse_mcmeta(bytes: &[u8]) -> Result<Option<Self>, Error> {
        let src = std::str::from_utf8(bytes).map_err(|_| Error::Utf8("animation meta"))?;
        if src.trim_start_matches('\u{feff}').trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(Self::from_mcmeta(&crate::json::parse(src)?)?))
    }

    /// The upstream adapter: an object of which only the `animation` member is read. It is also what reads
    /// `textures.json` back, where there is no `animation` wrapper, so reloaded animations become defaults.
    pub fn from_mcmeta(v: &Value) -> GResult<Self> {
        let mut meta = Self::default();
        if let Some(anim) = gson::object(v)?.get("animation") {
            meta.read_fields(anim)?;
        }
        meta.fill_frame_times();
        Ok(meta)
    }

    /// The inner `animation` object, as written to `textures.json`.
    pub fn from_fields(v: &Value) -> GResult<Self> {
        let mut meta = Self::default();
        meta.read_fields(v)?;
        meta.fill_frame_times();
        Ok(meta)
    }

    fn read_fields(&mut self, v: &Value) -> GResult<()> {
        for (name, value) in gson::object(v)? {
            match name.as_str() {
                "interpolate" => self.interpolate = gson::boolean(value)?,
                "width" => self.width = gson::int(value)?,
                "height" => self.height = gson::int(value)?,
                "frametime" => self.frametime = gson::double(value)? as i32,
                "frames" => self.frames = read_frames(value)?,
                _ => {}
            }
        }
        Ok(())
    }

    /// A frame without `time` (read as -1) uses `frametime`.
    fn fill_frame_times(&mut self) {
        for frame in self.frames.iter_mut().flatten() {
            if frame.time == -1 {
                frame.time = self.frametime;
            }
        }
    }

    /// Gson's reflective output: every field in declaration order, `frames` omitted when null.
    pub(crate) fn write_json(&self, out: &mut String) {
        out.push_str(&format!(
            "{{\"interpolate\":{},\"width\":{},\"height\":{},\"frametime\":{}",
            self.interpolate, self.width, self.height, self.frametime
        ));
        if let Some(frames) = &self.frames {
            out.push_str(",\"frames\":[");
            for (i, f) in frames.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&format!("{{\"index\":{},\"time\":{}}}", f.index, f.time));
            }
            out.push(']');
        }
        out.push('}');
    }
}

fn read_frames(v: &Value) -> GResult<Option<Vec<FrameMeta>>> {
    let items = v.as_array().ok_or(GsonError("expected an array of frames"))?;
    let mut frames = Vec::with_capacity(items.len());
    for item in items {
        let mut frame = FrameMeta { index: 0, time: -1 };
        if item.is_number() {
            frame.index = gson::int(item)?;
        } else {
            for (name, value) in gson::object(item)? {
                match name.as_str() {
                    "index" => frame.index = gson::int(value)?,
                    "time" => frame.time = gson::double(value)? as i32,
                    _ => {}
                }
            }
        }
        frames.push(frame);
    }
    Ok((!frames.is_empty()).then_some(frames))
}
