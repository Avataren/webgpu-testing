# egui-winit 0.36.2 (vendored)

Copy of [egui-winit 0.36.2](https://github.com/emilk/egui) (MIT OR Apache-2.0), patched in
through `[patch.crates-io]` in the root `Cargo.toml`.

The only change is in `src/dropped_file.rs`: egui's `DroppedFile` trait has `bytes_async`
instead of `bytes` on wasm32, and upstream 0.36.2 does not implement it, so the crate does
not compile for the web. The patch gates `bytes` to native and adds a `bytes_async` that
returns an error (winit reports no dropped files on the web).

Drop this copy once an egui-winit release compiles for wasm32.
