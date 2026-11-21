# Rust-native shell

## Goals

1. Preserve the original repository's Hyprland configuration, setup scripts, assets, and license.
2. Replace the Quickshell UI/runtime with a native Rust Linux desktop shell.
3. Keep the runtime small and idle-friendly: one Wayland connection, one shared system-bus connection and one shared session-bus connection, bounded/reused SHM buffers, shared font caches, one panel per output, no per-widget threads, one bounded Hyprland command worker, one shared UPower monitor worker, and redraw only when state changes.
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
          └── zbus ────────── shared system bus (UPower) and session bus
                              (MPRIS, portals, notifications and desktop services)
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
- `crates/ii-shell/src/battery.rs`: pure UPower battery-state projection, upstream threshold and icon-level mapping, health/time-estimate normalization, and edge-case tests; it performs no D-Bus I/O or power actions.
- `crates/ii-shell/src/bus.rs`: shared system- and session-bus connection constructors; adapters receive shared connection handles rather than opening per-widget connections.
- `crates/ii-shell/src/upower.rs`: asynchronous UPower system-bus adapter using `GetDisplayDevice`, one `Properties.GetAll` snapshot, and a filtered `PropertiesChanged` stream; it creates no connection or worker thread.
- `crates/ii-shell/src/battery_runtime.rs`: one bounded UPower worker with a latest-snapshot mailbox and Linux eventfd wakeup, allowing the blocking Wayland/Hyprland loop to consume updates without D-Bus work or an unbounded queue.
- `crates/ii-shell/src/clock.rs`: local-time clock snapshot with minute/second precision and a poll-loop deadline aligned to the next clock boundary.
- `crates/ii-shell/src/window_presentation.rs`: focused-monitor active app/title projection and largest-workspace-window fallback for other outputs, with safe label truncation.
- `crates/ii-shell/src/window_inventory.rs`: pure largest-area client selection per workspace, preserving upstream tie behavior.
- `crates/ii-shell/src/icon_lookup.rs`: XDG desktop-entry and icon-theme lookup, AppSearch-style class guessing, bounded `index.theme` inheritance, path-safe directory resolution, bounded PNG decoding and a 128-entry lazy icon cache.
- `crates/ii-shell/src/config.rs`: versionable shell settings model and read-only parsing of upstream `bar.workspaces.shown` and `bar.workspaces.showAppIcons` options from `~/.config/illogical-impulse/config.json`; `ii-shell` watches that file's parent directory with inotify and reloads both settings through its existing poll loop.
- `crates/ii-shell/src/output_model.rs`: checked logical-to-buffer geometry for output-bound panels.
- `crates/ii-shell/src/hyprland_ipc.rs`: bounded typed Hyprland monitor/workspace/active-window/client queries and validated workspace dispatch.
- `crates/ii-shell/src/workspace_model.rs`: per-monitor workspace grouping, occupancy projection, largest-client projection for each visible slot, and logical-coordinate pointer hit testing.
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

The executable creates one top-panel layer surface per advertised Wayland output. A single poll(2) loop waits on the Wayland display and Hyprland broadcast socket together. One bounded worker receives workspace commands from Wayland pointer callbacks and performs short, fresh Hyprland command-socket requests outside the display loop; it is shared by all outputs and never spawned per widget. It uses EventQueue::prepare_read() before sleeping and dispatches Wayland callbacks after the display fd is readable. Hyprland refreshes are coalesced after event batches. Each panel requests a Wayland frame callback and queues at most one follow-up redraw; a queued paint is retried after frame callbacks and buffer-release dispatches, without a repaint timer. The event stream reconnects after startup failures and disconnects with bounded exponential backoff (250 ms to 30 s), using the existing poll loop rather than a timer thread. The same poll timeout wakes at the next minute boundary for the local clock; panels redraw only when the formatted time changes. The runtime snapshots `j/clients` alongside monitor/workspace/active-window data and coalesces client-list refreshes after window open/close/move/title/fullscreen events. The focused output shows the active Hyprland app class and title when its workspace has a client; other outputs show the largest-area client in their active workspace. Each visible workspace slot exposes its largest client; the bar resolves its class through the XDG desktop entries and configured icon theme, then draws cached PNG icons directly into the reusable SHM buffer when available. Command-socket refreshes are still synchronous and bounded by the IPC adapter timeout; asynchronous refresh I/O remains outstanding. This is an early-stage shell, not a complete desktop replacement.

