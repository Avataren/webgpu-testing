//! Remote control of a running engine.
//!
//! A [`RemoteHub`] accepts JSON requests on a local TCP socket and executes them on the
//! main thread during the GPU stage of each frame, with full access to the active scene
//! and the renderer. The `engine-mcp` binary uses it to expose the engine to AI agents
//! over the Model Context Protocol, but any client can speak the protocol directly.
//!
//! # Protocol
//!
//! Newline-delimited JSON over `127.0.0.1:<port>`. Each request is one line:
//!
//! ```json
//! {"id": 1, "method": "scene.entities", "params": {"name_contains": "Cube"}}
//! ```
//!
//! and gets exactly one response line with the same id, either
//! `{"id": 1, "result": ...}` or `{"id": 1, "error": {"message": "..."}}`.
//! Requests on one connection are answered in order. `engine.methods` lists the
//! methods the running application understands.
//!
//! # Enabling
//!
//! The server starts when the application asks for it
//! ([`crate::RenderApplication::remote_control_port`]) or when the
//! `ENGINE_REMOTE_PORT` environment variable is set. `ENGINE_REMOTE_PORT=off`
//! disables it. It only listens on the loopback interface.
//!
//! # Execution model
//!
//! Requests are handled once per frame, after the application's GPU systems, in the
//! order they arrived. The application gets the first chance to handle each request
//! ([`crate::RenderApplication::remote_command`]); everything else falls through to the
//! built-in handlers in [`handlers`]. Screenshots and waits answer later frames.
//! Injected input is applied at the start of a frame, before scripts run, exactly like
//! window events.

mod capture;
pub mod handlers;
mod hub;
mod json;
pub mod log_tap;
pub mod preview;
mod profile;
#[cfg(not(target_arch = "wasm32"))]
mod server;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod validation;

pub use capture::{capture_surface, FrameCapture};
pub use handlers::{RemoteContext, RemoteResult, CORE_METHODS};
pub use hub::{fit_region, process_requests, CaptureOverride, RemoteHub, RemoteHubHandle};
pub use json::{entity_from_json, entity_id, transform_to_json};

/// Port the editor listens on when `ENGINE_REMOTE_PORT` is not set.
pub const DEFAULT_EDITOR_PORT: u16 = 47811;

/// Environment variable that enables the server on a port, or disables it with `off`.
pub const PORT_ENV_VAR: &str = "ENGINE_REMOTE_PORT";

/// Hooks an application implements to extend or observe remote control.
///
/// [`crate::RenderApplication`] forwards to this; plain [`crate::AppBuilder`] users can
/// pass [`NoRemoteApp`].
pub trait RemoteApp {
    /// Handle `method` before the built-in handlers. Return `None` to fall through.
    fn remote_command(
        &mut self,
        method: &str,
        params: &serde_json::Value,
        ctx: &mut RemoteContext<'_, '_>,
    ) -> Option<RemoteResult> {
        let _ = (method, params, ctx);
        None
    }

    /// Methods [`Self::remote_command`] handles, for `engine.methods`.
    fn remote_methods(&self) -> Vec<&'static str> {
        Vec::new()
    }

    /// Called once per frame after remote requests changed the scene.
    fn remote_scene_modified(&mut self, ctx: &mut crate::app::GpuUpdateContext<'_>) {
        let _ = ctx;
    }
}

/// A [`RemoteApp`] with no application-specific behaviour.
pub struct NoRemoteApp;

impl RemoteApp for NoRemoteApp {}

/// Resolve the port to listen on from `ENGINE_REMOTE_PORT` and the application's
/// default. Returns `None` when remote control is disabled.
pub fn resolve_port(app_default: Option<u16>) -> Option<u16> {
    match std::env::var(PORT_ENV_VAR) {
        Ok(value) => {
            let value = value.trim();
            if value.is_empty() {
                return app_default;
            }
            if matches!(value, "off" | "0" | "false" | "no") {
                return None;
            }
            match value.parse::<u16>() {
                Ok(port) => Some(port),
                Err(_) => {
                    log::warn!("Ignoring invalid {PORT_ENV_VAR}={value:?}");
                    app_default
                }
            }
        }
        Err(_) => app_default,
    }
}
