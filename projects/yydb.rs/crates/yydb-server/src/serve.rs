//! Loopback-first binary serve (TCP + WebSocket `/wire`).
//!
//! Security gates (Phase 1 — enforced at startup and per-connection):
//!
//! 1. Default bind MUST be loopback. Non-loopback bind is rejected *unless*
//!    an explicit auth bootstrap is configured (`YYDB_AUTH_TOKENS` with
//!    at least one named principal). `--insecure-bind` alone is never
//!    sufficient — the flag now accepts that explicit auth config is
//!    *not* being used, but the flag cannot bypass it on non-loopback.
//! 2. Max concurrent connections capped at `MAX_CONNECTIONS` (256).
//! 3. Handshake timeout `HANDSHAKE_TIMEOUT` (5s) — connections that don't
//!    complete Hello + Authenticate drop before accepting any other frame.
//!
//! Auth model (session registry):
//!
//! Every connection is unauthenticated until it sends a valid Authenticate
//! frame. The server looks up the bearer token in a shared bootstrap
//! registry (seeded from env), refuses anonymous principals from remote
//! peers, and creates an in-memory [`Session`] bound to the token with a
//! 1-hour TTL. Hello is the *only* frame accepted before auth; all
//! others get a structured auth error. Prepared statements are tracked
//! per-connection and drop when the connection closes.

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

use sha1::{Digest, Sha1};
use yydb_engine::Connection;
use yydb_protocol::{Frame, HEADER_LEN, MAX_BODY_LEN, read_frame, split_bind, write_frame};
use yydb_types::Error;

use crate::{
    auth::{AuthBootstrap, SessionRegistry},
    dispatch::{SessionState, dispatch, is_authenticated, is_loopback_host},
    host_wire::TcpWireHostAdapter,
};

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

// ─── Bounded semaphore (stable-Rust Condvar + Mutex) ──────────────────────

/// Stable-Rust semaphore: blocks until a permit is available.
#[derive(Clone)]
struct BoundedSemaphore {
    state: Arc<(Mutex<usize>, Condvar)>,
}

impl BoundedSemaphore {
    fn new(permits: usize) -> Self {
        Self { state: Arc::new((Mutex::new(permits), Condvar::new())) }
    }
    fn acquire(&self) -> SemaphorePermit {
        let (lock, cond) = &*self.state;
        let mut state = lock.lock().expect("semaphore poisoned");
        while *state == 0 {
            state = cond.wait(state).expect("semaphore condvar poisoned");
        }
        *state -= 1;
        SemaphorePermit { state: self.state.clone() }
    }
}

/// RAII permit — dropped when connection thread exits, releasing the slot.
/// Owns an Arc so it can cross thread boundaries without lifetimes.
struct SemaphorePermit {
    state: Arc<(Mutex<usize>, Condvar)>,
}

impl Drop for SemaphorePermit {
    fn drop(&mut self) {
        let (lock, cond) = &*self.state;
        let mut state = lock.lock().expect("semaphore poisoned");
        *state += 1;
        cond.notify_one();
    }
}

/// Max concurrent connections the server accepts.
pub const MAX_CONNECTIONS: usize = 256;

/// Handshake timeout — connection must complete Hello + Authenticate before
/// any further frame is accepted. Connections that miss this window drop.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Resolve the effective insecure-bind flag from CLI and env var.
/// Either source independently opts in. Internal helper for `run_serve`.
fn insecure_bind_opted_in(explicit: bool) -> bool {
    if explicit {
        return true;
    }
    std::env::var("YYDB_ALLOW_INSECURE_BIND").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false)
}

/// Whether this peer address is loopback (used to reject anonymous
/// principals from remote binds at lookup time).
fn peer_is_loopback(stream: &TcpStream) -> bool {
    match stream.peer_addr() {
        Ok(addr) => is_loopback_host(addr.ip().to_string().as_str()),
        Err(_) => false, // peer_addr shouldn't fail on an accepted stream; default to safe side
    }
}

