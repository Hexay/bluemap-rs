use serde_json::json;

use super::*;
use crate::{read_frame, write_frame};

#[test]
fn headers_are_tagged_camel_case() {
    let hello = ShimMsg::Hello {
        protocol: 1,
        platform: "paper".into(),
        mc_version: "26.3".into(),
        shim_version: "0.1.0".into(),
        config_folder: "plugins/BlueMap".into(),
        mods_folder: None,
        metrics: None,
        folia: false,
        max_memory_mib: Some(2048),
        worlds: vec![],
    };
    let v = serde_json::to_value(&hello).unwrap();
    assert_eq!(v["t"], "Hello");
    assert_eq!(v["mcVersion"], "26.3");
    assert_eq!(v["maxMemoryMib"], 2048);
    assert_eq!(serde_json::from_value::<ShimMsg>(v).unwrap(), hello);

    let reply: ShimMsg = serde_json::from_value(json!({"t": "Reply", "id": 7, "ok": true, "value": true})).unwrap();
    assert_eq!(reply, ShimMsg::Reply(Reply::ok(7, json!(true))));
    assert_eq!(
        serde_json::to_value(CoreMsg::Reply(Reply::err(3, "no"))).unwrap(),
        json!({"t": "Reply", "id": 3, "ok": false, "err": "no"})
    );
    assert_eq!(serde_json::to_value(CoreMsg::Bye).unwrap(), json!({"t": "Bye"}));
    let state = CoreMsg::StateChanged(StateInfo { render_threads_running: true, ..StateInfo::default() });
    assert_eq!(serde_json::to_value(&state).unwrap()["renderThreadsRunning"], true);
}

#[test]
fn optional_fields_may_be_missing() {
    let m: ShimMsg = serde_json::from_value(json!({"t": "Markers", "map": "world"})).unwrap();
    assert_eq!(m, ShimMsg::Markers { map: "world".into(), more: false });
    let s: ShimMsg = serde_json::from_value(json!({"t": "Schedule", "id": 1, "map": "w", "extra": 5})).unwrap();
    assert_eq!(s, ShimMsg::Schedule { id: 1, map: "w".into(), regions: None, force: false });
}

#[test]
fn server_load_round_trips_through_a_frame() {
    let mut wire = Vec::new();
    write_frame(&mut wire, &ShimMsg::ServerLoad { mspt: 52.5 }, &[]).unwrap();
    let frame = read_frame(&mut wire.as_slice()).unwrap().unwrap();
    assert_eq!(frame.kind(), "ServerLoad");
    assert_eq!(frame.parse::<ShimMsg>().unwrap(), ShimMsg::ServerLoad { mspt: 52.5 });
    let int: ShimMsg = serde_json::from_value(json!({"t": "ServerLoad", "mspt": 50})).unwrap();
    assert_eq!(int, ShimMsg::ServerLoad { mspt: 50.0 });
}
