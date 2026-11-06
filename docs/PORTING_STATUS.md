# Rust port status

**Branch:** `rust-port/phase-0`  
**Upstream:** `end-4/dots-hyprland`  
**Reference revision at port start:** `2f2a376fdb2ad738bf55f693dd0b474b64bb0ab8`

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

- Output hotplug, scale-factor changes, fractional scaling policy, multi-monitor panel instances.
- Input seats, pointer/keyboard focus, shortcut and IPC handling.
- Buffer lifecycle/reuse, damage tracking, frame callbacks, bounded buffer count.
- Event-loop integration for D-Bus; one shared connection and no worker per feature.
- [x] Hyprland IPC adapter foundation: typed read-only queries for monitors, workspaces, and active window; bounded response size and socket timeouts.
- [x] Pure per-monitor workspace model: active-group calculation, occupied/fake workspace indicators, safe slot lookup, and special-workspace naming, with edge-case tests.
- [ ] Hyprland event-socket stream, reconnect/backoff, and dispatch integration.
- Hot-reload configuration without rebuilding or restarting the shell.

## Phase 2 — desktop shell parity

Port and test one subsystem at a time:

1. Top bar: workspace grouping/occupancy data model is implemented; remaining work includes native workspace rendering and interactions, active-window presentation, clock, system tray, battery, network, volume, media, notifications and resource indicators.
2. Quick settings and sidebars: toggles, device status, controls, search and settings launch.
3. Overview and window management: foreign-toplevel tracking, previews only on demand, workspace movement.
4. Dock and app launcher: .desktop indexing, fuzzy search, activation and pinned apps.
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
