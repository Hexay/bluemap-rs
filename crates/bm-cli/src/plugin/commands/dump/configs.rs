//! Config objects (`BlueMapConfigManager`'s fields) with upstream's field names.

use bm_config::{MapConfig, StorageConfig};
use serde::Serialize;
use serde_json::{Value, json};

use super::java::Ids;

pub const COMMON: &str = "de.bluecolored.bluemap.common";

pub fn identity_of(v: &Value) -> String {
    v["#identity"].as_str().unwrap_or_default().to_owned()
}

/// A config struct by its serde field names; `nested` names the Java class of object and collection fields.
pub fn config<T: Serialize>(ids: &mut Ids, class: &str, value: &T, nested: &[(&str, &str)]) -> Value {
    let Ok(Value::Object(fields)) = serde_json::to_value(value) else { return Value::Null };
    let fields = fields
        .into_iter()
        .map(|(key, v)| {
            let class = nested.iter().find(|(k, _)| *k == key).map(|(_, c)| *c);
            let v = match (v, class) {
                (Value::Object(o), Some(c)) => {
                    let f: Vec<(&str, Value)> = o.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
                    ids.object(c, f)
                }
                (Value::Array(a), Some(c)) if c.ends_with("Map") => {
                    let pairs = a.into_iter().map(|p| (p[0].clone(), p[1].clone())).collect();
                    ids.map(c, pairs)
                }
                (Value::Array(a), Some(c)) => ids.list(c, a),
                (v, _) => v,
            };
            (key, v)
        })
        .collect::<Vec<_>>();
    ids.object(class, fields.iter().map(|(k, v)| (k.as_str(), v.clone())).collect())
}

/// `Float.toString`-like JSON for an `f32` field (its shortest decimal, not the widened `f64`).
fn float(v: f32) -> Value {
    v.to_string().parse::<f64>().map_or(Value::Null, Value::from)
}

pub fn map_config(ids: &mut Ids, c: &MapConfig) -> Value {
    let start = ids.object(
        "com.flowpowered.math.vector.Vector2i",
        vec![("#toString", json!(format!("({}, {})", c.start_pos[0], c.start_pos[1])))],
    );
    let fields = vec![
        ("loader", json!(c.loader.to_string())),
        ("world", json!(c.world.as_ref().map(|w| w.display().to_string()))),
        ("dimension", json!(c.dimension.as_ref().map(ToString::to_string))),
        ("dimensionType", json!(c.dimension_type.as_ref().map(ToString::to_string))),
        ("name", json!(c.name)),
        ("sorting", json!(c.sorting)),
        ("startPos", start),
        ("skyColor", json!(c.sky_color)),
        ("voidColor", json!(c.void_color)),
        ("ambientLight", float(c.ambient_light)),
        ("skyLight", float(c.sky_light)),
        ("removeCavesBelowY", json!(c.remove_caves_below_y)),
        ("caveDetectionOceanFloor", json!(c.cave_detection_ocean_floor)),
        ("caveDetectionUsesBlockLight", json!(c.cave_detection_uses_block_light)),
        ("minInhabitedTime", json!(c.min_inhabited_time)),
        ("minInhabitedTimeRadius", json!(c.min_inhabited_time_radius)),
        ("renderEdges", json!(c.render_edges)),
        ("edgeLightStrength", json!(c.edge_light_strength)),
        ("enablePerspectiveView", json!(c.enable_perspective_view)),
        ("enableFlatView", json!(c.enable_flat_view)),
        ("enableFreeFlightView", json!(c.enable_free_flight_view)),
        ("enableHires", json!(c.enable_hires)),
        ("checkForRemovedRegions", json!(c.check_for_removed_regions)),
        ("storage", json!(c.storage)),
        ("ignoreMissingLightData", json!(c.ignore_missing_light_data)),
        ("hiresTileSize", json!(c.hires_tile_size)),
        ("lowresTileSize", json!(c.lowres_tile_size)),
        ("lodCount", json!(c.lod_count)),
        ("lodFactor", json!(c.lod_factor)),
    ];
    ids.object(&format!("{COMMON}.config.MapConfig"), fields)
}

pub fn storage_config(ids: &mut Ids, c: &StorageConfig) -> Value {
    match c {
        StorageConfig::File(f) => ids.object(
            &format!("{COMMON}.config.storage.FileConfig"),
            vec![
                ("root", json!(f.root.display().to_string())),
                ("compression", json!(f.compression)),
                ("atomic", json!(f.atomic)),
                ("storageType", json!("file")),
                ("format", json!(format!("{:?}", f.format).to_lowercase())),
            ],
        ),
        // connectionUrl and connectionProperties are @DebugDump(exclude = true): they may hold credentials
        StorageConfig::Sql(s) => ids.object(
            &format!("{COMMON}.config.storage.SQLConfig"),
            vec![
                ("dialect", json!(s.dialect)),
                ("driverJar", json!(s.driver_jar)),
                ("driverClass", json!(s.driver_class)),
                ("maxConnections", json!(s.max_connections)),
                ("connectionInitSql", json!(s.connection_init_sql)),
                ("tablePrefix", json!(s.table_prefix)),
                ("compression", json!(s.compression)),
                ("storageType", json!("sql")),
                ("format", json!(format!("{:?}", s.format).to_lowercase())),
            ],
        ),
    }
}
