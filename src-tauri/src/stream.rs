//! The stream page (#55): the overlay's widgets as a page on this PC, for an OBS browser source.
//! Nothing is drawn over the game, so it works with exclusive fullscreen too.
//!
//! A small HTTP server on `127.0.0.1` only (never the network), answering only `GET`s whose `Host`
//! is `127.0.0.1` or `localhost` on its port, so a web page can't reach it through DNS rebinding.
//! It serves the overlay page and its assets from the app (`overlay_window::PAGE`) and the feed
//! (`overlay::feed`, as for the stream: its own widgets, never edit mode). Nothing it serves holds
//! the host token.

use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::match_log::Known;
use crate::overlay::{self, FeedRequest};
use crate::overlay_window::PAGE;
use crate::{config, Store};

#[derive(Default)]
pub struct StreamServer {
    running: Mutex<Option<Running>>,
    /// Why the page isn't served (its port is taken), for Settings.
    error: Mutex<Option<String>>,
}

struct Running {
    port: u16,
    stop: Arc<AtomicBool>,
}

impl StreamServer {
    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }
}

/// The page's address, for OBS.
pub fn url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/")
}

/// Starts, moves or stops the server to match the settings: call at startup and after every
/// change to them.
pub fn apply(app: &AppHandle) {
    let overlay = app.state::<Store>().get().overlay;
    let wanted = overlay.stream_on().then(|| overlay.stream_port());
    let server = app.state::<StreamServer>();
    let mut running = server.running.lock().unwrap();
    if running.as_ref().map(|r| r.port) == wanted {
        return;
    }
    if let Some(old) = running.take() {
        old.stop.store(true, Ordering::Relaxed);
        log::info!("Stream page stopped");
    }
    *server.error.lock().unwrap() = None;
    let Some(port) = wanted else { return };
    let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .and_then(|l| l.set_nonblocking(true).map(|_| l))
    {
        Ok(listener) => listener,
        Err(e) => {
            log::warn!("Couldn't serve the stream page on port {port}: {e}");
            *server.error.lock().unwrap() =
                Some(format!("Port {port} is taken ({e}). Pick another port"));
            return;
        }
    };
    log::info!("Stream page on {}", url(port));
    let stop = Arc::new(AtomicBool::new(false));
    *running = Some(Running {
        port,
        stop: stop.clone(),
    });
    let app = app.clone();
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let app = app.clone();
                    std::thread::spawn(move || {
                        if let Err(e) = serve(&app, stream, port) {
                            log::debug!("Stream page: {e}");
                        }
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(config::STREAM_ACCEPT_POLL_MS))
                }
                Err(e) => {
                    log::debug!("Stream page: {e}");
                    std::thread::sleep(Duration::from_millis(config::STREAM_ACCEPT_POLL_MS));
                }
            }
        }
    });
}

/// What a request asks for.
#[derive(Debug, PartialEq)]
pub enum Route {
    Page,
    Feed(FeedRequest),
    Asset(String),
}

/// A request turned down, with its status.
#[derive(Debug, PartialEq)]
pub struct Refused(pub u16, pub &'static str);

/// Reads a request's head: only a `GET`, with a `Host` of this server.
pub fn route(head: &str, port: u16) -> Result<Route, Refused> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split(' ');
    let (method, target) = (first.next().unwrap_or_default(), first.next());
    if method != "GET" {
        return Err(Refused(405, "Method Not Allowed"));
    }
    let host = lines
        .take_while(|l| !l.is_empty())
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("host")
                .then(|| value.trim())
        })
        .unwrap_or_default();
    if !host_allowed(host, port) {
        return Err(Refused(403, "Forbidden"));
    }
    let target = target.unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match path {
        "/" => Ok(Route::Page),
        "/feed" => Ok(Route::Feed(feed_request(query))),
        p if p.starts_with("/assets/") && !p.contains("..") && !p.contains('\\') => {
            Ok(Route::Asset(p.to_string()))
        }
        _ => Err(Refused(404, "Not Found")),
    }
}

/// Only this server's own names: a page on another site, rebound to `127.0.0.1`, sends its own.
fn host_allowed(host: &str, port: u16) -> bool {
    [format!("127.0.0.1:{port}"), format!("localhost:{port}")]
        .iter()
        .any(|h| h.eq_ignore_ascii_case(host))
}

/// The feed request in a query: `file` and `size` (the log the page has), each `name`, `matchKey`.
fn feed_request(query: &str) -> FeedRequest {
    let mut request = FeedRequest::default();
    let (mut file, mut size) = (None, None);
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "file" => file = Some(value.into_owned()),
            "size" => size = value.parse().ok(),
            "name" => request.names.push(value.into_owned()),
            "matchKey" => request.match_key = Some(value.into_owned()),
            _ => {}
        }
    }
    request.names.truncate(config::LOBBY_PLAYERS_MAX as usize);
    if let (Some(file), Some(size)) = (file, size) {
        request.known = Some(Known { file, size });
    }
    request
}

