//! Loopback HTTP reverse proxy for `*.localhost` style routes.
//!
//! The proxy matches the `Host` header, forwards the request to the service's
//! loopback port, and copies the response back. HTTP/1.1 upgrades (WebSocket)
//! are tunnelled in both directions after the switch, so Vite and similar
//! development servers keep their hot-reload connections.
//!
//! By default the listener binds to `127.0.0.1` only. It binds to all
//! interfaces only when the caller explicitly asks for it. Requests that
//! arrive from another machine are restricted to routes whose service set
//! `expose = true`; loopback clients can reach every route.

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::error::Error as StdError;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{HeaderMap, HeaderName, HeaderValue, CONNECTION, CONTENT_TYPE, HOST, UPGRADE};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

type BoxError = Box<dyn StdError + Send + Sync>;
type ProxyBody = BoxBody<Bytes, BoxError>;

/// How long a proxy request waits for the daemon to wake an idle service.
const WAKE_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// One hostname route.
#[derive(Clone, Copy, Debug)]
pub struct Route {
    /// Loopback port of the service.
    pub port: u16,
    /// Whether clients from other machines may use this route.
    pub expose: bool,
}

/// A request from the proxy for the daemon to wake an idle service.
pub struct WakeCall {
    /// Loopback port of the idle service.
    pub port: u16,
    /// Signalled once the service is healthy (`Ok`) or failed to wake (`Err`).
    pub reply: oneshot::Sender<std::result::Result<(), String>>,
}

/// Cooperative wake-up channel between the daemon and the proxy.
///
/// The daemon marks a service's port idle when it idle-stops the service and
/// clears the mark once the service is running again. Before connecting to an
/// upstream, the proxy asks for a wake-up when (and only when) the port is
/// marked idle, so requests for a running service pay no coordination cost.
#[derive(Clone)]
pub struct ProxyWake {
    idle: Arc<Mutex<BTreeSet<u16>>>,
    sender: mpsc::UnboundedSender<WakeCall>,
}

impl ProxyWake {
    /// Creates the channel and the daemon-side receiver.
    pub fn new() -> (Self, mpsc::UnboundedReceiver<WakeCall>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (
            Self {
                idle: Arc::new(Mutex::new(BTreeSet::new())),
                sender,
            },
            receiver,
        )
    }

    /// Marks `port` as idle; the next proxied request will ask for a wake-up.
    pub fn mark_idle(&self, port: u16) {
        self.idle
            .lock()
            .expect("idle set not poisoned")
            .insert(port);
    }

    /// Clears the idle mark for `port`.
    pub fn mark_running(&self, port: u16) {
        self.idle
            .lock()
            .expect("idle set not poisoned")
            .remove(&port);
    }

    /// Returns `Ok(())` once the service on `port` is running.
    ///
    /// Returns immediately when the port is not marked idle.
    pub async fn request(&self, port: u16) -> std::result::Result<(), String> {
        if !self
            .idle
            .lock()
            .expect("idle set not poisoned")
            .contains(&port)
        {
            return Ok(());
        }
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(WakeCall {
                port,
                reply: reply_tx,
            })
            .map_err(|_| "the Burrow daemon is no longer running".to_owned())?;
        match tokio::time::timeout(WAKE_REQUEST_TIMEOUT, reply_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("the Burrow daemon dropped the wake-up request".to_owned()),
            Err(_) => Err("the service did not wake up in time".to_owned()),
        }
    }
}

impl std::fmt::Debug for ProxyWake {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ProxyWake").finish_non_exhaustive()
    }
}

/// A running proxy. Dropping the handle stops accepting new connections.
pub struct ProxyHandle {
    port: u16,
    exposed: bool,
    task: JoinHandle<()>,
}

impl ProxyHandle {
    /// The port the proxy is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// True when the proxy is bound to all interfaces rather than loopback.
    pub fn exposed(&self) -> bool {
        self.exposed
    }
}

impl Drop for ProxyHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl std::fmt::Debug for ProxyHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProxyHandle")
            .field("port", &self.port)
            .field("exposed", &self.exposed)
            .finish_non_exhaustive()
    }
}

