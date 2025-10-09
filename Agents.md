# Agents.md

> Operating instructions for AI coding agents working in this repository.

## Mission

This repository contains Rust code targeting native and the web (WebGPU) using `wgpu` and WGSL. Your job is to:

- keep the code idiomatic and formatted,
- maintain a clean commit history,
- ensure everything builds and runs locally and for WASM,
- write focused tests and examples,
- propose safe refactors and small, reviewable PRs.

---

## Golden rules

1. **Never break the build.** Run all checks locally before committing.
2. **Prefer idiomatic Rust.** Use ownership/borrowing correctly, avoid unnecessary `clone`, prefer iterators over indexed loops, use `?` for error flow, avoid `unsafe` unless justified with comments and tests.
3. **Small steps.** Limit changes to a single concern per PR.
4. **Document intent.** Public APIs must have doc comments with examples.
5. **Keep shaders tidy.** WGSL should be validated, commented, and use pipeline-friendly layouts.

---

## Environment bootstrap

Perform this at the start of a session:

```bash
# 1) Toolchain
rustup update
rustup default stable
rustup target add wasm32-unknown-unknown

# 2) Formatters & linters
cargo install cargo-edit cargo-outdated --locked || true
rustup component add rustfmt clippy

# 3) WASM packagers (choose the one used by the repo; if both present prefer trunk)
cargo install trunk wasm-bindgen-cli wasm-opt || true

# 4) Optional dev tools
# wgsl_analyzer: WGSL LSP for shader authoring
# naga-cli: WGSL/spirv validation and translation
cargo install wgsl-analyzer naga-cli || true
```

If `build_web.sh` or other scripts exist in the repo, prefer them for consistency.

---

## Project scripts & common commands

> Adjust paths if the repo structure changes. Current top-level hints: `examples/`, `src/`, `tests/`, `web/`, `build_web.sh`.

### Core checks (must pass before any commit)

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo build --all
```

### WASM / Web build

Try in this order, using the project’s existing scripts if present:

```bash
# If a helper script exists
bash ./build_web.sh

