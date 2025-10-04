cargo build --lib --target wasm32-unknown-unknown && wasm-bindgen --target web --out-dir web/pkg target/wasm32-unknown-unknown/release/wgpu-cube.wasm
python3 -m http.server --directory web 8080