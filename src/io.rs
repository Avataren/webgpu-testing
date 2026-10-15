use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PercentDecodeError {
    IncompleteEscape,
    InvalidEscape,
    InvalidUtf8,
}

impl std::fmt::Display for PercentDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PercentDecodeError::IncompleteEscape => {
                write!(f, "incomplete percent-escape sequence")
            }
            PercentDecodeError::InvalidEscape => {
                write!(f, "invalid percent-escape sequence")
            }
            PercentDecodeError::InvalidUtf8 => {
                write!(f, "percent-decoded string is not valid UTF-8")
            }
        }
    }
}

impl std::error::Error for PercentDecodeError {}

fn decode_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Percent-decodes a URI string, returning an owned string on success.
///
/// This helper is shared by the editor import pipeline and the runtime
/// loader so both sides agree on how glTF dependency paths are decoded.
pub fn percent_decode_uri(uri: &str) -> Result<String, PercentDecodeError> {
    let mut bytes = Vec::with_capacity(uri.len());
    let mut i = 0;
    let raw = uri.as_bytes();

    while i < raw.len() {
        match raw[i] {
            b'%' => {
                if i + 2 >= raw.len() {
                    return Err(PercentDecodeError::IncompleteEscape);
                }

                let hi = decode_hex(raw[i + 1]).ok_or(PercentDecodeError::InvalidEscape)?;
                let lo = decode_hex(raw[i + 2]).ok_or(PercentDecodeError::InvalidEscape)?;
                bytes.push((hi << 4) | lo);
                i += 3;
            }
            byte => {
                bytes.push(byte);
                i += 1;
            }
        }
    }

    String::from_utf8(bytes).map_err(|_| PercentDecodeError::InvalidUtf8)
}

#[cfg(target_arch = "wasm32")]
use std::path::PathBuf;

#[cfg(target_arch = "wasm32")]
use js_sys::Uint8Array;
#[cfg(target_arch = "wasm32")]
use web_sys::XmlHttpRequest;

/// Percent-encodes a file path for use inside a relative URL.
///
/// Everything outside the RFC 3986 unreserved set (and the `/` separators) becomes `%XX`,
/// so the server decodes the request back to the exact file name: `a%2e.png` is requested
/// as `a%252e.png`, and names with spaces, `#`, `?` or non-ASCII characters survive too.
/// This is the inverse of [`percent_decode_uri`].
pub fn percent_encode_path(path: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(path.len());
    for &byte in path.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                encoded.push(byte as char)
            }
            _ => {
                encoded.push('%');
                encoded.push(HEX[(byte >> 4) as usize] as char);
                encoded.push(HEX[(byte & 0x0f) as usize] as char);
            }
        }
    }
    encoded
}

/// The URL, relative to the page, that a web build requests for the file at `path`.
///
/// `path` is a file system path (a literal file name, never an already encoded URI), so each
/// byte that is not URL safe is encoded. Absolute `http(s)://` URLs pass through unchanged.
#[cfg(any(target_arch = "wasm32", test))]
fn web_path_to_url(path: &str) -> Result<String, String> {
    if path.starts_with("http://") || path.starts_with("https://") {
        return Ok(path.to_string());
    }

    let mut path_str = path.replace('\\', "/");

    while let Some(stripped) = path_str.strip_prefix("./") {
        path_str = stripped.to_string();
    }

    if let Some(stripped) = path_str.strip_prefix("web/") {
        path_str = stripped.to_string();
    }

    if path_str.starts_with('/') {
        path_str.remove(0);
    }

    if path_str.is_empty() {
        return Err("Cannot load empty web path".into());
    }

    Ok(percent_encode_path(&path_str))
}

#[cfg(target_arch = "wasm32")]
fn normalize_web_path(path: &Path) -> Result<String, String> {
    web_path_to_url(&path.to_string_lossy())
}

#[cfg(target_arch = "wasm32")]
fn fetch_bytes_sync(url: &str) -> Result<Vec<u8>, String> {
    let request = XmlHttpRequest::new()
        .map_err(|err| format!("Failed to create XMLHttpRequest: {:?}", err))?;
    request
        .open_with_async("GET", url, false)
        .map_err(|err| format!("Failed to open request for {}: {:?}", url, err))?;
    // Browsers no longer allow configuring a binary response type for synchronous
    // `XMLHttpRequest`s. Use an `x-user-defined` MIME override so we can recover the
    // original bytes from the returned text payload instead. This keeps the rest of
    // the loading pipeline synchronous, which matches the expectations of the
    // existing glTF loader code.
    request.override_mime_type("text/plain; charset=x-user-defined");
    request
        .send()
        .map_err(|err| format!("Failed to send request for {}: {:?}", url, err))?;

    let status = request
        .status()
        .map_err(|err| format!("Failed to get status for {}: {:?}", url, err))?;

    if status < 200 || status >= 400 {
        return Err(format!("HTTP {} when requesting {}", status, url));
    }

    let text = request
        .response_text()
        .map_err(|err| format!("Failed to get response body for {}: {:?}", url, err))?
        .ok_or_else(|| format!("No response body for {}", url))?;

    let bytes = text.chars().map(|ch| ch as u32 as u8).collect();
    Ok(bytes)
}

