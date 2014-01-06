//! Loopback-first binary serve (TCP + WebSocket `/wire`).

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::Arc,
    thread,
};

use sha1::{Digest, Sha1};
use yydb::{
    wire::{self, dispatch, read_frame, write_frame, Frame},
    Connection,
};

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Validate bind policy before listening.
pub fn assert_bind_allowed(bind: &str, insecure_bind: bool) -> Result<(), String> {
    let (host, _port) = wire::split_bind(bind).map_err(|e| e.to_string())?;
    if wire::is_loopback_host(&host) {
        return Ok(());
    }
    if insecure_bind {
        return Ok(());
    }
    Err(format!(
        "refusing non-loopback bind `{bind}` — YYDB serve has no ACL; \
         use 127.0.0.1 / ::1, or pass --insecure-bind (not for public internet)"
    ))
}

/// Open the database and serve binary protocol v1 until the process exits.
pub fn run_serve(
    path: &Path,
    bind: &str,
    insecure_bind: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    assert_bind_allowed(bind, insecure_bind)?;
    if insecure_bind && {
        let (host, _) = wire::split_bind(bind)?;
        !wire::is_loopback_host(&host)
    } {
        eprintln!(
            "WARNING: yydb serve bound to non-loopback `{bind}` with --insecure-bind. \
             There is NO authentication. Do NOT expose this on the public internet."
        );
    }

    let conn = Arc::new(Connection::open(path)?);
    let listener = TcpListener::bind(bind)?;
    println!(
        "yydb serve {} on {bind} (wire v1: TCP + ws://{bind}/wire; no ACL)",
        path.display()
    );

    for stream in listener.incoming() {
        let stream = stream?;
        let conn = Arc::clone(&conn);
        thread::spawn(move || {
            if let Err(error) = handle_client(stream, &conn) {
                eprintln!("client error: {error}");
            }
        });
    }
    Ok(())
}

fn handle_client(stream: TcpStream, conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    let mut peek = [0_u8; 4];
    let n = stream.peek(&mut peek)?;
    if n >= 3 && &peek[..3] == b"GET" {
        handle_websocket(stream, conn)
    } else {
        handle_tcp(stream, conn)
    }
}

fn handle_tcp(mut stream: TcpStream, conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
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
        let response = dispatch(conn, &request);
        write_frame(&mut stream, &response)?;
    }
}

fn handle_websocket(
    mut stream: TcpStream,
    conn: &Connection,
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
    let path_ok = request
        .lines()
        .next()
        .map(|line| line.contains(" /wire"))
        .unwrap_or(false);
    if !path_ok {
        let body = b"use path /wire for YYDB binary websocket\n";
        let resp = format!(
            "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(resp.as_bytes())?;
        stream.write_all(body)?;
        return Ok(());
    }

    let key = request
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("Sec-WebSocket-Key") {
                Some(value.trim().to_owned())
            } else {
                None
            }
        })
        .ok_or("missing Sec-WebSocket-Key")?;

    let accept = {
        let mut hasher = Sha1::new();
        hasher.update(key.as_bytes());
        hasher.update(WS_GUID.as_bytes());
        base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            hasher.finalize(),
        )
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
        let response = dispatch(conn, &request);
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
    } else if len == 127 {
        let mut ext = [0_u8; 8];
        stream.read_exact(&mut ext)?;
        len = u64::from_be_bytes(ext);
    }

    let mut mask = [0_u8; 4];
    if masked {
        stream.read_exact(&mut mask)?;
    }

    if len > wire::MAX_BODY_LEN as u64 + wire::HEADER_LEN as u64 {
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
        0x8 => Ok(None), // close
        0x9 => {
            // ping → pong
            write_ws_frame(stream, 0xA, &payload)?;
            read_ws_binary(stream)
        }
        0x2 | 0x1 => Ok(Some(payload)), // binary (or text carrying binary frames)
        0x0 => Err("websocket continuation not supported".into()),
        other => Err(format!("unsupported websocket opcode {other}").into()),
    }
}

fn write_ws_binary(
    stream: &mut TcpStream,
    payload: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    write_ws_frame(stream, 0x2, payload)
}

fn write_ws_frame(
    stream: &mut TcpStream,
    opcode: u8,
    payload: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut header = Vec::with_capacity(10);
    header.push(0x80 | opcode);
    let len = payload.len();
    if len < 126 {
        header.push(len as u8);
    } else if len <= u16::MAX as usize {
        header.push(126);
        header.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        header.push(127);
        header.extend_from_slice(&(len as u64).to_be_bytes());
    }
    stream.write_all(&header)?;
    stream.write_all(payload)?;
    stream.flush()?;
    Ok(())
}

fn is_disconnect(error: &yydb::Error) -> bool {
    match error {
        yydb::Error::Io(io) => matches!(
            io.kind(),
            std::io::ErrorKind::UnexpectedEof
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::BrokenPipe
        ),
        _ => false,
    }
}
