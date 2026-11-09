# Rust-native shell

## Goals

1. Preserve the original repository's Hyprland configuration, setup scripts, assets, and license.
2. Replace the Quickshell UI/runtime with a native Rust Linux desktop shell.
3. Keep the runtime small and idle-friendly: one Wayland connection, one shared D-Bus session connection, bounded/reused SHM buffers, shared font caches, no per-widget threads, and redraw only when state changes.
4. Port behavior by subsystem and verify parity before retiring any QML implementation.

## Runtime layers

```text
Hyprland / Wayland compositor
          │
          ▼
 smithay-client-toolkit ── layer surfaces, outputs, input, frame events
          │
          ├── tiny-skia ────── CPU rendering into wl_shm buffers
          │
          ├── cosmic-text ──── font discovery, shaping, glyph cache
          │
          └── zbus ────────── NetworkManager, UPower, MPRIS, portals,
                               notifications, power and desktop services
          │
          ▼
 Rust shell state ── config, theme, panel models, IPC and actions
          │
          ▼
 Native panels / widgets / settings
```

The Hyprland compositor remains responsible for windows and composition. This project is a shell/client, not a replacement compositor.

## Workspace

- `crates/ii-shell`: first executable shell and reusable runtime modules.
- `crates/ii-shell/src/render.rs`: software rendering primitives.
- `crates/ii-shell/src/text.rs`: a shared cosmic-text font system and glyph cache.
- `crates/ii-shell/src/bus.rs`: shared D-Bus session connection boundary.
- `crates/ii-shell/src/config.rs`: versionable shell settings model.
- `crates/ii-shell/src/hyprland_ipc.rs`: bounded typed Hyprland command-socket queries.
- `crates/ii-shell/src/workspace_model.rs`: per-monitor workspace grouping and occupancy projection.
- `crates/ii-shell/src/hyprland_state.rs`: pure event-to-state reducer with targeted refresh hints.
- `crates/ii-shell/src/hyprland_runtime.rs`: event-batch coordinator and coalesced targeted command-socket refreshes.

## Build on Linux

The initial Wayland build uses the client-side Wayland protocol stack. On NixOS, use a development shell with Rust stable, pkg-config, Wayland and fontconfig development libraries. On other Linux distributions install the matching `wayland`, `fontconfig`, and Rust development packages.

```sh
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo check --manifest-path rust/Cargo.toml --workspace
cargo test --manifest-path rust/Cargo.toml --workspace
cargo run --manifest-path rust/Cargo.toml -p ii-shell
```

The first panel requires a Wayland compositor advertising `zwlr_layer_shell_v1` (Hyprland does). The Hyprland runtime coordinator is currently a separately tested component; the event fd is not yet registered with the main Wayland event loop. The project remains an early proof-of-life, not a complete desktop shell.

## Memory rules

- Reuse one `FontSystem` and one `SwashCache` for the process.
- Reuse the Wayland SHM pool; avoid a new full-screen heap pixmap for every paint.
- Paint directly into the SHM slot with tiny-skia, then convert its RGBA bytes to Wayland ARGB byte order in place.
- Keep rendering event-driven. No permanent animation timer or polling loop for unchanged state.
- Use one session-bus connection shared by service adapters. Avoid one D-Bus connection/thread per widget.
- Keep heavyweight features (AI, image/video processing, previews) out of the always-running base process; spawn them on demand.
- Measure RSS, allocations, wakeups, frame time, and buffer count on the same machine as the upstream shell. Do not claim a memory win without measurements.

## Porting rule

Existing QML remains the behavioral reference until the corresponding native subsystem has functional tests and an explicit acceptance checklist. See [PORTING_STATUS.md](../docs/PORTING_STATUS.md).
