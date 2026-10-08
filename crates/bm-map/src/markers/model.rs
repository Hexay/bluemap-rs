//! BlueMapAPI's marker classes as MarkerGson deserializes them: private no-arg constructor defaults, then every
//! known `lower-case-with-dashes` field the JSON names overwrites its default (unknown keys are skipped).

use super::json::Json;
use super::read::{boolean, double, float, string, tree_int};

type R<T> = Result<T, String>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Color {
    pub r: i32,
    pub g: i32,
    pub b: i32,
    pub a: f32,
}

const LINE_COLOR: Color = Color { r: 255, g: 0, b: 0, a: 1.0 };
const FILL_COLOR: Color = Color { r: 200, g: 0, b: 0, a: 0.3 };

/// The subclass's own fields, which Gson writes first.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Head {
    Poi { classes: Vec<String>, detail: String, icon: String, anchor: [i32; 2] },
    Html { classes: Vec<String>, anchor: [i32; 2], html: String },
    Line { line: Vec<[f64; 3]> },
    Shape { shape: Vec<[f64; 2]>, holes: Vec<Vec<[f64; 2]>>, y: f32 },
    Extrude { shape: Vec<[f64; 2]>, holes: Vec<Vec<[f64; 2]>>, min_y: f32, max_y: f32 },
}

/// Line/shape/extrude drawing fields; `fill_color` only on shapes and extrudes.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Style {
    pub depth_test: bool,
    pub line_width: i32,
    pub line_color: Color,
    pub fill_color: Option<Color>,
}

/// `ObjectMarker` (line/shape/extrude).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ObjectFields {
    pub detail: String,
    pub link: Option<String>,
    pub new_tab: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Marker {
    pub head: Head,
    pub style: Option<Style>,
    pub object: Option<ObjectFields>,
    pub min_distance: f64,
    pub max_distance: f64,
    pub kind: String,
    pub label: String,
    pub position: [f64; 3],
    pub sorting: i32,
    pub listed: bool,
}

