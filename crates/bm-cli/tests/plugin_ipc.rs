//! `bluemap --plugin-ipc` against a scripted shim: handshake, config generation, `NotReady` without resources,
//! command round trip, the single-core lock, `Shutdown` and stdin EOF.

use std::io::Write;
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use bm_ipc::{CommandSender, CoreMsg, PROTOCOL, ShimMsg, read_frame, write_frame};

fn spawn(cwd: &Path) -> (Child, ChildStdin, ChildStdout) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_bluemap"))
        .args(["--plugin-ipc", "--parent-pid", &std::process::id().to_string()])
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let (stdin, stdout) = (child.stdin.take().unwrap(), child.stdout.take().unwrap());
    (child, stdin, stdout)
}

fn send(w: &mut impl Write, msg: &ShimMsg) {
    write_frame(w, msg, &[]).unwrap();
}

fn hello(cwd: &Path) -> ShimMsg {
    ShimMsg::Hello {
        protocol: PROTOCOL,
        platform: "paper".into(),
        mc_version: "26.3".into(),
        shim_version: "test".into(),
        config_folder: cwd.join("plugins/BlueMap").display().to_string(),
        mods_folder: Some("mods".into()),
        metrics: None,
        folia: false,
        max_memory_mib: Some(1024),
        worlds: vec![],
    }
}

/// Next non-log message.
fn next(r: &mut ChildStdout) -> CoreMsg {
    loop {
        let frame = read_frame(r).unwrap().expect("core closed the stream");
        let msg: CoreMsg = frame.parse().unwrap();
        if !matches!(msg, CoreMsg::Log { .. } | CoreMsg::MarkerDemand { .. }) {
            return msg;
        }
    }
}

#[test]
fn lifecycle_without_resources() {
    let dir = tempfile::tempdir().unwrap();
    let (mut child, mut stdin, mut stdout) = spawn(dir.path());
    send(&mut stdin, &hello(dir.path()));
    assert!(matches!(next(&mut stdout), CoreMsg::Welcome { protocol: PROTOCOL, .. }));
    match next(&mut stdout) {
        CoreMsg::NotReady { reason, .. } => assert_eq!(reason, "missing-resources"),
        other => panic!("expected NotReady, got {other:?}"),
    }
    let config = dir.path().join("plugins/BlueMap");
    for file in ["core.conf", "webserver.conf", "webapp.conf", "plugin.conf", "storages/file.conf"] {
        assert!(config.join(file).is_file(), "{file} not generated");
    }

    let sender = CommandSender {
        kind: "console".into(),
        name: "CONSOLE".into(),
        world: None,
        position: None,
        permissions: vec!["bluemap.status".into(), "bluemap.debug.dump".into()],
    };
    send(&mut stdin, &ShimMsg::Command { id: 7, input: "bluemap".into(), sender: sender.clone() });
    match next(&mut stdout) {
        CoreMsg::CommandOutput { id: 7, component } => assert!(component.to_string().contains("not loaded")),
        other => panic!("expected CommandOutput, got {other:?}"),
    }
    assert!(matches!(next(&mut stdout), CoreMsg::CommandDone { id: 7, result: 0 }));

    send(&mut stdin, &ShimMsg::Command { id: 8, input: "bluemap debug dump".into(), sender });
    assert!(matches!(next(&mut stdout), CoreMsg::CommandOutput { id: 8, .. }));
    assert!(matches!(next(&mut stdout), CoreMsg::CommandDone { id: 8, result: 1 }));
    let dump = std::fs::read_to_string(dir.path().join("dump.json")).unwrap();
    assert!(dump.starts_with("{\n \"system-info\": {\n  \""), "StateDumper's layout and indent: {dump}");
    let dump: serde_json::Value = serde_json::from_str(&dump).unwrap();
    assert_eq!(dump["system-info"]["bluemap-version"], "5.28");
    assert_eq!(dump["dump"][0]["#identity"], "Plugin");
    assert_eq!(dump["dump"][0]["loaded"], false);
    assert!(dump["registries"].is_array() && dump["threads"].is_array());

    let (mut second, mut stdin2, mut stdout2) = spawn(dir.path());
    send(&mut stdin2, &hello(dir.path()));
    assert!(matches!(next(&mut stdout2), CoreMsg::Welcome { .. }));
    assert_eq!(second.wait().unwrap().code(), Some(4));

    send(&mut stdin, &ShimMsg::Shutdown);
    assert!(matches!(next(&mut stdout), CoreMsg::Bye));
    assert!(child.wait().unwrap().success());
    assert!(!config.join(".core.pid").exists());
}

/// Next `Log` line containing `needle`; panics on `Bye` or EOF.
fn log_containing(r: &mut ChildStdout, needle: &str) -> (bm_ipc::LogLevel, String) {
    loop {
        let frame = read_frame(r).unwrap().expect("core closed the stream");
        match frame.parse::<CoreMsg>().unwrap() {
            CoreMsg::Log { level, msg, .. } if msg.contains(needle) => return (level, msg),
            CoreMsg::Bye => panic!("core said Bye before logging '{needle}'"),
            _ => {}
        }
    }
}

#[test]
fn server_load_pauses_and_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let (mut child, mut stdin, mut stdout) = spawn(dir.path());
    send(&mut stdin, &hello(dir.path()));
    assert!(matches!(next(&mut stdout), CoreMsg::Welcome { .. }));
    assert!(matches!(next(&mut stdout), CoreMsg::NotReady { .. }));

    for _ in 0..4 {
        send(&mut stdin, &ShimMsg::ServerLoad { mspt: 30.0 });
    }
    for _ in 0..6 {
        send(&mut stdin, &ShimMsg::ServerLoad { mspt: 120.0 });
    }
    let (level, msg) = log_containing(&mut stdout, "lagging");
    assert_eq!(level, bm_ipc::LogLevel::Warning);
    assert!(msg.contains("MSPT 48.0"), "pauses on the 10 s average, at the first sample above 45: {msg}");
    for _ in 0..40 {
        send(&mut stdin, &ShimMsg::ServerLoad { mspt: 5.0 });
    }
    let (level, msg) = log_containing(&mut stdout, "recovered");
    assert_eq!(level, bm_ipc::LogLevel::Info);
    assert!(msg.contains("resuming rendering"), "{msg}");

    send(&mut stdin, &ShimMsg::Shutdown);
    loop {
        if matches!(next(&mut stdout), CoreMsg::Bye) {
            break;
        }
    }
    assert!(child.wait().unwrap().success());
}

#[test]
fn incompatible_protocol_and_eof() {
    let dir = tempfile::tempdir().unwrap();
    let (mut child, mut stdin, mut stdout) = spawn(dir.path());
    let mut wrong = hello(dir.path());
    if let ShimMsg::Hello { protocol, .. } = &mut wrong {
        *protocol = PROTOCOL + 1;
    }
    send(&mut stdin, &wrong);
    assert!(matches!(next(&mut stdout), CoreMsg::Incompatible { protocol: PROTOCOL, .. }));
    assert_eq!(child.wait().unwrap().code(), Some(3));

    let (mut child, mut stdin, mut stdout) = spawn(dir.path());
    send(&mut stdin, &hello(dir.path()));
    assert!(matches!(next(&mut stdout), CoreMsg::Welcome { .. }));
    drop(stdin);
    loop {
        if matches!(next(&mut stdout), CoreMsg::Bye) {
            break;
        }
    }
    assert!(child.wait().unwrap().success());
}
