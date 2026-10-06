use serde_json::Value;

use super::gson::{self, non_null, opt_bool, opt_i32, opt_strict_string, opt_vec};
use super::{Direction, ModelError, Rotation, TextureVariable, missing_texture};

const FULL_BLOCK_MIN: [f32; 3] = [0.0, 0.0, 0.0];
const FULL_BLOCK_MAX: [f32; 3] = [16.0, 16.0, 16.0];

/// A model element (`Element.java`): a box from `from` to `to` in 1/16 block units with up to six faces.
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub rotation: Rotation,
    pub shade: bool,
    pub light_emission: i32,
    /// Indexed by [`Direction::index`].
    pub faces: [Option<Face>; 6],
}

/// One element face (`Face.java`). `uv` is in texture pixels (0..16), defaulted from the element bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub uv: [f32; 4],
    pub texture: TextureVariable,
    pub cullface: Option<Direction>,
    pub rotation: i32,
    pub tintindex: i32,
}

impl Element {
    pub fn new(from: [f32; 3], to: [f32; 3], rotation: Rotation, faces: [Option<Face>; 6]) -> Self {
        Self { from, to, rotation, shade: true, light_emission: 0, faces }
    }

    pub fn from_value(v: &Value) -> Result<Self, ModelError> {
        let obj = gson::object(v, "element")?;
        let from = opt_vec(obj, "from")?.unwrap_or(FULL_BLOCK_MIN);
        let to = opt_vec(obj, "to")?.unwrap_or(FULL_BLOCK_MAX);
        let rotation = non_null(obj, "rotation").map(Rotation::from_value).transpose()?.unwrap_or_default();
        let mut element = Self {
            shade: opt_bool(obj, "shade")?.unwrap_or(true),
            light_emission: opt_i32(obj, "light_emission", 0)?,
            ..Self::new(from, to, rotation, Default::default())
        };
        match obj.get("faces") {
            None => {}
            // a null map makes Element.init throw
            Some(Value::Null) => return Err(ModelError::Type { field: "faces", expected: "an object" }),
            Some(faces) => element.read_faces(gson::object(faces, "faces")?)?,
        }
        Ok(element)
    }

    fn read_faces(&mut self, faces: &gson::Obj) -> Result<(), ModelError> {
        let mut seen = 0u8;
        for (name, value) in faces {
            let dir = Direction::parse(name)?;
            let bit = 1 << dir.index();
            // Gson's map adapter rejects a key read twice, e.g. "up" and "top"
            if seen & bit != 0 {
                return Err(ModelError::DuplicateFace(name.clone()));
            }
            seen |= bit;
            self.faces[dir.index()] = Some(Face::from_value(value, self.default_uv(dir))?);
        }
        Ok(())
    }

    pub fn face(&self, dir: Direction) -> Option<&Face> {
        self.faces[dir.index()].as_ref()
    }

    /// `calculateDefaultUV`.
    pub fn default_uv(&self, dir: Direction) -> [f32; 4] {
        let ([fx, fy, fz], [tx, ty, tz]) = (self.from, self.to);
        match dir {
            Direction::Up => [fx, fz, tx, tz],
            Direction::Down => [fx, 16.0 - tz, tx, 16.0 - fz],
            Direction::North => [16.0 - tx, 16.0 - ty, 16.0 - fx, 16.0 - fy],
            Direction::South => [fx, 16.0 - ty, tx, 16.0 - fy],
            Direction::East => [16.0 - tz, 16.0 - ty, 16.0 - fz, 16.0 - fy],
            Direction::West => [fz, 16.0 - ty, tz, 16.0 - fy],
        }
    }

    /// `isFullCube`: exactly 0..16 on every axis (`Float.compare`, so -0 is not 0) with all six faces.
    pub fn is_full_cube(&self) -> bool {
        java_eq(self.from, FULL_BLOCK_MIN) && java_eq(self.to, FULL_BLOCK_MAX) && self.faces.iter().all(Option::is_some)
    }
}

fn java_eq(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()))
}

impl Face {
    pub fn new(uv: [f32; 4], texture: TextureVariable) -> Self {
        Self { uv, texture, cullface: None, rotation: 0, tintindex: -1 }
    }

    fn from_value(v: &Value, default_uv: [f32; 4]) -> Result<Self, ModelError> {
        let obj = gson::object(v, "face")?;
        let cullface = opt_strict_string(obj, "cullface")?.map(|name| Direction::parse(&name)).transpose()?;
        let texture = match non_null(obj, "texture") {
            Some(t) => TextureVariable::from_value(t)?,
            None => TextureVariable::Path(missing_texture()),
        };
        Ok(Self {
            uv: opt_vec(obj, "uv")?.unwrap_or(default_uv),
            texture,
            cullface,
            rotation: opt_i32(obj, "rotation", 0)?,
            tintindex: opt_i32(obj, "tintindex", -1)?,
        })
    }
}