/// The URL, relative to the page, that a web build fetches `path` from.
#[cfg(target_arch = "wasm32")]
pub(crate) fn web_url(path: &Path) -> Result<String, String> {
    normalize_web_path(path)
}

#[cfg(target_arch = "wasm32")]
fn load_web_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let url = normalize_web_path(path)?;
    fetch_bytes_sync(&url)
}

#[cfg(target_arch = "wasm32")]
pub fn load_binary_from_str(path: &str) -> Result<Vec<u8>, String> {
    let path_buf = PathBuf::from(path);
    load_web_bytes(&path_buf)
}

pub(crate) fn load_binary(path: &Path) -> Result<Vec<u8>, String> {
    #[cfg(target_arch = "wasm32")]
    {
        load_web_bytes(path)
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read(path).map_err(|err| format!("Failed to read {:?}: {}", path, err))
    }
}

/// How a saved JSON file is laid out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum JsonLayout {
    /// As the file on disk already is (compact or indented); indented for new files.
    #[default]
    Preserve,
    Pretty,
    Compact,
}

/// Serialize `value` to `path` in `layout`, leaving the file untouched when it already
/// holds exactly that. Returns whether the file was written.
pub fn write_json<T: serde::Serialize + ?Sized>(
    path: &Path,
    value: &T,
    layout: JsonLayout,
) -> std::io::Result<bool> {
    let existing = std::fs::read(path).ok();
    let compact = match layout {
        JsonLayout::Compact => true,
        JsonLayout::Pretty => false,
        // serde_json only writes newlines between values when indenting.
        JsonLayout::Preserve => existing
            .as_deref()
            .map(<[u8]>::trim_ascii_end)
            .is_some_and(|bytes| !bytes.is_empty() && !bytes.contains(&b'\n')),
    };
    let json = if compact {
        serde_json::to_vec(value)
    } else {
        serde_json::to_vec_pretty(value)
    }
    .map_err(std::io::Error::other)?;
    if existing.as_deref() == Some(json.as_slice()) {
        return Ok(false);
    }
    std::fs::write(path, json)?;
    Ok(true)
}

/// Reads a UTF-8 text file: from disk natively, and on the web with a request
/// relative to the page (so a served project works like a project directory).
pub fn read_to_string(path: &Path) -> std::io::Result<String> {
    #[cfg(target_arch = "wasm32")]
    {
        let bytes = load_web_bytes(path).map_err(std::io::Error::other)?;
        String::from_utf8(bytes).map_err(std::io::Error::other)
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read_to_string(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_percent_signs_are_encoded_so_the_server_decodes_the_same_name() {
        // The B20 case: a file whose name really contains "%2e".
        assert_eq!(
            percent_encode_path("content/mat%2e.json"),
            "content/mat%252e.json"
        );
        assert_eq!(
            percent_decode_uri(&percent_encode_path("content/mat%2e.json")).unwrap(),
            "content/mat%2e.json"
        );
    }

    #[test]
    fn unreserved_characters_and_separators_are_kept() {
        assert_eq!(
            percent_encode_path("content/materials/Brass-01_v2~a.json"),
            "content/materials/Brass-01_v2~a.json"
        );
    }

    #[test]
    fn url_special_and_non_ascii_names_round_trip() {
        for name in [
            "a b.png",
            "hash#1.png",
            "what?.png",
            "100%.png",
            "plus+sign.png",
            "café/テクスチャ.png",
            "a%20b.png",
        ] {
            let encoded = percent_encode_path(name);
            assert!(
                encoded
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._~/%".contains(&b)),
                "{encoded}"
            );
            assert_eq!(percent_decode_uri(&encoded).unwrap(), name);
        }
    }

    #[test]
    fn web_urls_are_relative_normalized_and_encoded() {
        assert_eq!(
            web_path_to_url("./web/content/a b.png").unwrap(),
            "content/a%20b.png"
        );
        assert_eq!(
            web_path_to_url("/content/scenes\\main%2e.json").unwrap(),
            "content/scenes/main%252e.json"
        );
        assert_eq!(
            web_path_to_url("https://example.com/a%20b.png").unwrap(),
            "https://example.com/a%20b.png"
        );
        assert!(web_path_to_url("./").is_err());
    }
}
