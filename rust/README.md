# Rust-native shell

## Goals

1. Preserve the original repository's Hyprland configuration, setup scripts, assets, and license.
2. Replace the Quickshell UI/runtime with a native Rust Linux desktop shell.
3. Keep the runtime small and idle-friendly: one Wayland connection, one shared D-Bus session connection, bounded/reused SHM buffers, shared font caches, one panel per output, no per-widget threads, one bounded Hyprland command worker, and redraw only when state changes.
4. Port behavior by subsystem and verify parity before retiring any QML implementation.

## Runtime layers

```text
Hyprland / Wayland compositor
          │
          ▼
 smithay-client-toolkit ── per-output layer surfaces, output lifecycle,
          │                 scale changes, input and frame events
          ├── tiny-skia ────── CPU rendering into reusable wl_shm buffers
          ├── cosmic-text ──── font discovery, shaping, glyph cache
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
- `crates/ii-shell/src/config.rs`: versionable shell settings model and read-only parsing of the upstream `bar.workspaces.shown` option from `~/.config/illogical-impulse/config.json`; `ii-shell` watches that file's parent directory with inotify and reloads the workspace count through its existing poll loop.
- `crates/ii-shell/src/output_model.rs`: checked logical-to-buffer geometry for output-bound panels.
- `crates/ii-shell/src/hyprland_ipc.rs`: bounded typed Hyprland command-socket queries and validated workspace dispatch.
- `crates/ii-shell/src/workspace_model.rs`: per-monitor workspace grouping, occupancy projection and logical-coordinate pointer hit testing.
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

The executable creates one top-panel layer surface per advertised Wayland output. A single poll(2) loop waits on the Wayland display and Hyprland broadcast socket together. One bounded worker receives workspace commands from Wayland pointer callbacks and performs short, fresh Hyprland command-socket requests outside the display loop; it is shared by all outputs and never spawned per widget. It uses EventQueue::prepare_read() before sleeping and dispatches Wayland callbacks after the display fd is readable. Hyprland refreshes are coalesced after event batches. Each panel requests a Wayland frame callback and queues at most one follow-up redraw; a queued paint is retried after frame callbacks and buffer-release dispatches, without a repaint timer. The event stream reconnects after startup failures and disconnects with bounded exponential backoff (250 ms to 30 s), using the existing poll loop rather than a timer thread. Command-socket refreshes are still synchronous and bounded by the IPC adapter timeout; asynchronous refresh I/O remains outstanding. This is an early-stage shell, not a complete desktop replacement.

## Memory rules

- Reuse one `FontSystem` and one `SwashCache` for the process.
- Keep two `wl_buffer` objects per output geometry; write only to a buffer whose release has made its slot writable.
- Drop/rebuild only the affected output's buffer pair when its logical geometry or integer scale changes.
- Paint directly into SHM slots with tiny-skia; avoid an intermediate full-frame heap pixmap.
- Keep rendering event-driven. No permanent animation timer or polling loop for unchanged state.
- Use one session-bus connection shared by service adapters. Avoid one D-Bus connection/thread per widget.
- Keep heavyweight features (AI, image/video processing, previews) out of the always-running base process; spawn them on demand.
- Keep buffers, caches, histories and notification queues bounded.
- Measure RSS, allocations, wakeups, frame time, and buffer count on the same machine as the upstream shell. Do not claim a memory win without measurements.

## Porting rule

Existing QML remains the behavioral reference until the corresponding native subsystem has functional tests and an explicit acceptance checklist. See [PORTING_STATUS.md](../docs/PORTING_STATUS.md).


## Config reload behavior

The workspace-slot watcher observes the existing `~/.config/illogical-impulse`
directory with inotify and is registered in the same blocking event loop as
Wayland and Hyprland. It reloads only `bar.workspaces.shown`; the upstream
configuration remains read-only. The parent directory must exist when the
shell starts. If it is created later, restart the shell to arm the watch.