## Memory rules

- Reuse one `FontSystem` and one `SwashCache` for the process.
- Keep two `wl_buffer` objects per output geometry; write only to a buffer whose release has made its slot writable.
- Drop/rebuild only the affected output's buffer pair when its logical geometry or integer scale changes.
- Paint directly into SHM slots with tiny-skia; avoid an intermediate full-frame heap pixmap.
- Keep rendering event-driven. No permanent animation timer or polling loop for unchanged state.
- Share one system-bus connection among system-service adapters (such as UPower) and one session-bus connection among session-service adapters (such as MPRIS and notifications). The current UPower bridge uses one bounded process-wide worker and eventfd mailbox; avoid one D-Bus connection or worker thread per widget.
- Keep heavyweight features (AI, image/video processing, previews) out of the always-running base process; spawn them on demand.
- Keep buffers, caches, histories and notification queues bounded. The workspace PNG cache is capped at 128 icons; icon files are limited to 4 MiB and 512 × 512 pixels before decoding.
- Measure RSS, allocations, wakeups, frame time, and buffer count on the same machine as the upstream shell. Do not claim a memory win without measurements.

## Porting rule

Existing QML remains the behavioral reference until the corresponding native subsystem has functional tests and an explicit acceptance checklist. See [PORTING_STATUS.md](../docs/PORTING_STATUS.md).


## Config reload behavior

The workspace-slot watcher observes the existing `~/.config/illogical-impulse`
directory with inotify and is registered in the same blocking event loop as
Wayland and Hyprland. It reloads `bar.workspaces.shown` and
`bar.workspaces.showAppIcons`; the upstream configuration remains read-only.
The parent directory must exist when the shell starts. If it is created later,
restart the shell to arm the watch.


## Clock behavior

The top bar now displays the local time using the upstream default zero-padded
12-hour `hh:mm` format. A shared clock snapshot also prepares the upstream
long-date string. The next minute boundary is folded into the existing
Wayland/Hyprland/config `poll(2)` wait; no timer thread or periodic polling
loop is added. The popup, user-configurable Qt locale formats, localized
weekday names, and optional verbose date display remain parity work.


## Workspace app icon behavior

The native workspace bar now reads the upstream `bar.workspaces.showAppIcons`
setting (default `true`) and reloads it through the existing inotify watcher.
At startup, the resolver indexes up to 2,048 desktop entries from
`XDG_DATA_HOME/applications` and `XDG_DATA_DIRS/applications`, including
desktop IDs and `StartupWMClass`. Its class-to-icon fallback follows the
useful lookup order in upstream `AppSearch.qml`: desktop ID, explicit
substitutions, Steam/Minecraft/polkit patterns, direct and normalized names,
then fuzzy desktop icon/name matches.

The resolver reads bounded freedesktop `index.theme` files and follows
the configured theme's declared `Inherits` chain in priority order, using its
`Directories` and `ScaledDirectories` rather than assuming every theme uses
the same directory layout. It rejects path traversal and inheritance cycles;
index files, theme count, inheritance depth, per-theme directories, and total
search directories all have explicit limits. Missing or unusable index files
fall back to the common icon directory layout, then hicolor/Adwaita/Papirus/
Breeze and pixmaps. PNG files are decoded lazily and cached; the cache holds at
most 128 entries, and input files are bounded to 4 MiB and 512 × 512 pixels
before decoding. Rendering composites the PNG directly into the existing panel
buffer and adds no thread or image service.
SVG and XPM paths can be found, but are not rasterized yet; those slots keep
their occupancy dot. The upstream monochrome/colorizer effects and automatic
icon-theme-change detection remain outstanding. Restart the shell after
changing the system icon theme.


