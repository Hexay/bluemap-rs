//! Core → shim frames: one writer thread behind a bounded queue. Logs never block (overflow is counted and
//! summarised), everything else applies backpressure. Also tracks the core's own requests to the shim.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufWriter;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use bm_ipc::{CoreMsg, LogLevel, Reply, write_frame};

const QUEUE: usize = 1024;

enum Out {
    Frame(CoreMsg, Vec<u8>),
    /// Only makes the writer report dropped logs.
    Wake,
    /// Writes `Bye` and ends the writer.
    Close,
}

pub struct Outbox {
    tx: SyncSender<Out>,
    logs_dropped: Arc<AtomicUsize>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, mpsc::Sender<Reply>>>,
    writer: Mutex<Option<JoinHandle<()>>>,
}

impl Outbox {
    pub fn start(out: File) -> Self {
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        let logs_dropped = Arc::new(AtomicUsize::new(0));
        let dropped = logs_dropped.clone();
        let writer = std::thread::Builder::new()
            .name("bluemap-ipc-out".into())
            .spawn(move || write_loop(out, rx, &dropped))
            .expect("spawn ipc writer");
        Self {
            tx,
            logs_dropped,
            next_id: AtomicU64::new(1),
            pending: Mutex::default(),
            writer: Mutex::new(Some(writer)),
        }
    }

    pub fn send(&self, msg: CoreMsg) {
        self.send_with_body(msg, Vec::new());
    }

    pub fn send_with_body(&self, msg: CoreMsg, body: Vec<u8>) {
        let _ = self.tx.send(Out::Frame(msg, body));
    }

    pub fn log(&self, level: LogLevel, msg: &str) {
        let frame = Out::Frame(CoreMsg::Log { level, msg: msg.to_owned(), trace: None }, Vec::new());
        if let Err(TrySendError::Full(_)) = self.tx.try_send(frame)
            && self.logs_dropped.fetch_add(1, Ordering::Relaxed) == 0
        {
            let _ = self.tx.send(Out::Wake);
        }
    }

    /// Sends the request built by `make(id)` and waits for the shim's reply; `None` on timeout.
    pub fn request(&self, make: impl FnOnce(u64) -> CoreMsg, timeout: Duration) -> Option<Reply> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap_or_else(PoisonError::into_inner).insert(id, tx);
        self.send(make(id));
        let reply = rx.recv_timeout(timeout).ok();
        self.pending.lock().unwrap_or_else(PoisonError::into_inner).remove(&id);
        reply
    }

    pub fn on_reply(&self, reply: Reply) {
        if let Some(tx) = self.pending.lock().unwrap_or_else(PoisonError::into_inner).remove(&reply.id) {
            let _ = tx.send(reply);
        }
    }

    /// Sends `Bye` after everything queued and waits until it is written.
    pub fn close(&self) {
        let _ = self.tx.send(Out::Close);
        if let Some(writer) = self.writer.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = writer.join();
        }
    }
}

fn write_loop(out: File, rx: Receiver<Out>, logs_dropped: &AtomicUsize) {
    let mut w = BufWriter::new(out);
    let mut broken = false;
    let mut write = |msg: &CoreMsg, body: &[u8]| {
        // a dead pipe means the shim is gone; keep draining so senders never block
        if !broken && write_frame(&mut w, msg, body).is_err() {
            broken = true;
        }
    };
    for out in rx {
        let dropped = logs_dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            let msg = format!("{dropped} log lines dropped");
            write(&CoreMsg::Log { level: LogLevel::Warning, msg, trace: None }, &[]);
        }
        match out {
            Out::Frame(msg, body) => write(&msg, &body),
            Out::Wake => {}
            Out::Close => {
                write(&CoreMsg::Bye, &[]);
                return;
            }
        }
    }
}
