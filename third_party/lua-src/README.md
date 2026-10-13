# lua-src (vendored)

A copy of [lua-src 547.0.0](https://github.com/khvzak/lua-src-rs) (MIT, see `LICENSE`),
patched in through `[patch.crates-io]` in the root `Cargo.toml`. Only the Lua 5.4.7
sources are kept, since mlua is built with `lua54`.

The only change is a `wasm32-unknown-unknown` branch in `src/lib.rs`. Upstream panics
for that target because it has no C library, which broke every web build of the engine.
The branch:

- compiles Lua with clang against the minimal headers in `wasm/include/`;
- links `wasm/libc.c`, the slice of libc that Lua uses. Strings, ctype, printf (with
  `%a`), strtod, UTC time and locale are implemented there. stdio has no filesystem
  (`fopen` fails with ENOENT), and stdout/stderr go to the log;
- lowers `setjmp`/`longjmp` (Lua's error handling) to wasm exception handling with
  `-mllvm -wasm-enable-sjlj`. `wasm/sjlj.c` and `wasm/sjlj_tag.S` hold the runtime and the
  `__c_longjmp` tag.

The program that links Lua must provide `malloc`/`calloc`/`realloc`/`free` and the
`lua_wasm_*` functions declared at the top of `wasm/libc.c`. The engine does this in
`src/scripting/lua/wasm_libc.rs`. compiler-builtins provides `memcpy` and friends and
the math functions, except `frexp`.

Requirements for web builds: `clang` with the wasm32 backend, and a browser with wasm
exception handling (legacy `try`/`catch`; every major browser since 2022). wasm-bindgen
0.2.129 handles modules that use it. Older CLIs need `__instance_terminated` exported.

To update: copy a newer lua-src release over `src/` and `lua-5.4.x/`, then re-apply the
`wasm32-unknown-unknown` match arm.
