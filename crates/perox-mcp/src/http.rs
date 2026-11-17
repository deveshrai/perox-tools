//! A deliberately small HTTP/1.1 implementation: one request per connection,
//! `Content-Length` bodies only, hard limits on size and time. MCP clients send
//! exactly this, and a small surface is easy to keep correct.

use std::io::{self, Read, Write};

use std::net::TcpStream;
use std::time::Duration;

/// Maximum size of the request line plus headers.
pub const MAX_HEAD: usize = 16 * 1024;
/// Maximum body size.
pub const MAX_BODY: usize = 4 * 1024 * 1024;
/// A whole request must arrive within this time.
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    /// Header names are lowercased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Why a request could not be read. Each maps to an HTTP status.
#[derive(Debug)]
pub enum ReadError {
    /// Connection closed or timed out before a full request; no response is sent.
    Closed,
    /// Malformed request: 400.
    Bad(&'static str),
    /// Head or body too large: 413 (431 for headers is folded into this).
    TooLarge,
    /// No Content-Length on a request with a body: 411.
    LengthRequired,
}

pub fn read_request(stream: &mut TcpStream) -> Result<Request, ReadError> {
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .map_err(|_| ReadError::Closed)?;
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > MAX_HEAD {
            return Err(ReadError::TooLarge);
        }
        let n = stream.read(&mut chunk).map_err(|_| ReadError::Closed)?;
        if n == 0 {
            return Err(ReadError::Closed);
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    if head_end > MAX_HEAD {
        return Err(ReadError::TooLarge);
    }
    let head =
        std::str::from_utf8(&buf[..head_end]).map_err(|_| ReadError::Bad("head is not UTF-8"))?;
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split(' ');
    let (Some(method), Some(target), Some(version)) = (
        request_line.next(),
        request_line.next(),
        request_line.next(),
    ) else {
        return Err(ReadError::Bad("malformed request line"));
    };
    if !version.starts_with("HTTP/1.") {
        return Err(ReadError::Bad("unsupported HTTP version"));
    }
    let path = target.split('?').next().unwrap_or(target).to_owned();
    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or(ReadError::Bad("malformed header"))?;
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
    }
    let mut request = Request {
        method: method.to_owned(),
        path,
        headers,
        body: Vec::new(),
    };
    if request
        .header("transfer-encoding")
        .is_some_and(|v| !v.eq_ignore_ascii_case("identity"))
    {
        return Err(ReadError::LengthRequired);
    }
    let length = match request.header("content-length") {
        Some(v) => v
            .parse::<usize>()
            .map_err(|_| ReadError::Bad("bad Content-Length"))?,
        None if request.method == "POST" => return Err(ReadError::LengthRequired),
        None => 0,
    };
    if length > MAX_BODY {
        return Err(ReadError::TooLarge);
    }
    let mut body = buf.split_off(head_end + 4);
    body.truncate(length);
    while body.len() < length {
        let want = (length - body.len()).min(chunk.len());
        let n = stream
            .read(&mut chunk[..want])
            .map_err(|_| ReadError::Closed)?;
        if n == 0 {
            return Err(ReadError::Closed);
        }
        body.extend_from_slice(&chunk[..n]);
    }
    request.body = body;
    Ok(request)
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn empty(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn text(status: u16, text: &str) -> Self {
        Self {
            status,
            headers: vec![("Content-Type", "text/plain; charset=utf-8".into())],
            body: text.as_bytes().to_vec(),
        }
    }

    pub fn json(value: &serde_json::Value) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Type", "application/json".into())],
            body: value.to_string().into_bytes(),
        }
    }

    pub fn with_header(mut self, name: &'static str, value: String) -> Self {
        self.headers.push((name, value));
        self
    }

    pub fn write_to(&self, stream: &mut TcpStream) -> io::Result<()> {
        let mut out = format!("HTTP/1.1 {} {}\r\n", self.status, reason(self.status));
        for (name, value) in &self.headers {
            out.push_str(&format!("{name}: {value}\r\n"));
        }
        out.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            self.body.len()
        ));
        stream.write_all(out.as_bytes())?;
        stream.write_all(&self.body)?;
        stream.flush()
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Payload Too Large",
        503 => "Service Unavailable",
        _ => "",
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