/// Validate bind policy and bootstrap auth config before listening.
///
/// Loopback bind is always accepted (with a dev-warning if auth uses the
/// anonymous loopback default). Non-loopback bind requires at least one
/// *named* principal in the bootstrap registry — anonymous loopback-only
/// principals are rejected. Passing `insecure_bind=true` suppresses the
/// warning on loopback but does not bypass the non-loopback named-principal
/// check.
pub fn assert_bind_allowed(bind: &str, insecure_bind: bool, bootstrap: &AuthBootstrap) -> Result<(), String> {
    let (host, _port) = split_bind(bind).map_err(|e| e.to_string())?;
    let bind_is_loopback = is_loopback_host(&host);
    if !bind_is_loopback {
        if bootstrap.named_count() == 0 {
            return Err(Error::AuthConfigMissing {
                message: format!(
                    "refusing to bind to non-loopback address `{bind}` without explicit named auth — \
                     set YYDB_AUTH_TOKENS to configure at least one named principal before starting serve"
                ),
            }
            .to_string());
        }
        eprintln!(
            "⚠  yy-db listening on non-loopback bind `{bind}` with {} named principal(s) configured — \
             TLS is still NOT enforced at the wire layer",
            bootstrap.named_count()
        );
        eprintln!("⚠  This is ONLY safe on a trusted network or behind a TLS-terminating proxy");
        if insecure_bind {
            eprintln!("⚠  --insecure-bind passed — treated as explicit consent for non-TLS transport");
        }
        return Ok(());
    }
    // loopback bind: fine either way
    if insecure_bind {
        eprintln!("⚠  --insecure-bind passed on loopback bind — anonymous dev access allowed locally");
    }
    Ok(())
}

/// Open the database and serve binary protocol v1 until the process exits.
pub fn run_serve(path: &Path, bind: &str, insecure_bind: bool) -> Result<(), Box<dyn std::error::Error>> {
    let insecure_bind = insecure_bind_opted_in(insecure_bind);
    let (host, _port) = split_bind(bind)?;
    let bind_is_loopback = is_loopback_host(&host);

    let bootstrap = AuthBootstrap::from_env(bind_is_loopback)?;
    assert_bind_allowed(bind, insecure_bind, &bootstrap)?;

    let registry = Arc::new(SessionRegistry::new(&bootstrap));

    let conn = Connection::open(path)?;
    let listener = TcpListener::bind(bind)?;

    let limit = BoundedSemaphore::new(MAX_CONNECTIONS);
    let next_id = Arc::new(AtomicU64::new(1));

    println!(
        "yydb serve {} on {bind} (wire v1: TCP + ws://{bind}/wire; max={MAX_CONNECTIONS}, handshake={}s)",
        path.display(),
        HANDSHAKE_TIMEOUT.as_secs()
    );
    println!("Auth: Hello → Authenticate ({} principal(s); tokens from YYDB_AUTH_TOKENS) → queries", bootstrap.tokens.len());
    if bind_is_loopback && bootstrap.named_count() == 0 {
        println!("⚠  Loopback-only anonymous bootstrap — dev mode. Set YYDB_AUTH_TOKENS for explicit principals.");
    }

    for stream_res in listener.incoming() {
        let permit = limit.acquire();
        let connection_id = next_id.fetch_add(1, Ordering::Relaxed);
        let stream = stream_res?;
        let remote = !peer_is_loopback(&stream);
        let registry_ref = registry.clone();
        let session_state = Arc::new(Mutex::new(SessionState::new(connection_id, registry_ref, remote)));
        let conn_ref = conn.session();

        thread::spawn(move || {
            let _permit = permit; // RAII — drops when thread exits
            let _ = stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT));
            if let Err(error) = handle_client(stream, &conn_ref, session_state) {
                eprintln!("client error: {error}");
            }
        });
    }
    Ok(())
}

fn handle_client(
    stream: TcpStream,
    conn: &Connection,
    state: Arc<Mutex<SessionState>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut peek = [0_u8; 4];
    let n = stream.peek(&mut peek)?;
    if n >= 3 && &peek[..3] == b"GET" { handle_websocket(stream, conn, state) } else { handle_tcp(stream, conn, state) }
}

