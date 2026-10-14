//! File paths and `file:` URIs as the reference writes them
//! (`LspClientDescriptor.getFileUri` of IntelliJ Platform): every byte but
//! the unreserved characters and `/` percent-encoded, a Windows drive letter
//! lower-cased with its colon escaped (`file:///c%3A/p/a.py`, the form VS
//! Code sends and every server accepts), a UNC host as the authority.

use std::path::{Path, PathBuf};

use lsp_types::Url;

/// The URI of an absolute path.
pub(crate) fn from_path(path: &Path) -> Url {
    let uri = encode(&path.to_string_lossy(), cfg!(windows));
    // Always valid: only unreserved characters and escapes after `file://`.
    Url::parse(&uri).unwrap_or_else(|_| Url::parse("file:///").expect("a valid URI"))
}

/// The path of a `file:` URI; `None` for other schemes.
pub(crate) fn to_path(uri: &Url) -> Option<PathBuf> {
    decode(uri.as_str(), cfg!(windows)).map(PathBuf::from)
}

/// The path of a `file:` URI given as text.
pub(crate) fn str_to_path(uri: &str) -> Option<PathBuf> {
    decode(uri, cfg!(windows)).map(PathBuf::from)
}

fn encode(path: &str, windows: bool) -> String {
    let mut out = String::from("file://");
    let mut path = if windows {
        path.replace('\\', "/")
    } else {
        path.to_owned()
    };
    if windows {
        if let Some(rest) = path.strip_prefix("//?/") {
            // A verbatim path: \\?\C:\x
            path = rest.to_owned();
        } else if let Some(rest) = path.strip_prefix("//") {
            // UNC: \\host\share\x -> file://host/share/x
            let (host, rest) = rest.split_once('/').unwrap_or((rest, ""));
            percent_encode(host, &mut out);
            path = format!("/{rest}");
        }
        let bytes = path.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            out.push('/');
            out.push(bytes[0].to_ascii_lowercase() as char);
            out.push_str("%3A");
            path = path[2..].to_owned();
        }
    }
    if !path.starts_with('/') {
        out.push('/');
    }
    percent_encode(&path, &mut out);
    out
}

fn percent_encode(s: &str, out: &mut String) {
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
}

fn decode(uri: &str, windows: bool) -> Option<String> {
    let rest = strip_prefix_ignore_case(uri, "file:")?;
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let (authority, path) = match rest.strip_prefix("//") {
        Some(rest) => match rest.find('/') {
            Some(slash) => (&rest[..slash], &rest[slash..]),
            None => (rest, "/"),
        },
        None => ("", rest),
    };
    let path = percent_decode(path)?;
    let authority = percent_decode(authority)?;
    let local = authority.is_empty() || authority.eq_ignore_ascii_case("localhost");
    if !windows {
        return Some(if local {
            path
        } else {
            format!("//{authority}{path}")
        });
    }
    if !local {
        return Some(format!("\\\\{authority}{}", path.replace('/', "\\")));
    }
    // /c:/p/a.py -> C:\p\a.py
    let bytes = path.as_bytes();
    let path = if bytes.len() >= 3
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
    {
        let mut drive = path[1..].to_owned();
        drive[..1].make_ascii_uppercase();
        drive
    } else {
        path
    };
    Some(path.replace('/', "\\"))
}

fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
            && let Some(byte) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_paths() {
        assert_eq!(encode("/p/main.py", false), "file:///p/main.py");
        assert_eq!(
            encode("/My Project/a+b é#.py", false),
            "file:///My%20Project/a%2Bb%20%C3%A9%23.py"
        );
        assert_eq!(decode("file:///p/main.py", false).unwrap(), "/p/main.py");
        assert_eq!(
            decode("file:///My%20Project/a%2Bb%20%C3%A9%23.py", false).unwrap(),
            "/My Project/a+b é#.py"
        );
        // Servers may leave characters unescaped or name localhost.
        assert_eq!(decode("file:///p/a b.py", false).unwrap(), "/p/a b.py");
        assert_eq!(decode("FILE://localhost/p/x.py", false).unwrap(), "/p/x.py");
        assert_eq!(
            decode("file://host/share/x.py", false).unwrap(),
            "//host/share/x.py"
        );
        assert_eq!(decode("untitled:Untitled-1", false), None);
        assert_eq!(decode("file:///p/%ff.py", false), None);
        assert_eq!(decode("file:///p/100%.py", false).unwrap(), "/p/100%.py");
    }

    #[test]
    fn windows_paths() {
        assert_eq!(
            encode(r"C:\Users\dev\My App\main.py", true),
            "file:///c%3A/Users/dev/My%20App/main.py"
        );
        assert_eq!(
            encode(r"\\server\share\x.py", true),
            "file://server/share/x.py"
        );
        for uri in [
            "file:///c%3A/Users/dev/main.py",
            "file:///C:/Users/dev/main.py",
            "file:///c:/Users/dev/main.py",
        ] {
            assert_eq!(decode(uri, true).unwrap(), r"C:\Users\dev\main.py", "{uri}");
        }
        assert_eq!(
            decode("file://server/share/x.py", true).unwrap(),
            r"\\server\share\x.py"
        );
    }

    #[test]
    fn urls_round_trip() {
        let path = if cfg!(windows) {
            PathBuf::from(r"C:\p\dir with space\é.py")
        } else {
            PathBuf::from("/p/dir with space/é.py")
        };
        let uri = from_path(&path);
        assert_eq!(to_path(&uri), Some(path.clone()));
        assert_eq!(str_to_path(uri.as_str()), Some(path));
        if cfg!(windows) {
            assert!(uri.as_str().starts_with("file:///c%3A/"), "{uri}");
        } else {
            assert_eq!(uri.as_str(), "file:///p/dir%20with%20space/%C3%A9.py");
        }
    }
}