# Otherwise, a common pattern:
cargo build --target wasm32-unknown-unknown --release
# If using wasm-bindgen directly:
wasm-bindgen --target web --no-typescript   --out-dir web/pkg   target/wasm32-unknown-unknown/release/*.wasm
# Optimize if wasm-opt is available:
wasm-opt -O3 -o web/pkg/app_opt.wasm web/pkg/*.wasm

# If using trunk (preferred when index.html is present under web/)
trunk build web/index.html --release
```

### Run examples (native)

```bash
# If examples exist under examples/
cargo run --example <name>
```

### Shader validation (recommended)

```bash
# Validate WGSL shaders (replace with actual paths)
naga validate --format wgsl path/to/shader.wgsl
```

---

## Coding standards

### Rust

- **Modules & crates**
  - Keep `lib.rs` small; move features to modules.
  - Public items must have `///` docs and examples runnable with `cargo test --doc`.
- **Errors**
  - Prefer `thiserror` or `anyhow` for ergonomic errors at boundaries.
  - Library code: typed errors (`thiserror`); app bins: `anyhow::Result`.
- **Allocation & perf**
  - Avoid needless `clone`. Use `Cow`, slices, and iterators.
  - Use `#[inline]` sparingly; benchmark first.
- **Concurrency**
  - Prefer `async` where it matches `wgpu` lifetimes and event loops (e.g., `winit`).
- **Testing**
  - Unit tests live next to the code. Examples doubled as doc tests.
  - For GPU paths, add CPU-side validation of inputs/outputs and feature-gated tests.

### WGSL

- Name buffers/bindings clearly (`Camera`, `Globals`, `Material`).
- Keep `struct` layouts stable; document each field and its alignment.
- Group uniforms by frequency of change; minimize bind group churn.
- Prefer `@group(N) @binding(M)` consistency across pipeline stages.

---

## Housekeeping tasks (safe to auto-run)

Perform these regularly as separate PRs:

1. **Dependency hygiene**

   ```bash
   cargo update
   cargo outdated || true
   ```

   - If updating `wgpu`/`naga`, scan changelogs for breaking changes. Adjust WGSL syntax and pipeline creation accordingly.

2. **Lint + format enforcement**

   - Add or update `rustfmt.toml` (use stable defaults).
   - Add CI step that fails on format or clippy warnings.

3. **Dead code & warnings**

   - Remove unused imports, feature-gate experimental code, delete stale files in `web/` or `examples/`.

4. **Docs pass**
   - Ensure `README.md` has run instructions for native and web targets.
   - Add `cargo doc --no-deps` check; fix broken intra-doc links.

---

## Pull request etiquette

- **Branch naming:** `feat/<area>-short`, `fix/<area>-short`, `chore/<topic>`.
- **Commit messages:** Conventional commits (`feat:`, `fix:`, `chore:`, `docs:`, `refactor:`).
- **PR size:** < 400 lines if possible. Include a concise description and test notes.
- **Checklist (PR description):**
  - [ ] `cargo fmt --all`
  - [ ] `cargo clippy -- -D warnings`
  - [ ] `cargo test --all`
  - [ ] Web build tested (WASM)
  - [ ] Shader(s) validated or exercised by an example

---

## Adding an example

When introducing new GPU functionality, add a small example under `examples/`:

```
examples/
  clear_screen.rs
  triangle.rs
```

Each example should:

- Parse basic CLI args if relevant.
- Create a window/event loop (e.g., `winit`) for native.
- For web, include a `web/index.html` or entry that `trunk` can use, and wire up the canvas.

---

## Testing guidance

- **Unit tests:** ordinary Rust tests for math, CPU-side resource setup, and serialization.
- **Doc tests:** show basic usage; ensure they compile.
- **Integration tests:** if present under `tests/`, prefer creating small headless tests that validate buffer contents after compute passes where feasible.
- **Web smoke tests:** ensure the WASM bundle loads and creates a device; log adapter details.

> Note: Headless WebGPU validation in CI varies by runner. Prefer native tests for logic and keep web validation as a smoke test or manual step unless a WebGPU runner is configured.

---

## CI suggestion (GitHub Actions)

_Add `.github/workflows/ci.yml` (agents may create if missing):_

```yaml
name: CI
on:
  push:
  pull_request:
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
          targets: wasm32-unknown-unknown
      - name: Cache cargo
        uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all --check
      - run: cargo clippy --all-targets --all-features -- -D warnings
      - run: cargo test --all --verbose
      - run: cargo build --all --release
```

(Optional) add a separate job for web build if the repo requires it.

---

## Safe refactor playbook

- Extract long functions into smaller, testable units.
- Replace manual loops with iterator adapters where it improves clarity.
- Introduce `From`/`TryFrom` for conversions.
- Use `#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]` where appropriate.
- For config/state objects, implement builders to avoid long parameter lists.

---

## When touching WGSL / pipelines

- Keep a single source of truth for vertex layouts and bind group layouts (Rust side).
- Add comments mapping Rust bind groups to WGSL `@group/@binding`.
- After changes, run **both** native and web builds to catch backend differences.
- Validate shaders with `naga` if available:
  ```bash
  naga validate --format wgsl path/to/shader.wgsl
  ```

---

## Error handling & logging

- Convert low-level errors into user-friendly messages at app boundaries.
- Prefer `tracing` crate for structured logs; add `RUST_LOG=info` examples to README.
- On web, surface initialization errors to the console and the page (e.g., a status `<div>`).

---

## Files the agent may create/update

- `README.md` sections for build/run instructions
- `.github/workflows/ci.yml`
- `rustfmt.toml`
- `clippy.toml` (only if needed for justified lints)
- Small examples under `examples/`
- Minimal `web/index.html` or `web/` assets to support `trunk` or `wasm-bindgen`
- This `Agents.md` file

Do **not** introduce large dependency changes or rewrite architecture without explicit direction.

---

## Definition of done (per task)

A change is complete when:

- All local checks pass,
- Code is idiomatic and documented,
- Examples/tests cover new behavior,
- The web build (if affected) still loads and initializes a device,
- The PR description states **what** changed and **why**.
