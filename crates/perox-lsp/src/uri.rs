//! `file://` URIs ↔ paths, with percent-encoding.

use std::path::{Path, PathBuf};

pub fn from_path(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !s.starts_with('/') {
        out.push('/'); // Windows: file:///C:/...
    }
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(rest.len());
    let raw = rest.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 2 < raw.len() {
            let hex = std::str::from_utf8(raw.get(i + 1..i + 3)?).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    let s = String::from_utf8(bytes).ok()?;
    // file:///C:/x → C:/x on Windows.
    let s = if s.len() > 2 && s.as_bytes()[0] == b'/' && s.as_bytes()[2] == b':' {
        s[1..].to_owned()
    } else {
        s
    };
    Some(PathBuf::from(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let p = Path::new("/Users/me/My Project/src/main.rs");
        let u = from_path(p);
        assert_eq!(u, "file:///Users/me/My%20Project/src/main.rs");
        assert_eq!(to_path(&u).unwrap(), p);
        assert_eq!(
            to_path("file:///C:/x/y.rs").unwrap(),
            PathBuf::from("C:/x/y.rs")
        );
    }
}
