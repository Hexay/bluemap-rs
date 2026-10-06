use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

#[derive(Debug, thiserror::Error)]
#[error("no axis named '{0}'")]
pub struct ParseAxisError(String);

impl Axis {
    pub fn to_vector(self) -> [i32; 3] {
        match self {
            Axis::X => [1, 0, 0],
            Axis::Y => [0, 1, 0],
            Axis::Z => [0, 0, 1],
        }
    }
}

impl FromStr for Axis {
    type Err = ParseAxisError;

    /// `Axis.fromString`: case-insensitive.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            "x" | "X" => Ok(Axis::X),
            "y" | "Y" => Ok(Axis::Y),
            "z" | "Z" => Ok(Axis::Z),
            _ => Err(ParseAxisError(name.to_owned())),
        }
    }
}