fn handle_tcp(
    mut stream: TcpStream,
    conn: &Connection,
    state: Arc<Mutex<SessionState>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let adapter = Arc::new(TcpWireHostAdapter::new(stream.try_clone()?));
    conn.set_host_adapter(adapter);
    loop {
        let request = match read_frame(&mut stream) {
            Ok(frame) => frame,
            Err(error) => {
                if is_disconnect(&error) {
                    return Ok(());
                }
                return Err(error.into());
            }
        };
        let response = dispatch(conn, &state.lock().unwrap(), &request);
        write_frame(&mut stream, &response)?;
        // After first Authenticate frame succeeds, clear handshake timeout
        if is_authenticated(&state.lock().unwrap()) {
            let _ = stream.set_read_timeout(None);
        }
    }
}

fn handle_websocket(
    mut stream: TcpStream,
    conn: &Connection,
    state: Arc<Mutex<SessionState>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > 64 * 1024 {
            return Err("websocket handshake too large".into());
        }
    }

    let request = String::from_utf8_lossy(&buf);
    let path_ok = request.lines().next().map(|line| line.contains(" /wire")).unwrap_or(false);
    if !path_ok {
        let body = b"use path /wire for YYDB binary websocket\n";
        let resp = format!("HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        stream.write_all(resp.as_bytes())?;
        stream.write_all(body)?;
        return Ok(());
    }

    let key = request
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("Sec-WebSocket-Key") { Some(value.trim().to_owned()) } else { None }
        })
        .ok_or("missing Sec-WebSocket-Key")?;

    let accept = {
        let mut hasher = Sha1::new();
        hasher.update(key.as_bytes());
        hasher.update(WS_GUID.as_bytes());
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, hasher.finalize())
    };

    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    stream.write_all(response.as_bytes())?;

    loop {
        let payload = match read_ws_binary(&mut stream)? {
            Some(bytes) => bytes,
            None => return Ok(()),
        };
        let request = Frame::decode(&payload)?;
        let response = dispatch(conn, &state.lock().unwrap(), &request);
        let encoded = response.encode()?;
        write_ws_binary(&mut stream, &encoded)?;
    }
}

fn read_ws_binary(stream: &mut TcpStream) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
    let mut header = [0_u8; 2];
    match stream.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let fin_opcode = header[0];
    let opcode = fin_opcode & 0x0f;
    let masked = (header[1] & 0x80) != 0;
    let mut len = (header[1] & 0x7f) as u64;

    if len == 126 {
        let mut ext = [0_u8; 2];
        stream.read_exact(&mut ext)?;
        len = u16::from_be_bytes(ext) as u64;
    }
    else if len == 127 {
        let mut ext = [0_u8; 8];
        stream.read_exact(&mut ext)?;
        len = u64::from_be_bytes(ext);
    }

    let mut mask = [0_u8; 4];
    if masked {
        stream.read_exact(&mut mask)?;
    }

    if len > MAX_BODY_LEN as u64 + HEADER_LEN as u64 {
        return Err("websocket frame too large".into());
    }

    let mut payload = vec![0_u8; len as usize];
    if len > 0 {
        stream.read_exact(&mut payload)?;
    }
    if masked {
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[i % 4];
        }
    }

    match opcode {
        0x8 => Ok(None),
        0x9 => {
            write_ws_frame(stream, 0xA, &payload)?;
            read_ws_binary(stream)
        }
        0x2 | 0x1 => Ok(Some(payload)),
        0x0 => Err("websocket continuation not supported".into()),
        other => Err(format!("unsupported websocket opcode {other}").into()),
    }
}

fn write_ws_binary(stream: &mut TcpStream, payload: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    write_ws_frame(stream, 0x2, payload)
}

fn write_ws_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let mut header = Vec::with_capacity(10);
    header.push(0x80 | opcode);
    let len = payload.len();
    if len < 126 {
        header.push(len as u8);
    }
    else if len <= u16::MAX as usize {
        header.push(126);
        header.extend_from_slice(&(len as u16).to_be_bytes());
    }
    else {
        header.push(127);
        header.extend_from_slice(&(len as u64).to_be_bytes());
    }
    stream.write_all(&header)?;
    stream.write_all(payload)?;
    stream.flush()?;
    Ok(())
}

fn is_disconnect(error: &Error) -> bool {
    match error {
        Error::Io(io) => matches!(
            io.kind(),
            std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
        ),
        _ => false,
    }
}
