# Rust port status

**Branch:** `rust-port/phase-0`  
**Upstream:** `end-4/dots-hyprland`  
**Reference revision at port start:** `2f2a376fdb2ad738bf55f693dd0b474b64bb0ab8`  
**Latest upstream inspected (2026-11-10):** `cb1c428bf841dabccff1b126904047c28e89ba8c`

## What is being ported

The upstream repository is a full Hyprland desktop configuration plus a large Quickshell/QtQuick application. The current Quickshell tree includes the shell root, two panel families, common UI components, services, settings, background widgets, bar, dock, lock UI, media controls, notifications, OSD, on-screen keyboard, and many helper scripts/assets.

This Rust project replaces the graphical shell/runtime incrementally. It does not replace Hyprland itself, and it does not assume that every upstream script or configuration file should be rewritten in Rust. Existing Hyprland configs, themes, icons, scripts, and setup files stay available as the compatibility base.

## Phase 0 — workspace and rendering spike

- [x] Fork upstream into the user's GitHub account.
- [x] Create isolated `rust-port/phase-0` branch.
- [x] Add Rust workspace with pinned initial UI/runtime crate versions.
- [x] Add a Wayland layer-shell panel using smithay-client-toolkit.
- [x] Render into the Wayland SHM slot using tiny-skia, avoiding an intermediate full-frame pixmap.
- [x] Add cosmic-text system-font shaping/rasterization using one shared font system/cache.
- [x] Add a shared session-bus connection boundary with zbus.
- [x] Add JSON config model and basic unit tests.
- [x] Add GitHub Actions checks for formatting, compilation, tests, and Clippy.
- [ ] Run the first build/test on a real NixOS/Hyprland machine. GitHub source edits cannot prove compositor compatibility or runtime RSS.
- [x] Add a bounded Hyprland command-socket adapter with typed monitor/workspace/window queries and validated workspace dispatch, plus parser/path-validation tests.

## Phase 1 — platform/runtime

- [x] Create one output-bound top-panel layer surface per advertised `wl_output`, keyed by the output global id; remove its surface on output destruction.
- [x] Track output logical width and integer scale-factor changes, with checked logical-to-buffer geometry conversion.
- [x] Allocate two SHM buffers per output geometry and retain/reuse each `wl_buffer` only after its slot becomes writable following compositor release. Rebuild the bounded pair when geometry changes.
- [x] Add one outstanding frame callback per output; coalesce invalidations while a frame is in flight and retry queued paints after frame/buffer-release dispatch.
- [ ] Add partial damage tracking.
- [ ] Define and test fractional-scale policy; current path uses integer `wl_output` scale factors.
- [ ] Validate output hotplug, scale changes, suspend/resume and multiple outputs on a real NixOS/Hyprland session.
- [x] Track hot-plugged Wayland seats and pointer capability with SCTK `SeatState`; handle per-output pointer events without doing synchronous IPC in the callback.
- [x] Native workspace input: left-click focuses the selected workspace on the panel's monitor, vertical wheel switches relative workspaces, and the back button toggles the special workspace. Actions use one bounded command worker and a validated batched Hyprland command socket request.
- [ ] Keyboard focus/shortcuts and right-click overview toggle; the Overview subsystem is not ported yet.
- [x] Register the Hyprland broadcast socket and Wayland display fd in one blocking poll(2) loop using EventQueue::prepare_read(); drain event batches, coalesce command-socket refreshes, and invalidate output panels only when cached state changes.
- [x] Reconnect/backoff after Hyprland restarts; initial connection failures and event-stream disconnects retry from 250 ms up to a 30 s cap, with the existing poll loop providing the timer.
- [ ] Event-loop integration for D-Bus; one shared connection and no worker per feature.
- [x] Hot-reload the upstream workspace-slot setting using a non-blocking inotify directory watch integrated into the existing poll loop; atomic editor replacements and unrelated-file filtering are covered by tests.
- [x] Hyprland IPC adapter: typed read queries for monitors, workspaces, and active window plus validated monitor-scoped workspace dispatch; bounded response sizes and socket timeouts.
- [x] Pure per-monitor workspace model: active-group calculation, occupied/fake workspace indicators, safe slot lookup, and special-workspace naming, with edge-case tests.
- [x] Hyprland event-socket foundation: bounded incremental line decoder, typed workspace/window/monitor events, unknown-event preservation, and non-blocking socket drain API.
- [x] Event-to-state reducer tests for multi-monitor focus, workspace lifecycle, active-window updates, special workspaces, hotplug refreshes, and ignored events.
- [x] Pure Hyprland event-to-state reducer for focused workspaces/monitors, workspace create/destroy/move/rename, special workspace state, and active-window metadata; emits targeted refresh hints when event payloads are incomplete.
- [x] Event coordinator joins the non-blocking event stream, reducer, and command IPC; coalesces monitor/workspace/active-window refreshes, reports parse failures without losing neighboring events, and retains failed refreshes for retry.
- [x] Reconnect/backoff and Wayland event-loop fd registration for the event socket; reconnect deadlines are integrated into the existing blocking poll loop.

## Phase 2 — desktop shell parity

Port and test one subsystem at a time:

