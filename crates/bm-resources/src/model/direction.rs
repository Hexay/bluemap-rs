use super::ModelError;

/// `util.Direction`, in Java ordinal order (array indices follow it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Up,
    Down,
    North,
    South,
    West,
    East,
}

impl Direction {
    pub const ALL: [Direction; 6] = [Self::Up, Self::Down, Self::North, Self::South, Self::West, Self::East];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn to_vector(self) -> [i32; 3] {
        match self {
            Self::Up => [0, 1, 0],
            Self::Down => [0, -1, 0],
            Self::North => [0, 0, -1],
            Self::South => [0, 0, 1],
            Self::West => [-1, 0, 0],
            Self::East => [1, 0, 0],
        }
    }

    pub fn opposite(self) -> Self {
        match self {
            Self::Up => Self::Down,
            Self::Down => Self::Up,
            Self::North => Self::South,
            Self::South => Self::North,
            Self::West => Self::East,
            Self::East => Self::West,
        }
    }

    /// `getLocalUp`: the direction that is "up" in a face's texture.
    pub fn local_up(self) -> Self {
        match self {
            Self::Up => Self::North,
            Self::Down => Self::South,
            _ => Self::Up,
        }
    }

    /// `DirectionAdapter`: `bottom`/`top` aliases, otherwise `Direction.fromString` (case-insensitive).
    pub fn parse(name: &str) -> Result<Self, ModelError> {
        if name.eq_ignore_ascii_case("bottom") {
            return Ok(Self::Down);
        }
        if name.eq_ignore_ascii_case("top") {
            return Ok(Self::Up);
        }
        // Java's String.toUpperCase, which also folds e.g. 'ſ' to 'S'
        match name.to_uppercase().as_str() {
            "UP" => Ok(Self::Up),
            "DOWN" => Ok(Self::Down),
            "NORTH" => Ok(Self::North),
            "SOUTH" => Ok(Self::South),
            "WEST" => Ok(Self::West),
            "EAST" => Ok(Self::East),
            _ => Err(ModelError::Direction(name.to_owned())),
        }
    }
}
