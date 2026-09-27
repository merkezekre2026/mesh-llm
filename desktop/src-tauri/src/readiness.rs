//! Detects whether a mesh-llm management API is answering on the local
//! console port, both for attaching to an already running instance and for
//! knowing when a freshly started sidecar is ready to show its console.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

const STATUS_PATH: &str = "/api/status";
const MAX_DRAIN_BYTES: usize = 1024 * 1024;

pub fn console_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/")
}

/// Returns true when `GET /api/status` on the loopback console port answers
/// with HTTP 200.
pub fn status_ok(port: u16, timeout: Duration) -> bool {
    fetch_status_code(port, timeout) == Some(200)
}

fn fetch_status_code(port: u16, timeout: Duration) -> Option<u16> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, timeout).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    let request = format!(
        "GET {STATUS_PATH} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).ok()?;
    let mut head = [0u8; 64];
    let read = stream.read(&mut head).ok()?;
    let code = parse_status_code(&String::from_utf8_lossy(&head[..read]));
    drain(&mut stream);
    code
}

/// Reads the rest of the response before closing, so the server sees an
/// orderly close rather than a connection reset.
fn drain(stream: &mut TcpStream) {
    let mut sink = [0u8; 4096];
    let mut remaining = MAX_DRAIN_BYTES;
    while remaining > 0 {
        match stream.read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(read) => remaining = remaining.saturating_sub(read),
        }
    }
}

/// Parses the status code out of an HTTP/1.x status line.
fn parse_status_code(response: &str) -> Option<u16> {
    let line = response.lines().next()?;
    let mut parts = line.split_whitespace();
    let version = parts.next()?;
    if !version.starts_with("HTTP/1.") {
        return None;
    }
    parts.next()?.parse().ok()
}

/// Delay before the next readiness probe: quick at first, then settling at
/// two seconds so a long first-run model download is not hammered.
pub fn probe_delay(attempt: u32) -> Duration {
    Duration::from_millis(250u64.saturating_mul(1u64 << attempt.min(3)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn parses_status_lines() {
        assert_eq!(parse_status_code("HTTP/1.1 200 OK\r\n"), Some(200));
        assert_eq!(parse_status_code("HTTP/1.0 503 Busy\r\n"), Some(503));
        assert_eq!(parse_status_code("SSH-2.0-OpenSSH\r\n"), None);
        assert_eq!(parse_status_code(""), None);
    }

    #[test]
    fn probe_delay_backs_off_and_caps() {
        assert_eq!(probe_delay(0), Duration::from_millis(250));
        assert_eq!(probe_delay(1), Duration::from_millis(500));
        assert_eq!(probe_delay(3), Duration::from_millis(2000));
        assert_eq!(probe_delay(40), Duration::from_millis(2000));
    }

    fn serve_once(response: &'static str) -> u16 {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 512];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(response.as_bytes());
            }
        });
        port
    }

    #[test]
    fn detects_a_running_management_api() {
        let port = serve_once("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}");
        assert!(status_ok(port, Duration::from_secs(2)));
    }

    #[test]
    fn rejects_non_200_and_closed_ports() {
        let port = serve_once("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        assert!(!status_ok(port, Duration::from_secs(2)));

        let closed = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let closed_port = closed.local_addr().unwrap().port();
        drop(closed);
        assert!(!status_ok(closed_port, Duration::from_millis(300)));
    }

    #[test]
    fn console_url_uses_loopback() {
        assert_eq!(console_url(3131), "http://127.0.0.1:3131/");
    }
}
