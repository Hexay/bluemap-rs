use super::*;

/// BlueNBT 3.5.1 output for two region updates of `world` and one unknown task (generated on Java 21).
const JAVA: &str = "0a000009000b72656e6465725461736b730a00000003080004747970650015626c75656d61703a726567696f6e2d7570646174650a0004646174610800036d61700005776f726c64090009726567696f6e506f73030000000200000000ffffffff080005666f7263650012626c75656d61703a666f7263652d6e6f6e650000080004747970650015626c75656d61703a726567696f6e2d7570646174650a0004646174610800036d61700005776f726c64090009726567696f6e506f73030000000200000001ffffffff080005666f7263650012626c75656d61703a666f7263652d6e6f6e65000008000474797065000f626c75656d61703a756e6b6e6f776e0000";
const EMPTY_JAVA: &str = "0a000009000b72656e6465725461736b730a0000000000";

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn java() -> Vec<u8> {
    // the probe used a made-up key; the real registry key is force_none (same length)
    let fixed = JAVA.replace(&hex_of("force-none"), &hex_of("force_none"));
    hex(&fixed)
}

fn hex_of(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

fn region(x: i32, z: i32) -> RenderTask {
    RenderTask::region("world", (x, z))
}

#[test]
fn encodes_like_bluenbt() {
    let tasks = [region(0, -1), region(1, -1), RenderTask::full("gone", TileUpdateStrategy::ForceNone)];
    assert_eq!(encode(&tasks, &[], &|_| None), java());
    assert_eq!(encode(&[], &[], &|_| None), hex(EMPTY_JAVA));
}

#[test]
fn decodes_java_and_round_trips_map_updates() {
    let loaded = decode(&java(), &|m| m == "world").unwrap();
    assert_eq!(loaded.tasks, [region(0, -1), region(1, -1)]);
    assert!(decode(&java(), &|_| false).unwrap().tasks.is_empty());

    let force = RenderTask::full("world", TileUpdateStrategy::ForceAll);
    let bytes = encode(&[force], &[], &|_| Some(vec![(0, 0), (1, 0), (2, 5)]));
    let back = decode(&bytes, &|_| true).unwrap();
    let expected =
        RenderTask::new("world", Regions::Only([(0, 0), (1, 0), (2, 5)].into()), TileUpdateStrategy::ForceAll);
    assert_eq!(back.tasks, [expected]);
}

#[test]
fn partly_done_tasks_resume_past_their_done_regions() {
    let mut force = RenderTask::full("world", TileUpdateStrategy::ForceAll);
    force.done = [(2, 5), (0, 0)].into();
    let mut finished = RenderTask::region("world", (7, 7));
    finished.done = [(7, 7)].into();
    let bytes = encode(&[force, finished], &["old".into()], &|_| Some(vec![(0, 0), (1, 0), (2, 5)]));
    let back = decode(&bytes, &|_| true).unwrap();
    let left = RenderTask::new("world", Regions::Only([(1, 0)].into()), TileUpdateStrategy::ForceAll);
    assert_eq!(back, Loaded { tasks: vec![left], purges: vec!["old".into()] }, "a fully done task is left out");
}

#[test]
fn resumes_map_updates_at_the_current_index() {
    let mut bytes =
        encode(&[RenderTask::full("world", TileUpdateStrategy::ForceNone)], &[], &|_| Some(vec![(0, 0), (1, 0)]));
    // currentTaskIndex is the last int before the two closing ENDs and the root END
    let n = bytes.len();
    bytes[n - 4] = 1;
    let back = decode(&bytes, &|_| true).unwrap();
    assert_eq!(back.tasks, [RenderTask { regions: Regions::Only([(1, 0)].into()), ..region(0, 0) }]);
}

#[test]
fn garbage_is_an_error() {
    assert!(decode(b"\x08nope", &|_| true).is_err());
}