/// Starts the proxy with `routes` (hostname to loopback route).
///
/// `port` must already be reserved by the caller. Pass `0` to let the
/// operating system choose one, which is useful in tests. `wake` enables
/// idle wake-up for routed services; pass `None` when no daemon backs the
/// proxy.
pub async fn start(
    routes: BTreeMap<String, Route>,
    bind_all: bool,
    port: u16,
    wake: Option<ProxyWake>,
) -> std::io::Result<ProxyHandle> {
    let address = if bind_all {
        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    } else {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    };
    let listener = TcpListener::bind(SocketAddr::new(address, port)).await?;
    let port = listener.local_addr()?.port();

    let routes = Arc::new(
        routes
            .into_iter()
            .map(|(host, port)| (host.trim_end_matches('.').to_ascii_lowercase(), port))
            .collect::<BTreeMap<_, _>>(),
    );
    if routes.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the reverse proxy needs at least one route",
        ));
    }

    let task = tokio::spawn(accept_loop(listener, routes, wake));
    Ok(ProxyHandle {
        port,
        exposed: bind_all,
        task,
    })
}

/// Builds the browser URL for a route, omitting the default HTTP port.
pub fn url_for(host: &str, port: u16) -> String {
    if port == 80 {
        format!("http://{host}")
    } else {
        format!("http://{host}:{port}")
    }
}

async fn accept_loop(
    listener: TcpListener,
    routes: Arc<BTreeMap<String, Route>>,
    wake: Option<ProxyWake>,
) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(error) => {
                warn!(%error, "reverse proxy accept failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        stream.set_nodelay(true).ok();
        let routes = Arc::clone(&routes);
        let wake = wake.clone();
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let routes = Arc::clone(&routes);
                let wake = wake.clone();
                async move { Ok::<_, Infallible>(handle(request, routes, wake, peer).await) }
            });
            let connection = hyper::server::conn::http1::Builder::new()
                .preserve_header_case(true)
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades();
            if let Err(error) = connection.await {
                debug!(%peer, %error, "reverse proxy connection ended");
            }
        });
    }
}

async fn handle(
    mut request: Request<Incoming>,
    routes: Arc<BTreeMap<String, Route>>,
    wake: Option<ProxyWake>,
    peer: SocketAddr,
) -> Response<ProxyBody> {
    let Some(host) = request_host(request.headers()) else {
        return text_response(
            StatusCode::BAD_REQUEST,
            "Burrow proxy: this request has no Host header.\n",
        );
    };

    let Some(route) = routes.get(&host).copied() else {
        let mut known = routes.keys().cloned().collect::<Vec<_>>();
        known.sort();
        return text_response(
            StatusCode::NOT_FOUND,
            &format!(
                "Burrow proxy: no service is routed to `{host}`.\n\n\
                 Known routes:\n  {}\n\n\
                 Add `route = \"{host}\"` to a service in burrow.toml.\n",
                known.join("\n  ")
            ),
        );
    };
    if !peer.ip().is_loopback() && !route.expose {
        return text_response(
            StatusCode::NOT_FOUND,
            &format!(
                "Burrow proxy: `{host}` is not exposed to other machines.\n\n\
                 help: add `expose = true` to this service only if remote access is intended\n"
            ),
        );
    }
    let upstream_port = route.port;

    // An idle-stopped service is woken through the daemon before the proxy
    // connects, so the request waits for a healthy upstream instead of
    // failing on a closed port.
    if let Some(wake) = &wake {
        if let Err(message) = wake.request(upstream_port).await {
            return text_response(
                StatusCode::SERVICE_UNAVAILABLE,
                &format!(
                    "Burrow proxy: could not wake `{host}` on 127.0.0.1:{upstream_port}: {message}\n\
                     help: run `burrow status` and check the service logs\n"
                ),
            );
        }
    }

    let upgrade_requested = is_upgrade_request(request.headers());
    let path = request
        .uri()
        .path_and_query()
        .map(|value| value.as_str().to_owned())
        .unwrap_or_else(|| "/".to_owned());

    let mut builder = Request::builder()
        .method(request.method().clone())
        .uri(path);
    for (name, value) in request.headers() {
        if is_hop_by_hop(name, upgrade_requested) {
            continue;
        }
        builder = builder.header(name, value);
    }
    builder = builder
        .header("x-forwarded-host", host.as_str())
        .header("x-forwarded-for", peer.ip().to_string());

    // The upgrade future must be created before the request body moves into
    // the upstream client.
    let server_upgrade = if upgrade_requested {
        Some(hyper::upgrade::on(&mut request))
    } else {
        None
    };
    let upstream_request = match builder.body(request.into_body()) {
        Ok(request) => request,
        Err(error) => {
            return text_response(
                StatusCode::BAD_REQUEST,
                &format!("Burrow proxy: could not build the upstream request: {error}\n"),
            )
        }
    };

    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), upstream_port);
    let stream = match tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(address))
        .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => {
            return text_response(
                StatusCode::BAD_GATEWAY,
                &format!(
                    "Burrow proxy: `{host}` is not accepting connections on 127.0.0.1:{upstream_port}: {error}\n\
                     help: run `burrow status` and check the service logs\n"
                ),
            )
        }
        Err(_) => {
            return text_response(
                StatusCode::GATEWAY_TIMEOUT,
                &format!("Burrow proxy: timed out connecting to `{host}` on 127.0.0.1:{upstream_port}\n"),
            )
        }
    };
    stream.set_nodelay(true).ok();

    let (mut sender, connection) =
        match hyper::client::conn::http1::handshake(TokioIo::new(stream)).await {
            Ok(pair) => pair,
            Err(error) => {
                return text_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("Burrow proxy: could not speak HTTP to `{host}`: {error}\n"),
                )
            }
        };
    tokio::spawn(async move {
        if let Err(error) = connection.with_upgrades().await {
            debug!(%error, "upstream proxy connection ended");
        }
    });

    let mut upstream_response = match sender.send_request(upstream_request).await {
        Ok(response) => response,
        Err(error) => {
            return text_response(
                StatusCode::BAD_GATEWAY,
                &format!("Burrow proxy: `{host}` failed to answer: {error}\n"),
            )
        }
    };

    if upstream_response.status() == StatusCode::SWITCHING_PROTOCOLS {
        let client_upgrade = hyper::upgrade::on(&mut upstream_response);
        if let Some(server_upgrade) = server_upgrade {
            tokio::spawn(async move {
                let (server, client) = match tokio::try_join!(server_upgrade, client_upgrade) {
                    Ok(parts) => parts,
                    Err(error) => {
                        debug!(%error, "proxy upgrade failed");
                        return;
                    }
                };
                let mut server = TokioIo::new(server);
                let mut client = TokioIo::new(client);
                if let Err(error) = tokio::io::copy_bidirectional(&mut server, &mut client).await {
                    debug!(%error, "proxy tunnel ended");
                }
            });
        }

        let mut response = Response::new(empty_body());
        *response.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
        for (name, value) in upstream_response.headers() {
            response.headers_mut().insert(name, value.clone());
        }
        return response;
    }

    let (mut parts, body) = upstream_response.into_parts();
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "proxy-connection",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ] {
        parts.headers.remove(name);
    }
    let body = body
        .map_err(|error| -> BoxError { Box::new(error) })
        .boxed();
    Response::from_parts(parts, body)
}

