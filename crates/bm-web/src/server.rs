//! Binding exactly the configured address (#736) and serving HTTP/1.1 with bounded headers, header-read timeouts
//! and graceful shutdown. "WebServer started." is logged only once the socket is listening (#568).

use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use hyper::body::Incoming;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use socket2::{Domain, Socket, Type};
use tokio::net::TcpListener;
use tower::ServiceExt;

use crate::WebError;
use crate::app::{PeerAddr, WebApp};

const MAX_HEADER_BUF: usize = 64 * 1024;
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(30);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

pub struct WebServer {
    listener: TcpListener,
    all_interfaces: bool,
}

impl WebServer {
    /// `WebserverConfig.resolveIp`: `""`, `0.0.0.0`, `::0` = all interfaces (dual-stack when possible),
    /// `#getLocalHost` = this host's name, else an address or host name (first result, IPv4 preferred like Java).
    pub async fn bind(ip: &str, port: i32) -> Result<Self, WebError> {
        let port = u16::try_from(port).map_err(|_| WebError::Port(port))?;
        if matches!(ip, "" | "0.0.0.0" | "::0") {
            let v6 = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port);
            let listener = match listen(v6, true) {
                Ok(l) => l,
                Err(_) => listen(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port), false)
                    .map_err(|source| WebError::Bind { addr: v6, source })?,
            };
            return Ok(Self { listener, all_interfaces: true });
        }
        let host = if ip == "#getLocalHost" {
            gethostname::gethostname().to_string_lossy().into_owned()
        } else {
            ip.to_owned()
        };
        let addr = resolve(&host, port).await.map_err(|source| WebError::Resolve { ip: ip.to_owned(), source })?;
        let listener = listen(addr, false).map_err(|source| WebError::Bind { addr, source })?;
        Ok(Self { listener, all_interfaces: false })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.listener.local_addr().expect("bound listener has an address")
    }

    /// Serves `app` until `shutdown` resolves, then stops accepting, ends SSE streams and waits up to 10 s for
    /// in-flight responses.
    pub async fn serve(self, app: WebApp, shutdown: impl Future<Output = ()> + Send) -> Result<(), WebError> {
        let addr = self.local_addr();
        if self.all_interfaces {
            tracing::info!("WebServer bound to all network interfaces on port {}", addr.port());
        } else {
            tracing::info!("WebServer bound to: /{addr}");
        }
        let token = app.shutdown_token();
        let router = app.into_router();
        let mut http = hyper::server::conn::http1::Builder::new();
        http.timer(TokioTimer::new())
            .header_read_timeout(HEADER_READ_TIMEOUT)
            .max_buf_size(MAX_HEADER_BUF)
            .title_case_headers(true)
            // Java sends no Date header
            .auto_date_header(false)
            .keep_alive(true);
        let graceful = GracefulShutdown::new();
        tokio::pin!(shutdown);
        tracing::info!("WebServer started.");
        loop {
            let (stream, peer) = tokio::select! {
                () = &mut shutdown => break,
                accepted = self.listener.accept() => match accepted {
                    Ok(a) => a,
                    Err(e) => {
                        tracing::debug!("Failed to accept connection: {e}");
                        // e.g. out of file descriptors: back off instead of spinning
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                },
            };
            let _ = stream.set_nodelay(true);
            let svc = router.clone().map_request(move |mut req: http::Request<Incoming>| {
                req.extensions_mut().insert(PeerAddr(peer));
                req
            });
            let conn = graceful.watch(http.serve_connection(TokioIo::new(stream), TowerToHyperService::new(svc)));
            tokio::spawn(async move {
                if let Err(e) = conn.await {
                    tracing::debug!("Exception in HttpConnection: {e}");
                }
            });
        }
        drop(self.listener);
        token.cancel();
        if tokio::time::timeout(DRAIN_TIMEOUT, graceful.shutdown()).await.is_err() {
            tracing::debug!("webserver shutdown: dropping connections still open after {DRAIN_TIMEOUT:?}");
        }
        Ok(())
    }
}

async fn resolve(host: &str, port: u16) -> io::Result<SocketAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, port));
    }
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port)).await?.collect();
    addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or(addrs.first())
        .copied()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no address for {host}")))
}

fn listen(addr: SocketAddr, dual_stack: bool) -> io::Result<TcpListener> {
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, None)?;
    if dual_stack {
        socket.set_only_v6(false)?;
    }
    // Windows SO_REUSEADDR would let another process steal the port; elsewhere it only skips TIME_WAIT
    #[cfg(not(windows))]
    socket.set_reuse_address(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024)?;
    socket.set_nonblocking(true)?;
    TcpListener::from_std(socket.into())
}
