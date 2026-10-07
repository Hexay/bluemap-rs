//! The JVM shutdown hook BlueMapCLI relies on: Ctrl+C, SIGTERM/SIGHUP (Unix) and Ctrl+Break/console close
//! (Windows) start a graceful stop; a second signal exits immediately.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::watch;

type Hook = Box<dyn FnOnce() + Send>;

pub struct Shutdown {
    triggered: AtomicBool,
    hooks: Mutex<Vec<Hook>>,
    tx: watch::Sender<bool>,
}

impl Shutdown {
    /// Starts listening for signals on a background thread.
    pub fn install() -> Arc<Self> {
        let this =
            Arc::new(Self { triggered: AtomicBool::new(false), hooks: Mutex::default(), tx: watch::channel(false).0 });
        let listener = this.clone();
        let spawned = std::thread::Builder::new().name("bluemap-signals".into()).spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
            rt.block_on(async {
                signal().await;
                listener.trigger();
                signal().await;
                crate::log::warn("Forced shutdown.");
                std::process::exit(130);
            });
        });
        if let Err(e) = spawned {
            crate::log::warn(&format!("Failed to listen for shutdown signals: {e}"));
        }
        this
    }

    /// Runs on the first signal (or [`Shutdown::trigger`]), in registration order; right away if that already
    /// happened.
    pub fn on_trigger(&self, hook: impl FnOnce() + Send + 'static) {
        let mut hooks = self.hooks.lock().unwrap_or_else(PoisonError::into_inner);
        if self.is_triggered() {
            drop(hooks);
            hook();
        } else {
            hooks.push(Box::new(hook));
        }
    }

    pub fn trigger(&self) {
        let hooks = {
            let mut hooks = self.hooks.lock().unwrap_or_else(PoisonError::into_inner);
            if self.triggered.swap(true, Ordering::SeqCst) {
                return;
            }
            std::mem::take(&mut *hooks)
        };
        hooks.into_iter().for_each(|h| h());
        self.tx.send_replace(true);
    }

    pub fn is_triggered(&self) -> bool {
        self.triggered.load(Ordering::SeqCst)
    }

    /// Resolves once triggered.
    pub fn wait(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut rx = self.tx.subscribe();
        async move {
            let _ = rx.wait_for(|&t| t).await;
        }
    }
}

#[cfg(unix)]
async fn signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut term), Ok(mut hup)) = (signal(SignalKind::terminate()), signal(SignalKind::hangup())) else {
        let _ = tokio::signal::ctrl_c().await;
        return;
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
        _ = hup.recv() => {}
    }
}

#[cfg(windows)]
async fn signal() {
    use tokio::signal::windows::{ctrl_break, ctrl_close};
    let (Ok(mut brk), Ok(mut close)) = (ctrl_break(), ctrl_close()) else {
        let _ = tokio::signal::ctrl_c().await;
        return;
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = brk.recv() => {}
        _ = close.recv() => {}
    }
}