1. Top bar: per-output surfaces, workspace grouping/occupancy model, native workspace indicators, pointer input, and workspace switching are implemented. Each output renders the configured number of 26 px slots (default 10), a 22 px active marker, connected occupied pills, compact occupied/empty dots, and a labeled pill for an active special workspace; it supports left-click focus, vertical relative-workspace switching, and back-button special-workspace toggling. Remaining work includes rounded/animated rendering, app icons, keyboard/shortcut handling, right-click overview, active-window presentation, clock, system tray, battery, network, volume, media, notifications and resource indicators.
2. Quick settings and sidebars: toggles, device status, controls, search and settings launch.
3. Overview and window management: foreign-toplevel tracking, previews only on demand, workspace movement.
4. Dock and app launcher: `.desktop` indexing, fuzzy search, activation and pinned apps.
5. OSD, notifications and media: freedesktop notifications, MPRIS, audio and brightness state.
6. Background widgets, wallpaper chooser, wallpaper-derived Material palette and lock screen.
7. Screenshot/region tools, clipboard history, on-screen keyboard, translation and AI integrations as separately spawned optional components.
8. Settings UI, keybind help, first-run flow, panel-family variants and shell IPC.

## Phase 3 — integration and packaging

- Nix flake and Home Manager module for NixOS, with explicit package/runtime dependency split.
- Installer/update path that leaves the upstream user's personal overrides intact.
- Accessibility, keyboard navigation, multi-monitor, suspend/resume and session restart tests.
- Behavior checklist against upstream screenshots and workflows.
- CI, release binaries and measurable memory/performance reports.

## Performance acceptance

Do not call the port “RAM efficient” based on crate choice alone. Record idle and peak RSS, proportional set size, thread count, wakeups/sec, allocations, first-frame latency and steady-state frame time. Compare the same features, monitor setup, wallpaper, font cache state, and power profile against upstream QuickShell.

Hard constraints:

- No Qt/QtQuick in the always-running Rust process.
- No busy polling or permanent timer for unchanged state.
- Share font discovery and glyph caches.
- Reuse Wayland SHM buffers and draw directly into them.
- D-Bus adapters reuse a shared connection and are event-driven.
- Heavy AI, video, previews, downloads and image operations are lazy/on-demand.
- Avoid unbounded caches, unbounded history, and unbounded notification queues.


## Latest implementation update (2026-11-10)

- Registered the Wayland display and Hyprland broadcast socket in one blocking poll loop. No listener thread or periodic polling timer is created.
- Uses the synchronized Wayland prepare_read/read/dispatch_pending sequence, avoiding competing reads from the display socket.
- Added an allocation-free per-output frame scheduler. Redraw requests coalesce while a frame callback is outstanding and remain pending when no SHM slot is writable; dispatching buffer-release events retries pending paints.
- Requests wl_surface.frame for each submitted panel commit and schedules a follow-up only when a state/configure invalidation arrived during the prior frame.
- Hyprland connection is optional at startup. If either socket is unavailable, or the broadcast stream closes, the shell retries with bounded exponential backoff from 250 ms to 30 s. The retry deadline is folded into poll(2)'s timeout, so no timer thread or periodic wakeup is added; successful reconnection takes a fresh snapshot and invalidates all panels.
- Event batches apply the reducer and run only coalesced pending monitor/workspace/window queries. Snapshot refresh queries still run synchronously with the existing bounded socket timeout; user-initiated workspace dispatch is isolated on the single bounded command worker.
- The upstream `HyprlandData.qml` singleton listens to `Hyprland.onRawEvent` and schedules its data refreshes; the Rust adapter preserves the event-driven model while coalescing targeted queries and now reconnecting its socket after compositor restarts.
- The native top bar projects the live Hyprland monitor/workspace snapshot into one `WorkspaceModel` per output and draws the workspace strip directly into the reusable SHM buffer. It reads `bar.workspaces.shown` from the existing user config (default 10), uses 26 px spacing with a 22 px active marker, occupied pills, occupied/empty dots, and draws the active special-workspace name in a capped label pill.
- Added SCTK `SeatState` tracking and a pointer handler scoped to each panel surface. Hit testing uses logical surface coordinates, so it does not scale-click incorrectly on high-DPI outputs. Left-click selects the workspace under the pointer; wheel switches relative workspaces; the back button toggles the special workspace. A single bounded command worker performs monitor focus + action as one validated Hyprland batch, keeping synchronous socket I/O out of Wayland callbacks and the display poll loop.
- Hyprland write IPC is deliberately narrow: no arbitrary raw command strings are exposed; monitor names are character-validated, normal workspace ids must be positive, batch response size is bounded, and non-`ok` replies surface as errors. Worker queue capacity is fixed at eight.
- The workspace slot count reads the user's existing `~/.config/illogical-impulse/config.json` field `bar.workspaces.shown`; absent or invalid files/values safely fall back to the upstream default of 10. The file remains read-only. An inotify watch on its parent directory now detects in-place writes and atomic rename-based saves; relevant events trigger a bounded config reread, model rebuild and redraw without adding a thread or timer.
- App icon lookup from biggest-window client state, visual pill rounding/animations, keyboard and shortcut handling, overview/right-click behavior, and real compositor interaction tests remain pending.
- No real NixOS/Hyprland runtime test or RSS benchmark has been performed.


## Latest implementation update (2026-11-11)

- Added a non-blocking inotify watcher for the parent directory of the existing Illogical Impulse config file. It detects writes, creates, deletes and atomic rename-based editor saves for `config.json`, ignores unrelated file events, and treats inotify queue overflow as a reason to reload.
- Registered the watcher fd beside Wayland and Hyprland in the existing blocking `poll(2)` loop. No timer, watcher thread, config rewrite or periodic filesystem polling was added.
- On a relevant event, the shell rereads only the existing `bar.workspaces.shown` setting, validates it through the existing bounded parser, rebuilds per-output workspace models and redraws panels only when the effective count changes.
- Added tests for in-place writes, atomic file replacement, unrelated-file filtering and a missing config directory. These tests still need the branch CI result; they do not substitute for a live compositor test.
- The watch is armed only when the config's parent directory already exists at startup. Creating that directory later requires restarting the shell; once the directory exists, creating or replacing `config.json` is detected.