/// `Shape.createRect(0, 0, 1, 1)`, the default shape; its center is the default position.
const UNIT_RECT: [[f64; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

impl Marker {
    /// `MarkerDeserializer`: dispatch on `type`, then the subclass's reflective adapter.
    pub fn read(v: &Json) -> R<Marker> {
        let Json::Obj(members) = v else { return Err(format!("expected an object, got {}", v.kind())) };
        let kind = match v.get("type") {
            None => return Err("missing \"type\"".into()),
            Some(t @ (Json::Arr(_) | Json::Obj(_) | Json::Null)) => return Err(format!("\"type\" is {}", t.kind())),
            Some(t) => string(t)?,
        };
        let mut marker = Marker::new(&kind).ok_or_else(|| format!("Unknown marker type: {kind}"))?;
        for (key, value) in members {
            marker.set(key, value).map_err(|e| format!("{key}: {e}"))?;
        }
        Ok(marker)
    }

    fn new(kind: &str) -> Option<Marker> {
        let style = |fill: bool| Style {
            depth_test: true,
            line_width: 2,
            line_color: LINE_COLOR,
            fill_color: fill.then_some(FILL_COLOR),
        };
        let object = || ObjectFields { detail: String::new(), link: None, new_tab: false };
        let shape_center = [0.5, 0.0, 0.5];
        let (head, style, object, position) = match kind {
            "poi" => {
                let head = Head::Poi {
                    classes: vec![],
                    detail: String::new(),
                    icon: "assets/poi.svg".into(),
                    anchor: [25, 45],
                };
                (head, None, None, [0.0; 3])
            }
            "html" => (Head::Html { classes: vec![], anchor: [0, 0], html: String::new() }, None, None, [0.0; 3]),
            "line" => (Head::Line { line: vec![[0.0; 3], [1.0; 3]] }, Some(style(false)), Some(object()), [0.5; 3]),
            "shape" => {
                let head = Head::Shape { shape: UNIT_RECT.to_vec(), holes: vec![], y: 0.0 };
                (head, Some(style(true)), Some(object()), shape_center)
            }
            "extrude" => {
                let head = Head::Extrude { shape: UNIT_RECT.to_vec(), holes: vec![], min_y: 0.0, max_y: 0.0 };
                (head, Some(style(true)), Some(object()), shape_center)
            }
            _ => return None,
        };
        let (min_distance, max_distance) = (0.0, 10_000_000.0);
        let (label, sorting, listed) = (String::new(), 0, true);
        Some(Marker {
            head,
            style,
            object,
            min_distance,
            max_distance,
            kind: kind.into(),
            label,
            position,
            sorting,
            listed,
        })
    }

    fn set(&mut self, key: &str, v: &Json) -> R<()> {
        if self.head.set(key, v)? {
            return Ok(());
        }
        if let Some(style) = &mut self.style {
            match key {
                "depth-test" => return boolean(v).map(|b| style.depth_test = b),
                "line-width" => return tree_int(v).map(|w| style.line_width = w),
                "line-color" => return color(v).map(|c| style.line_color = c),
                "fill-color" if style.fill_color.is_some() => return color(v).map(|c| style.fill_color = Some(c)),
                _ => {}
            }
        }
        if let Some(object) = &mut self.object {
            match key {
                "detail" => return string(v).map(|s| object.detail = s),
                "link" => return string(v).map(|s| object.link = Some(s)),
                "new-tab" => return boolean(v).map(|b| object.new_tab = b),
                _ => {}
            }
        }
        match key {
            "min-distance" => self.min_distance = double(v)?,
            "max-distance" => self.max_distance = double(v)?,
            "type" => self.kind = string(v)?,
            "label" => self.label = string(v)?,
            "position" => self.position = vec3(v)?,
            "sorting" => self.sorting = tree_int(v)?,
            "listed" => self.listed = boolean(v)?,
            _ => {}
        }
        Ok(())
    }
}

impl Head {
    /// `Ok(true)` when `key` is one of this subclass's own fields.
    fn set(&mut self, key: &str, v: &Json) -> R<bool> {
        match (self, key) {
            (Head::Poi { classes, .. } | Head::Html { classes, .. }, "classes") => *classes = string_set(v)?,
            (Head::Poi { anchor, .. } | Head::Html { anchor, .. }, "anchor") => *anchor = vec2i(v)?,
            (Head::Poi { detail, .. }, "detail") => *detail = string(v)?,
            (Head::Poi { icon, .. }, "icon") => *icon = string(v)?,
            (Head::Html { html, .. }, "html") => *html = string(v)?,
            (Head::Line { line }, "line") => *line = points(v, 2, vec3)?,
            (Head::Shape { shape, .. } | Head::Extrude { shape, .. }, "shape") => *shape = points(v, 3, vec2_xz)?,
            (Head::Shape { holes, .. } | Head::Extrude { holes, .. }, "holes") => {
                *holes = array(v)?.iter().map(|h| points(h, 3, vec2_xz)).collect::<R<_>>()?
            }
            (Head::Shape { y, .. }, "shape-y") => *y = float(v)?,
            (Head::Extrude { min_y, .. }, "shape-min-y") => *min_y = float(v)?,
            (Head::Extrude { max_y, .. }, "shape-max-y") => *max_y = float(v)?,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

fn array(v: &Json) -> R<&[Json]> {
    match v {
        Json::Arr(items) => Ok(items),
        _ => Err(format!("expected an array, got {}", v.kind())),
    }
}

fn object(v: &Json) -> R<&[(String, Json)]> {
    match v {
        Json::Obj(members) => Ok(members),
        _ => Err(format!("expected an object, got {}", v.kind())),
    }
}

/// `Set<String>` deserializes into a `LinkedHashSet`: order kept, duplicates dropped.
fn string_set(v: &Json) -> R<Vec<String>> {
    let mut set: Vec<String> = Vec::new();
    for item in array(v)? {
        let s = string(item)?;
        if !set.contains(&s) {
            set.push(s);
        }
    }
    Ok(set)
}

/// `Line`/`Shape` adapters: an array of points; the constructors reject fewer than `min`.
fn points<P>(v: &Json, min: usize, point: fn(&Json) -> R<P>) -> R<Vec<P>> {
    let points = array(v)?.iter().map(point).collect::<R<Vec<P>>>()?;
    if points.len() < min { Err(format!("needs at least {min} points, has {}", points.len())) } else { Ok(points) }
}

/// Vector adapters: unknown components are skipped, missing ones stay 0.
fn components<const N: usize, T: Default + Copy>(v: &Json, names: [&str; N], read: fn(&Json) -> R<T>) -> R<[T; N]> {
    let mut out = [T::default(); N];
    for (key, value) in object(v)? {
        if let Some(i) = names.iter().position(|n| n == key) {
            out[i] = read(value)?;
        }
    }
    Ok(out)
}

fn vec3(v: &Json) -> R<[f64; 3]> {
    components(v, ["x", "y", "z"], double)
}

/// `Vector2dAdapter(useZ)`: shape points are `{x, z}`.
fn vec2_xz(v: &Json) -> R<[f64; 2]> {
    components(v, ["x", "z"], double)
}

fn vec2i(v: &Json) -> R<[i32; 2]> {
    components(v, ["x", "y"], tree_int)
}

/// `ColorAdapter`: an `{r, g, b, a}` object only; alpha defaults to 1.
fn color(v: &Json) -> R<Color> {
    let [r, g, b] = components(v, ["r", "g", "b"], tree_int)?;
    let a = match object(v)?.iter().rev().find(|(k, _)| k == "a") {
        Some((_, a)) => float(a)?,
        None => 1.0,
    };
    Ok(Color { r, g, b, a })
}
