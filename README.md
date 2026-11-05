# dots-hyprland — Rust port

This repository is a fork of [end-4/dots-hyprland](https://github.com/end-4/dots-hyprland). The original configuration and assets remain intact. The parallel native implementation is being developed under `rust/`.

## Direction

Replace the Quickshell/QtQuick shell incrementally with a native Rust Wayland shell while preserving the useful Hyprland configuration, scripts, themes, and user-facing behavior.

The native runtime is built around:

- **smithay-client-toolkit** for Wayland globals, layer-shell surfaces, shared-memory buffers, and event dispatch.
- **tiny-skia** for CPU-side 2D drawing directly into the Wayland shared-memory buffer.
- **cosmic-text** for system-font discovery, shaping, layout, and glyph rasterization.
- **zbus** for desktop D-Bus services, with a shared session connection rather than a connection per widget.

The goal is low idle memory, bounded allocations, and no Qt runtime dependency in the Rust shell. This is a staged port, not a claim of feature parity yet.

## Current status

See [the Rust port plan](rust/README.md) and [porting status](docs/PORTING_STATUS.md). Phase 0 establishes a workspace and a first Wayland layer-shell panel. The original dotfiles remain the reference implementation until native replacements have equivalent behavior and tests.
