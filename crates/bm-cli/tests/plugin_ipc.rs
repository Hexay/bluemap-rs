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
        permissions: vec!["bluemap.status".into()],
    };
    send(&mut stdin, &ShimMsg::Command { id: 7, input: "bluemap".into(), sender });
    match next(&mut stdout) {
        CoreMsg::CommandOutput { id: 7, component } => assert!(component.to_string().contains("not loaded")),
        other => panic!("expected CommandOutput, got {other:?}"),
    }
    assert!(matches!(next(&mut stdout), CoreMsg::CommandDone { id: 7, result: 0 }));

    let (mut second, mut stdin2, mut stdout2) = spawn(dir.path());
    send(&mut stdin2, &hello(dir.path()));
    assert!(matches!(next(&mut stdout2), CoreMsg::Welcome { .. }));
    assert_eq!(second.wait().unwrap().code(), Some(4));

    send(&mut stdin, &ShimMsg::Shutdown);
    assert!(matches!(next(&mut stdout), CoreMsg::Bye));
    assert!(child.wait().unwrap().success());
    assert!(!config.join(".core.pid").exists());
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
