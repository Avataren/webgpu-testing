---
name: smoke-test
description: Build and briefly run the editor, player and examples on the real GPU, flag crashes and wgpu/naga/egui errors, and optionally screenshot the engine window. Use after renderer, shader, egui, dependency or app-loop changes, or whenever "it compiles and tests pass" is not enough evidence.
---

# GPU smoke test

Unit tests don't create pipelines on a real device, so shader validation failures
(naga), wgpu validation errors and egui debug assertions only show up when the
binaries actually run. This skill runs them.

## Run it

```bash
.claude/skills/smoke-test/smoke.sh                     # default set: editor + 9 examples
.claude/skills/smoke-test/smoke.sh editor pbr          # specific targets
.claude/skills/smoke-test/smoke.sh --shots editor      # also screenshot the window
.claude/skills/smoke-test/smoke.sh --project <project-dir> player
```

The script:
- picks a toolchain that satisfies `rust-version` in `Cargo.toml` (cargo is not on
  PATH in agent shells). It uses `target/nightly` as the target dir when it falls back
  to nightly.
- builds with `--all-features`, then runs each target in the background (8 s, editor 12 s)
  with `RUST_LOG=warn`.
- marks a target OK only if it is still running at the end and the log has no
  `error|panic|invalid` lines. Vulkan loader, validation-layer and libdecor messages
  are environment noise and are filtered out.
- writes logs and screenshots to `target/smoke/`, and exits non-zero if anything failed.

`player` needs a project directory; without `--project` it exits immediately with
`Io(NotFound)`, which is expected.

## Screenshots

Read the PNGs to check that the frame actually rendered. A clean log can still mean a
black or garbled frame.

Only ever capture the engine window, by its title `wgpu hecs Renderer`; `smoke.sh`
does this for you. Never screenshot the root window or the whole desktop: the user's
other monitors show their private apps. If you take a screenshot by hand, use
`import -window "$(xdotool search --onlyvisible --name '^wgpu hecs Renderer$' | tail -1)" out.png`.

## When something fails

- `panicked at ... epaint ... TexturesDelta`: egui texture deltas were dropped without
  being applied (see `src/ui/egui_integration.rs`).
- naga parse or validation errors: check `ShaderBuilder` output, including the
  `enable wgpu_binding_array;` directive for bindless shaders.
- Reproduce with `RUST_LOG=info` or `RUST_BACKTRACE=1` by running the binary from
  `target/debug` (or `target/nightly/debug`) under `timeout`.

`docs/ARCHITECTURE.md` describes the frame flow and the gotchas these checks guard against.