fn request_host(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(HOST)?.to_str().ok()?.trim();
    if raw.is_empty() {
        return None;
    }
    let host = if let Some(rest) = raw.strip_prefix('[') {
        rest.split_once(']').map(|(host, _)| host).unwrap_or(rest)
    } else {
        raw.split_once(':').map(|(host, _)| host).unwrap_or(raw)
    };
    Some(host.trim_end_matches('.').to_ascii_lowercase())
}

fn is_upgrade_request(headers: &HeaderMap) -> bool {
    if !headers.contains_key(UPGRADE) {
        return false;
    }
    headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
}

fn is_hop_by_hop(name: &HeaderName, upgrade: bool) -> bool {
    match name.as_str() {
        "connection" | "upgrade" => !upgrade,
        "keep-alive"
        | "proxy-authenticate"
        | "proxy-authorization"
        | "proxy-connection"
        | "te"
        | "trailer"
        | "transfer-encoding" => true,
        _ => false,
    }
}

fn empty_body() -> ProxyBody {
    Full::new(Bytes::new())
        .map_err(|error| -> BoxError { match error {} })
        .boxed()
}

fn text_response(status: StatusCode, message: &str) -> Response<ProxyBody> {
    let body = Full::new(Bytes::from(message.to_owned()))
        .map_err(|error| -> BoxError { match error {} })
        .boxed();
    let mut response = Response::new(body);
    *response.status_mut() = status;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::{start, url_for};
    use std::collections::BTreeMap;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    async fn read_headers(stream: &mut TcpStream) -> String {
        let mut buffer = Vec::new();
        loop {
            let mut byte = [0_u8; 1];
            let read = stream.read(&mut byte).await.expect("read header");
            if read == 0 {
                break;
            }
            buffer.push(byte[0]);
            if buffer.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(&buffer).into_owned()
    }

    async fn spawn_upstream(response: &'static str, echo: bool) -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind upstream");
        let port = listener.local_addr().expect("address").port();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept upstream");
            let request = read_headers(&mut stream).await;
            assert!(
                request.to_ascii_lowercase().contains("host:"),
                "request had no host:\n{request}"
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write response");
            if echo {
                let mut buffer = [0_u8; 4];
                stream.read_exact(&mut buffer).await.expect("read tunnel");
                let upper = String::from_utf8_lossy(&buffer).to_ascii_uppercase();
                stream
                    .write_all(upper.as_bytes())
                    .await
                    .expect("write tunnel");
            }
            stream.shutdown().await.ok();
        });
        port
    }

    async fn read_response(stream: &mut TcpStream) -> String {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk))
                .await
                .expect("response timeout")
                .expect("read response");
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
            if buffer.windows(5).any(|window| window == b"hello") {
                break;
            }
        }
        String::from_utf8_lossy(&buffer).into_owned()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn routes_by_host_and_returns_a_404_for_unknown_hosts() {
        let upstream = spawn_upstream(
            "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
            false,
        )
        .await;
        let proxy = start(
            BTreeMap::from([(
                "shop.localhost".to_owned(),
                super::Route {
                    port: upstream,
                    expose: false,
                },
            )]),
            false,
            0,
            None,
        )
        .await
        .expect("start proxy");

        let mut stream = TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .expect("connect proxy");
        stream
            .write_all(
                b"GET /check?x=1 HTTP/1.1\r\nHost: Shop.LocalHost:1234\r\nConnection: close\r\n\r\n",
            )
            .await
            .expect("write request");
        let response = read_response(&mut stream).await;
        assert!(
            response.starts_with("HTTP/1.1 200"),
            "response:\n{response}"
        );
        assert!(response.ends_with("hello"), "response:\n{response}");

        let mut unknown = TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .expect("connect proxy");
        unknown
            .write_all(b"GET / HTTP/1.1\r\nHost: nope.localhost\r\nConnection: close\r\n\r\n")
            .await
            .expect("write request");
        let response = read_response(&mut unknown).await;
        assert!(
            response.starts_with("HTTP/1.1 404"),
            "response:\n{response}"
        );
        assert!(response.contains("shop.localhost"), "response:\n{response}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn proxies_a_websocket_upgrade_and_tunnels_bytes() {
        let upstream = spawn_upstream(
            "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n",
            true,
        )
        .await;
        let proxy = start(
            BTreeMap::from([(
                "shop.localhost".to_owned(),
                super::Route {
                    port: upstream,
                    expose: false,
                },
            )]),
            false,
            0,
            None,
        )
        .await
        .expect("start proxy");

        let mut stream = TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .expect("connect proxy");
        stream
            .write_all(
                b"GET /ws HTTP/1.1\r\nHost: shop.localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n",
            )
            .await
            .expect("write upgrade");
        let headers = read_headers(&mut stream).await;
        assert!(headers.starts_with("HTTP/1.1 101"), "headers:\n{headers}");
        assert!(
            headers.to_ascii_lowercase().contains("upgrade: websocket"),
            "headers:\n{headers}"
        );

        stream.write_all(b"ping").await.expect("write tunnel");
        let mut echoed = [0_u8; 4];
        tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut echoed))
            .await
            .expect("tunnel timeout")
            .expect("read tunnel");
        assert_eq!(&echoed, b"PING");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_idle_route_is_woken_before_the_proxy_connects() {
        // Reserve a port, then free it again so the route points at a port
        // nothing is listening on until the wake handler starts the upstream.
        let port = {
            let listener = TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("reserve port");
            listener.local_addr().expect("address").port()
        };
        let (wake, mut wake_rx) = super::ProxyWake::new();
        wake.mark_idle(port);
        let proxy = start(
            BTreeMap::from([(
                "shop.localhost".to_owned(),
                super::Route {
                    port,
                    expose: false,
                },
            )]),
            false,
            0,
            Some(wake),
        )
        .await
        .expect("start proxy");

        // The daemon side of the channel: start the upstream on the routed
        // port, then reply so the proxy connects.
        let daemon = tokio::spawn(async move {
            let call = wake_rx.recv().await.expect("wake call");
            assert_eq!(call.port, port);
            let listener = TcpListener::bind(("127.0.0.1", port))
                .await
                .expect("bind upstream on the routed port");
            tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.expect("accept upstream");
                let request = read_headers(&mut stream).await;
                assert!(request.contains("host:"), "request:\n{request}");
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
                    )
                    .await
                    .expect("write response");
            });
            let _ = call.reply.send(Ok(()));
        });

        let mut stream = TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .expect("connect proxy");
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: shop.localhost\r\nConnection: close\r\n\r\n")
            .await
            .expect("write request");
        let response = read_response(&mut stream).await;
        assert!(
            response.starts_with("HTTP/1.1 200"),
            "response:\n{response}"
        );
        daemon.await.expect("daemon task");
    }

    #[test]
    fn urls_omit_the_default_port() {
        assert_eq!(url_for("shop.localhost", 80), "http://shop.localhost");
        assert_eq!(
            url_for("shop.localhost", 43123),
            "http://shop.localhost:43123"
        );
    }
}