## Battery state model

The battery projection in `crates/ii-shell/src/battery.rs` is a pure model
of the upstream `Battery.qml`, `BatteryIndicator.qml`, `BatteryPopup.qml`,
`Icons.qml`, and battery defaults in `common/Config.qml`. It converts
UPower's 0–100 percentage into the 0.0–1.0 fraction used by Quickshell,
derives charging/plugged-in and low/critical/suspend/full flags, applies the
same battery icon thresholds, and exposes positive time estimates only when
UPower reports them. Battery health is derived from full versus design energy
when both values are valid.

The model itself does not connect to D-Bus, send notifications or sounds, or
execute suspend actions. The separate `upower.rs` adapter accepts a shared
system-bus connection, resolves the aggregate device with `GetDisplayDevice`,
reads all device fields with one `Properties.GetAll` call, and installs a
filtered `PropertiesChanged` subscription before emitting its initial snapshot.
It refreshes the full immutable snapshot after each relevant signal, so
invalidated or omitted changed properties cannot leave the model partially
updated. The adapter itself adds no connection, worker thread, timer, or polling loop. The current zbus async-io backend still owns its internal executor; integrating or measuring that runtime overhead remains part of the D-Bus event-loop work.

The runtime now starts one named UPower monitor worker. It owns the
shared system-bus connection, retries connection/device failures with bounded
backoff (250 ms to 30 s), and publishes snapshots into a single
`Mutex<Option<BatterySnapshot>>` slot. A nonblocking Linux eventfd wakes the
existing Wayland/Hyprland `poll(2)` loop; multiple property changes coalesce
to the newest snapshot rather than building a queue. The UI thread only drains
the eventfd and copies the latest immutable snapshot, then redraws panels when
the value actually changes. The panel now draws an upstream-inspired rounded
clipped progress bar beside the clock, with a red fill below the low threshold
and a small charging bolt while charging below 100%. Percentage text is
overlaid through the shared cosmic-text font system with a contrasting
foreground. The indicator reserves a fixed logical width so title-space
reservation remains bounded across percentage changes.

This bridge uses one additional process-wide D-Bus worker thread, not one
thread per widget. The worker can be stopped and joined during shell shutdown.
The zbus async-io backend's internal executor overhead has not been measured
or removed. Battery thresholds still use upstream defaults rather than the
full user configuration parser.
Notifications, sounds, and automatic-suspend actions remain intentionally
unimplemented until their user-configurable behavior and safety checks are
ported. No live UPower service, NixOS/Hyprland compositor, or memory/wakeup
benchmark has been exercised in this hosted run.


## UPower private-bus integration test

On Linux, the UPower adapter test starts an isolated `dbus-daemon`, registers
mock `org.freedesktop.UPower` manager and display-device objects, and connects
the real adapter through a second D-Bus connection. It verifies the initial
`GetDisplayDevice`/`Properties.GetAll` snapshot, a `PropertiesChanged`
update, and a property-invalidation signal that requires a fresh full snapshot.
The test daemon is private to the test and is terminated on teardown; it does
not connect to or modify the developer's real system/session bus. The Rust
port GitHub Actions job installs `dbus-daemon` explicitly for this test.


## Native battery indicator rendering

The renderer draws the track and proportional fill directly into the existing
Wayland SHM buffer. Low, non-charging states use the upstream error color, and
a small fixed-vector bolt is drawn only while charging below 100%. The
percentage label uses the shared cosmic-text font system with a dark
foreground for contrast. No per-frame pixmap, icon resource, or additional
worker is created; updates remain driven by battery snapshot changes.

Pixel-level tests cover proportional fill, track color, low-battery color,
charging-bolt visibility, zero scale, unavailable devices, and short buffers.
Configurable battery thresholds, the popup, notifications, sounds, and guarded
suspend behavior remain separate parity work.