fn serve(app: &AppHandle, mut stream: TcpStream, port: u16) -> io::Result<()> {
    // Accepted sockets inherit the listener's non-blocking mode on Windows.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(config::STREAM_READ_TIMEOUT_SECS)))?;
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        if head.len() > config::STREAM_REQUEST_MAX_BYTES {
            return respond(
                &mut stream,
                431,
                "Request Header Fields Too Large",
                "text/plain",
                b"",
            );
        }
        let n = stream.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        head.extend_from_slice(&buf[..n]);
    }
    let head = String::from_utf8_lossy(&head);
    match route(&head, port) {
        Err(Refused(status, reason)) => respond(&mut stream, status, reason, "text/plain", b""),
        Ok(Route::Feed(request)) => {
            let feed = overlay::feed(app, &request, true);
            let body = serde_json::to_vec(&feed).unwrap_or_default();
            respond(&mut stream, 200, "OK", "application/json", &body)
        }
        Ok(Route::Page) => asset(app, &mut stream, &format!("/{PAGE}"), port),
        Ok(Route::Asset(path)) => asset(app, &mut stream, &path, port),
    }
}

fn asset(app: &AppHandle, stream: &mut TcpStream, path: &str, port: u16) -> io::Result<()> {
    match app.asset_resolver().get(path.to_string()) {
        Some(asset) => respond(stream, 200, "OK", &asset.mime_type, &asset.bytes),
        // `tauri dev` serves the page from Vite, not the app: send OBS there, with this server as
        // its feed.
        None if cfg!(debug_assertions) && path == format!("/{PAGE}") => {
            let location = format!("http://localhost:1420/{PAGE}?feed={}", url(port));
            write!(
                stream,
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
        }
        None => respond(stream, 404, "Not Found", "text/plain", b""),
    }
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    // `tauri dev`'s page is on Vite's port: it may read the feed. A built app serves both itself.
    let cors = if cfg!(debug_assertions) {
        "Access-Control-Allow-Origin: http://localhost:1420\r\n"
    } else {
        ""
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n{cors}Connection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(target: &str, host: &str) -> String {
        format!("GET {target} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: OBS\r\n\r\n")
    }

    #[test]
    fn serves_the_page_the_feed_and_assets() {
        assert_eq!(route(&get("/", "127.0.0.1:47623"), 47623), Ok(Route::Page));
        assert_eq!(
            route(&get("/assets/overlay-1a2b.js", "localhost:47623"), 47623),
            Ok(Route::Asset("/assets/overlay-1a2b.js".into()))
        );
        let Ok(Route::Feed(request)) = route(
            &get(
                "/feed?file=Log-2026-10-06-20-00-00.txt&size=1200&name=Kenzo&name=A%2CB%7C&matchKey=482913507226",
                "127.0.0.1:47623",
            ),
            47623,
        ) else {
            panic!("not the feed");
        };
        assert_eq!(request.names, ["Kenzo", "A,B|"]);
        assert_eq!(request.match_key.as_deref(), Some("482913507226"));
        let known = request.known.unwrap();
        assert_eq!(
            (known.file.as_str(), known.size),
            ("Log-2026-10-06-20-00-00.txt", 1200)
        );
        // Without both, the page has no log yet.
        let Ok(Route::Feed(request)) = route(&get("/feed?size=12", "127.0.0.1:47623"), 47623)
        else {
            panic!("not the feed");
        };
        assert!(request.known.is_none());
    }

    #[test]
    fn turns_down_other_hosts_methods_and_paths() {
        // DNS rebinding: a site's page asking 127.0.0.1 under its own name.
        assert_eq!(
            route(&get("/feed", "evil.example:47623"), 47623),
            Err(Refused(403, "Forbidden"))
        );
        assert_eq!(
            route(&get("/feed", "127.0.0.1:80"), 47623),
            Err(Refused(403, "Forbidden"))
        );
        assert_eq!(
            route("GET /feed HTTP/1.1\r\n\r\n", 47623),
            Err(Refused(403, "Forbidden"))
        );
        assert_eq!(
            route(
                "POST /feed HTTP/1.1\r\nHost: 127.0.0.1:47623\r\n\r\n",
                47623
            ),
            Err(Refused(405, "Method Not Allowed"))
        );
        for path in ["/settings.json", "/assets/../index.html", "/assets\\x"] {
            assert_eq!(
                route(&get(path, "127.0.0.1:47623"), 47623),
                Err(Refused(404, "Not Found")),
                "{path}"
            );
        }
    }

    #[test]
    fn answers_on_its_port() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            respond(&mut stream, 404, "Not Found", "text/plain", b"nope").unwrap();
        });
        let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        let mut answer = String::new();
        client.read_to_string(&mut answer).unwrap();
        assert!(answer.starts_with("HTTP/1.1 404 Not Found\r\n"));
        assert!(answer.contains("Content-Length: 4\r\n") && answer.ends_with("\r\n\r\nnope"));
    }
}
