//! `live/players.json` exactly as `LivePlayersDataSupplier` writes it: a plain Gson `JsonWriter` (not HTML-safe),
//! doubles through `Double.toString`.

use bm_java::fmt::double_to_string;

/// `LivePlayerInfo` plus the `foreign` flag (player is on another world than the map's).
#[derive(Debug, Clone, PartialEq)]
pub struct LivePlayer<'a> {
    pub uuid: &'a str,
    pub name: &'a str,
    pub foreign: bool,
    pub position: [f64; 3],
    /// pitch, yaw, roll.
    pub rotation: [f64; 3],
}

pub fn players_json<'a>(players: impl IntoIterator<Item = LivePlayer<'a>>) -> String {
    let mut out = String::from("{\"players\":[");
    for (i, p) in players.into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"uuid\":");
        push_string(&mut out, p.uuid);
        out.push_str(",\"name\":");
        push_string(&mut out, p.name);
        out.push_str(if p.foreign { ",\"foreign\":true" } else { ",\"foreign\":false" });
        let [x, y, z] = p.position.map(double_to_string);
        out.push_str(&format!(",\"position\":{{\"x\":{x},\"y\":{y},\"z\":{z}}}"));
        let [pitch, yaw, roll] = p.rotation.map(double_to_string);
        out.push_str(&format!(",\"rotation\":{{\"pitch\":{pitch},\"yaw\":{yaw},\"roll\":{roll}}}}}"));
    }
    out.push_str("]}");
    out
}

/// `JsonWriter.string` without `htmlSafe`.
fn push_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{c}' => out.push_str("\\f"),
            '\u{2028}' | '\u{2029}' | '\0'..='\u{1f}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_list() {
        assert_eq!(players_json([]), "{\"players\":[]}");
    }

    #[test]
    fn matches_json_writer() {
        let p = LivePlayer {
            uuid: "0b2a8e4c-1f1e-4d5e-9b7a-3c2d1e0f9a8b",
            name: "Al<i>ce\"\u{1}",
            foreign: false,
            position: [12.5, 64.0, -3.25e-4],
            // (double) 12.345f, the way Vector3d widens Location's floats
            rotation: [f64::from(12.345_f32), -90.0, 0.0],
        };
        let other = LivePlayer { uuid: "u2", name: "Bob", foreign: true, position: [1e7, 0.0, -0.0], rotation: [0.0; 3] };
        assert_eq!(
            players_json([p, other]),
            "{\"players\":[{\"uuid\":\"0b2a8e4c-1f1e-4d5e-9b7a-3c2d1e0f9a8b\",\"name\":\"Al<i>ce\\\"\\u0001\",\
             \"foreign\":false,\"position\":{\"x\":12.5,\"y\":64.0,\"z\":-3.25E-4},\
             \"rotation\":{\"pitch\":12.345000267028809,\"yaw\":-90.0,\"roll\":0.0}},\
             {\"uuid\":\"u2\",\"name\":\"Bob\",\"foreign\":true,\"position\":{\"x\":1.0E7,\"y\":0.0,\"z\":-0.0},\
             \"rotation\":{\"pitch\":0.0,\"yaw\":0.0,\"roll\":0.0}}]}"
        );
    }
}
