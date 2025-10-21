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
#[cfg(target_arch = "wasm32")]
fn normalize_web_path(path: &Path) -> Result<String, String> {
    let mut path_str = path.to_string_lossy().replace('\\', "/");

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

    Ok(path_str)
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

#[cfg(target_arch = "wasm32")]
fn load_web_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let url = normalize_web_path(path)?;
    fetch_bytes_sync(&url)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn load_binary_from_str(path: &str) -> Result<Vec<u8>, String> {
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
