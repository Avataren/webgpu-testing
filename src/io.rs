use std::path::Path;

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

// This helper is only used by our WebAssembly code paths but remains available on
// native builds to keep the API surface consistent, so silence the unused warning
// when it is compiled but not referenced.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
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
