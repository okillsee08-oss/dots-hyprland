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
- [x] Add a bounded, read-only Hyprland command-socket adapter for monitor, workspace, and active-window JSON state, with typed models and parser/path-validation tests.

## Phase 1 — platform/runtime

- [x] Create one output-bound top-panel layer surface per advertised `wl_output`, keyed by the output global id; remove its surface on output destruction.
- [x] Track output logical width and integer scale-factor changes, with checked logical-to-buffer geometry conversion.
- [x] Allocate two SHM buffers per output geometry and retain/reuse each `wl_buffer` only after its slot becomes writable following compositor release. Rebuild the bounded pair when geometry changes.
- [ ] Add frame-callback-driven redraw scheduling when both buffers are busy; add partial damage tracking.
- [ ] Define and test fractional-scale policy; current path uses integer `wl_output` scale factors.
- [ ] Validate output hotplug, scale changes, suspend/resume and multiple outputs on a real NixOS/Hyprland session.
- [ ] Input seats, pointer/keyboard focus, shortcut and IPC handling.
- [ ] Hyprland event socket registration with the main Wayland event loop, reconnect/backoff, and redraw scheduling from state changes.
- [ ] Event-loop integration for D-Bus; one shared connection and no worker per feature.
- [ ] Hot-reload configuration without rebuilding or restarting the shell.
- [x] Hyprland IPC adapter foundation: typed read-only queries for monitors, workspaces, and active window; bounded response size and socket timeouts.
- [x] Pure per-monitor workspace model: active-group calculation, occupied/fake workspace indicators, safe slot lookup, and special-workspace naming, with edge-case tests.
- [x] Hyprland event-socket foundation: bounded incremental line decoder, typed workspace/window/monitor events, unknown-event preservation, and non-blocking socket drain API.
- [x] Event-to-state reducer tests for multi-monitor focus, workspace lifecycle, active-window updates, special workspaces, hotplug refreshes, and ignored events.
- [x] Pure Hyprland event-to-state reducer for focused workspaces/monitors, workspace create/destroy/move/rename, special workspace state, and active-window metadata; emits targeted refresh hints when event payloads are incomplete.
- [x] Event coordinator joins the non-blocking event stream, reducer, and command IPC; coalesces monitor/workspace/active-window refreshes, reports parse failures without losing neighboring events, and retains failed refreshes for retry.
- [ ] Reconnect/backoff and Wayland event-loop fd registration for the event socket.

## Phase 2 — desktop shell parity

Port and test one subsystem at a time:

1. Top bar: per-output surfaces and workspace grouping/occupancy data model are implemented; remaining work includes native workspace rendering and interactions, active-window presentation, clock, system tray, battery, network, volume, media, notifications and resource indicators.
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
