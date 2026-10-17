//! Frame captures through RenderDoc's in-application API (`renderdoc.capture`).
//!
//! The engine never loads RenderDoc itself; it only finds it when the process was
//! started under it: from qrenderdoc, `renderdoccmd capture`, or with
//! `ENABLE_VULKAN_RENDERDOC_CAPTURE=1`, which turns on RenderDoc's Vulkan layer
//! (`engine_launch {renderdoc: true}` sets it). Captures are written where RenderDoc's
//! capture path template says (`RENDERDOC_CAPFILE`), and the reply names the file.

use std::ffi::{c_void, CStr};
use std::path::PathBuf;
use std::sync::OnceLock;

use serde_json::json;

use super::hub::Reply;

/// Frames to wait for RenderDoc to write a capture before giving up.
const CAPTURE_TIMEOUT_FRAMES: u64 = 120;

/// RenderDoc's API table, from the copy of RenderDoc already loaded into the process.
pub struct RenderDoc {
    api: renderdoc_sys::RENDERDOC_API_1_4_1,
    _library: libloading::Library,
}

// The table holds plain function pointers into a library that stays loaded.
unsafe impl Send for RenderDoc {}
unsafe impl Sync for RenderDoc {}

static RENDERDOC: OnceLock<RenderDoc> = OnceLock::new();

impl RenderDoc {
    /// RenderDoc, if the process runs under it.
    pub fn get() -> Result<&'static RenderDoc, String> {
        if let Some(renderdoc) = RENDERDOC.get() {
            return Ok(renderdoc);
        }
        let renderdoc = Self::load()?;
        Ok(RENDERDOC.get_or_init(|| renderdoc))
    }

    fn load() -> Result<Self, String> {
        type GetApi = unsafe extern "C" fn(version: u32, out: *mut *mut c_void) -> i32;
        const NOT_LOADED: &str = "RenderDoc is not loaded into the engine; launch it under \
            RenderDoc (engine_launch with renderdoc: true, or from qrenderdoc)";

        // Only find a RenderDoc that is already loaded; never load one.
        #[cfg(unix)]
        let library: libloading::Library = {
            const RTLD_NOLOAD: i32 = 0x4;
            unsafe {
                libloading::os::unix::Library::open(
                    Some("librenderdoc.so"),
                    libloading::os::unix::RTLD_NOW | RTLD_NOLOAD,
                )
            }
            .map_err(|_| NOT_LOADED.to_string())?
            .into()
        };
        #[cfg(windows)]
        let library: libloading::Library =
            libloading::os::windows::Library::open_already_loaded("renderdoc.dll")
                .map_err(|_| NOT_LOADED.to_string())?
                .into();

        let get_api = unsafe { library.get::<GetApi>(b"RENDERDOC_GetAPI\0") }
            .map_err(|err| format!("RenderDoc has no RENDERDOC_GetAPI: {err}"))?;
        let mut api = std::ptr::null_mut();
        // 1.4.1 has everything used here and is what wgpu asks for too.
        if unsafe { get_api(10401, &mut api) } != 1 || api.is_null() {
            return Err("RenderDoc does not provide API version 1.4.1".into());
        }
        Ok(Self {
            api: unsafe { *api.cast::<renderdoc_sys::RENDERDOC_API_1_4_1>() },
            _library: library,
        })
    }

    /// Capture the next frame: from this frame's present to the next one's.
    fn trigger_capture(&self) {
        if let Some(trigger) = self.api.TriggerCapture {
            unsafe { trigger() };
        }
    }

    /// Captures written since the process started.
    fn capture_count(&self) -> u32 {
        self.api
            .GetNumCaptures
            .map_or(0, |count| unsafe { count() })
    }

    /// Path of capture `index` (0 is the first).
    fn capture_path(&self, index: u32) -> Option<PathBuf> {
        let get = self.api.GetCapture?;
        let mut length = 0u32;
        if unsafe {
            get(
                index,
                std::ptr::null_mut(),
                &mut length,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return None;
        }
        // The length includes the terminating NUL.
        let mut buffer = vec![0u8; length as usize + 1];
        if unsafe {
            get(
                index,
                buffer.as_mut_ptr().cast(),
                &mut length,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return None;
        }
        let path = CStr::from_bytes_until_nul(&buffer).ok()?;
        Some(PathBuf::from(path.to_string_lossy().into_owned()))
    }
}

/// A `renderdoc.capture` request, answered once RenderDoc has written the file.
pub(super) struct PendingCapture {
    reply: Reply,
    renderdoc: &'static RenderDoc,
    captures_before: u32,
    give_up_after: u64,
}

impl PendingCapture {
    /// Ask RenderDoc for a capture of the next frame. Answers `reply` with an error
    /// and returns `None` when the engine does not run under RenderDoc.
    pub(super) fn start(reply: Reply, frame: u64) -> Option<Self> {
        let renderdoc = match RenderDoc::get() {
            Ok(renderdoc) => renderdoc,
            Err(message) => {
                reply.send(Err(message));
                return None;
            }
        };
        let captures_before = renderdoc.capture_count();
        renderdoc.trigger_capture();
        Some(Self {
            reply,
            renderdoc,
            captures_before,
            give_up_after: frame + CAPTURE_TIMEOUT_FRAMES,
        })
    }

    /// Answer the request if the capture has been written (call after each present).
    /// Returns the request back while it is still waiting.
    pub(super) fn poll(self, frame: u64) -> Option<Self> {
        let count = self.renderdoc.capture_count();
        if count > self.captures_before {
            let result = self
                .renderdoc
                .capture_path(count - 1)
                .ok_or_else(|| "RenderDoc wrote a capture but did not report its path".into())
                .map(|path| {
                    let bytes = std::fs::metadata(&path).map_or(0, |meta| meta.len());
                    json!({
                        "path": path.display().to_string(),
                        "size_mb": (bytes as f64 / 1e5).round() / 10.0,
                        "frame": frame,
                        "note": "Open it with the renderdoc MCP server's open_capture or in qrenderdoc. Captures are large; delete them when done.",
                    })
                });
            self.reply.send(result);
            return None;
        }
        if frame > self.give_up_after {
            self.reply.send(Err(format!(
                "RenderDoc wrote no capture within {CAPTURE_TIMEOUT_FRAMES} frames; it captures \
                 between presents, so the window must be presenting (not minimized)"
            )));
            return None;
        }
        Some(self)
    }
}
