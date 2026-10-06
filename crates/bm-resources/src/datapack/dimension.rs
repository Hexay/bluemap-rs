//! `data/<ns>/dimension_type/*.json` (`DimensionTypeData`, Gson snake_case): missing fields are false/0/none, so
//! 26.x files without `fixed_time` read as having none.

use bm_world::DimensionType;
use serde_json::Value;

use super::{DataError, gson};

pub fn dimension_type_from_json(v: &Value) -> Result<DimensionType, DataError> {
    let Value::Object(data) = v else {
        return Err(DataError::Invalid { field: "dimension_type".into(), expected: "an object" });
    };
    let mut t = DimensionType {
        has_skylight: false,
        has_ceiling: false,
        ambient_light: 0.0,
        min_y: 0,
        height: 0,
        fixed_time: None,
        coordinate_scale: 0.0,
    };
    for (name, x) in data {
        if x.is_null() {
            continue;
        }
        match name.as_str() {
            // read only for its failure modes; bm_world has no use for it
            "natural" => _ = gson::boolean(x, name)?,
            "has_skylight" => t.has_skylight = gson::boolean(x, name)?,
            "has_ceiling" => t.has_ceiling = gson::boolean(x, name)?,
            "ambient_light" => t.ambient_light = gson::double(x, name)? as f32,
            "min_y" => t.min_y = gson::int(x, name)?,
            "height" => t.height = gson::int(x, name)?,
            "fixed_time" => t.fixed_time = Some(gson::long(x, name)?),
            "coordinate_scale" => t.coordinate_scale = gson::double(x, name)?,
            _ => {}
        }
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn reads_snake_case_fields() {
        let t = dimension_type_from_json(&json!({
            "ambient_light": 0.1, "coordinate_scale": 8.0, "has_ceiling": true, "has_fixed_time": true,
            "has_skylight": false, "height": 256, "min_y": 0, "natural": "false", "attributes": {}
        }))
        .unwrap();
        assert_eq!(t, DimensionType { fixed_time: None, ..DimensionType::NETHER });
        let t = dimension_type_from_json(&json!({"fixed_time": 18000, "min_y": "-64"})).unwrap();
        assert_eq!((t.fixed_time, t.min_y, t.coordinate_scale), (Some(18000), -64, 0.0));
    }

    #[test]
    fn bad_fields_fail() {
        for bad in [json!(null), json!({"natural": 1}), json!({"height": 1.5}), json!({"min_y": []})] {
            assert!(dimension_type_from_json(&bad).is_err(), "{bad}");
        }
    }
}
