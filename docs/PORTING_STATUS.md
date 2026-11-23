# Rust port status

**Branch:** `rust-port/phase-0`  
**Upstream:** `end-4/dots-hyprland`  
**Reference revision at port start:** `2f2a376fdb2ad738bf55f693dd0b474b64bb0ab8`  
**Latest upstream inspected (2026-11-20):** `21733c3ca7f4edc20faafedea844dc96002c6f7f`

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
- [x] Add JSON config model and basic unit tests, including read-only parsing of the upstream workspace app-icon visibility toggle.
- [x] Add GitHub Actions checks for formatting, compilation, tests, and Clippy.
- [ ] Run the first build/test on a real NixOS/Hyprland machine. GitHub source edits cannot prove compositor compatibility or runtime RSS.
- [x] Add a bounded Hyprland command-socket adapter with typed monitor/workspace/window/client queries and validated workspace dispatch, plus parser/path-validation tests.
- [x] Refresh the bounded client inventory on window open/close/move/title/fullscreen events through the existing event coordinator; failed client queries remain pending.

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
- [ ] Event-loop integration for D-Bus; the UPower adapter now accepts a shared system-bus connection and exposes a filtered signal stream, but main-loop handoff and reconnect handling remain.
- [x] Hot-reload the upstream workspace-slot setting using a non-blocking inotify directory watch integrated into the existing poll loop; atomic editor replacements and unrelated-file filtering are covered by tests.
- [x] Hyprland IPC adapter: typed read queries for monitors, workspaces, and active window plus validated monitor-scoped workspace dispatch; bounded response sizes and socket timeouts.
- [x] Pure per-monitor workspace model: active-group calculation, occupied/fake workspace indicators, safe slot lookup, and special-workspace naming, with edge-case tests.
- [x] Hyprland event-socket foundation: bounded incremental line decoder, typed workspace/window/monitor events, unknown-event preservation, and non-blocking socket drain API.
- [x] Event-to-state reducer tests for multi-monitor focus, workspace lifecycle, active-window updates, special workspaces, hotplug refreshes, and ignored events.
- [x] Pure Hyprland event-to-state reducer for focused workspaces/monitors, workspace create/destroy/move/rename, special workspace state, and active-window metadata; emits targeted refresh hints when event payloads are incomplete.
- [x] Event coordinator joins the non-blocking event stream, reducer, and command IPC; coalesces monitor/workspace/active-window refreshes, reports parse failures without losing neighboring events, and retains failed refreshes for retry.
- [x] Reconnect/backoff and Wayland event-loop fd registration for the event socket; reconnect deadlines are integrated into the existing blocking poll loop.
- [x] Drive the default local clock from the same poll loop, waking at the next minute boundary and redrawing only when its formatted time changes; no timer thread or periodic busy polling.

## Phase 2 — desktop shell parity

Port and test one subsystem at a time:

1. Top bar: per-output surfaces, workspace grouping/occupancy model, native workspace indicators, pointer input, workspace switching, local clock, focused-monitor active-window presentation, and largest-window-per-workspace projection are implemented. Each output renders the configured number of 26 px slots (default 10), a 22 px active marker, connected occupied pills, compact occupied/empty dots, and a labeled pill for an active special workspace; it supports left-click focus, vertical relative-workspace switching, and back-button special-workspace toggling. The focused output shows the active Hyprland app class/title when its workspace has a client; other outputs show the largest client in their active workspace. Each visible workspace slot now exposes its largest client for icon rendering. Remaining work includes SVG/XPM rasterization and live theme-change detection, upstream monochrome/colorizer effects, rounded/animated rendering, keyboard/shortcut handling, right-click overview, configurable/localized date and clock popup, system tray, UPower event-loop handoff and battery rendering, network, volume, media, notifications and resource indicators. The pure battery-state projection and asynchronous UPower system-bus adapter are implemented and tested; the adapter is not yet wired into the shell poll loop or panel.
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


## Latest review and verification update (2026-11-12)

- Rechecked the fork branch head before editing: `2fc030223bb3f8c759425829d48f09756baaba78`.
- Rechecked upstream `main`: latest commit remains `cb1c428bf841dabccff1b126904047c28e89ba8c` (2026-11-08). Reviewed the upstream `Workspaces.qml`, `WorkspaceModel.qml`, and `HyprlandData.qml` implementations at that revision. The QML reference still includes app-icon selection from the biggest workspace window, right-click overview toggling, Super-key press behavior, and richer visual transitions; these remain parity work, not silently assumed to be ported.
- GitHub Actions run #47 for branch head `2fc030223bb3f8c759425829d48f09756baaba78` failed at the `Format` step only. Its log identified two rustfmt layout differences in `main.rs`; compilation, tests and Clippy were skipped by the workflow after that failure.
- Corrected those two formatting differences in `main.rs` and pushed the fix. GitHub Actions run #48 for `e17ddd713c4a3a9bb5d0379c5652c61293b9deb5` and run #49 for `0ea631426d36e73810c52f3dc6c889069f113197` both completed successfully: Format, Check, Test and Clippy all passed. The test job reported 56 workspace tests plus 3 inotify watcher tests passing (59 total).
- Documented the config watcher's startup requirement: the parent directory must exist when the process starts; creating it later requires a restart. No config files are modified by the shell.
- No local Rust toolchain, NixOS/Hyprland compositor session, or runtime RSS measurement was available in this run. Do not treat the QML review or source-level changes as runtime validation.


- Final CI verification for this run: [Actions run #48](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/43241104361) and [Actions run #49](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/43241163003) are both green. Run #49 includes the README update and the same Rust source as the formatting-fix commit.


## Latest implementation update (2026-11-13)

- Rechecked the fork's Rust branch and upstream `end-4/dots-hyprland` head. Upstream remains at `cb1c428bf841dabccff1b126904047c28e89ba8c` (2026-11-08); no newer upstream commit was present on 2026-11-13.
- Reviewed the upstream `ClockWidget.qml` and `DateTime.qml`: the bar shows `DateTime.time`, optionally shows `DateTime.longDate`, and exposes a popup. The upstream clock defaults to minute precision and switches to seconds when configured or while the screen is locked. The service also maintains uptime separately.
- Fast-forwarded `rust-port/phase-0` from `0cfbdbdbf9c336c0c57ad5541eeea074e974a968` to fork merge commit `190ddd7b7f6683e2adf167e4cf659a776bb78723`. This brings in the latest upstream merge without rewriting history; the merge changed four upstream files and did not modify the Rust port.
- Added a shared Rust clock snapshot using the system local timezone and the upstream default zero-padded 12-hour `hh:mm` display. It also prepares the default `dddd, dd/MM` date string for later use.
- Added the clock's next-minute deadline to the existing blocking `poll(2)` timeout. The shell redraws panels only when the formatted time changes; no new thread, timer loop, or periodic wakeup was added.
- Added unit tests for minute/second boundary alignment, midnight and afternoon formatting, second precision, invalid time components, and poll-timeout rounding.
- The current top bar displays time only. User-configurable Qt format strings, locale-specific weekday names, verbose date visibility, clock popup, and uptime integration remain pending.
- This run could not execute local `cargo fmt`, `cargo check`, `cargo test`, or Clippy because no local Rust toolchain/repository checkout is available in the hosted editing environment. GitHub Actions is the source of build/test verification after this push.
- GitHub Actions run [#52](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/43448313114) for commit `7eb07c8a1656efbcf8a1d83d03ee80c0ac9bdb9f` passed Format, Check, Test and Clippy. The test step passed 61 workspace tests plus 3 config-watcher tests (64 total). Two intermediate Actions attempts failed at Format; the source was restored and formatting corrected before this green run.
- No real NixOS/Hyprland compositor run, output hotplug test, or RSS/performance benchmark has been performed.


## Latest implementation update (2026-11-14)

- Rechecked the fork's `rust-port/phase-0` head at `d927b13a3d6a8b76aa377faf723bd5fcfcc15c17` and compared it with `main`: the feature branch was five commits ahead and not behind. The latest upstream `end-4/dots-hyprland` commit remains `cb1c428bf841dabccff1b126904047c28e89ba8c` (2026-11-08); no newer upstream commit was present at this review.
- Inspected upstream `HyprlandData.qml`, `WorkspaceModel.qml`, `Workspaces.qml`, and `ActiveWindow.qml`. The QML data service loads the full client list and selects the largest-area client per workspace; Workspaces uses that client class for `AppSearch.guessIcon`, while ActiveWindow shows the active toplevel app id/title on the focused monitor and the largest workspace window on other monitors.
- Replaced the native bar's placeholder label with a compact two-line active-window presentation. The focused output now displays the active Hyprland window class and title from the existing cached active-window snapshot; other outputs show a Desktop/Workspace fallback until the full client inventory is implemented.
- Added a pure presentation module with tests for focused/unfocused output behavior, missing metadata, workspace fallback, and UTF-8-safe truncation. Label lengths are capped to the available space before the clock so long titles cannot paint over the clock area.
- The implementation reuses the existing Hyprland snapshot/event path and poll loop; it adds no socket, worker, timer, or dependency. The full client list, largest-window-per-workspace selection, and application icon lookup/rendering remain the next related milestone.
- GitHub Actions run [#55](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/43605086736) for the final source commit `e88b369c9bd22bc7349eb0349539d48e0490ea49` passed Format, Check, Test, and Clippy. The test step passed 66 workspace tests plus 3 config-watcher tests (69 total). Earlier run #53 failed at Format and run #54 exposed four test-only lifetime errors; both were corrected before the green run. No local Rust toolchain or NixOS/Hyprland compositor session is available in this hosted editing environment; runtime behavior and RSS/performance remain unverified.


## Latest implementation update (2026-11-15)

- Rechecked the Rust branch head at `20914124bdd4eb2dfc2fe22b9ee5271d30674472`; it is nine commits ahead of fork `main` and zero behind. The latest upstream `end-4/dots-hyprland` head is `21733c3ca7f4edc20faafedea844dc96002c6f7f` (2026-11-15).
- Reviewed upstream changes since the fork's last upstream merge. The commits `fd8f15f`, `18c214d`, and `21733c3` update Gentoo Quickshell packaging metadata only; they do not change the QML behavior being ported, so no unrelated packaging-only merge was added to this Rust feature commit.
- Re-read upstream `HyprlandData.qml`, `WorkspaceModel.qml`, `Workspaces.qml`, and `ActiveWindow.qml`. The reference queries `hyprctl clients -j`, indexes clients by address, selects the first largest-area client per workspace, uses that client class for workspace app-icon lookup, and shows the largest active-workspace client on non-focused monitors.
- Added a typed, default-tolerant Hyprland client model and a bounded `j/clients` command-socket query. The initial snapshot now includes the client inventory.
- Added coalesced client-list refresh requests for window open, close, move, title-change, fullscreen and config-reload events. The event coordinator clears the refresh flag only after a successful bounded query; failed requests remain pending.
- Added a pure largest-window selector that matches the QML reduce behavior: workspace-scoped selection, area comparison, stable first-client tie handling, and zero-area rejection. Each visible workspace slot now carries its selected client.
- Updated the two-line top-bar presentation: the focused monitor uses the active-window metadata when a client exists in its active workspace; non-focused monitors show the largest client in their active workspace. App icon lookup/rendering is still pending.
- Added tests for client JSON defaults, workspace filtering, largest-area selection, equal-area ties, missing/negative geometry, per-slot projection, event refresh coalescing, successful client refresh, and failed-refresh retention.
- No local Rust toolchain is available in this hosted editing environment. GitHub Actions [run #60](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/43727760151) for commit `5b2c3eb11933dc4dffd2270337e2031f9f6790bc` passed Format, Check, Test, and Clippy. The test step passed 76 workspace tests plus 3 config-watcher tests (79 total). Runs #56–#58 exposed formatting differences; run #59 passed Format and Check but caught three render-test fixtures missing the new workspace-model field. Those issues were corrected before run #60.

- No NixOS/Hyprland compositor session or RSS/performance benchmark was available; live runtime behavior and measured memory efficiency remain unverified.


## Latest implementation update (2026-11-16)

- Rechecked the Rust branch head at `93e89c7f6e79262e2598fce362550a326ddde560`; it is 18 commits ahead of fork `main` and zero behind. The latest upstream `end-4/dots-hyprland` head remains `21733c3ca7f4edc20faafedea844dc96002c6f7f` (2026-11-15); the November 13–15 upstream commits since the prior merge are Gentoo packaging/revision updates, with no newer QML behavior changes.
- Inspected upstream `AppSearch.qml`, `Workspaces.qml`, and `Config.qml`. The reference checks desktop-entry IDs, explicit and regex substitutions, installed icon existence, normalized names, fuzzy desktop icon/name matches, and heuristic desktop-entry lookup. `Workspaces.qml` passes the largest workspace client's class into `AppSearch.guessIcon` and `Quickshell.iconPath`; `Config.qml` defaults `bar.workspaces.showAppIcons` to true.
- Added `icon_lookup.rs`: indexes bounded XDG desktop-entry roots, resolves `StartupWMClass` and desktop IDs, mirrors the principal AppSearch substitutions and fallback stages, and searches the configured GTK theme plus common fallback themes and pixmaps.
- Added lazy PNG decoding and compositing directly into the existing reusable tiny-skia/Wayland SHM buffer. The cache is capped at 128 entries; icon files are limited to 4 MiB and 512 × 512 pixels before decoding. No extra worker, timer, D-Bus connection, or dependency was introduced.
- Added read-only parsing and live reload for `bar.workspaces.showAppIcons` through the existing config watcher. Disabling the setting restores the occupancy-dot presentation.
- SVG and XPM icon paths can be resolved but are not rasterized; those slots continue to show their occupancy dot. Upstream monochrome/colorizer effects, full icon-theme inheritance and theme-change watching remain pending.
- Added tests for desktop ID and StartupWMClass lookup, normal and regex substitutions, normalized and fuzzy lookup, theme path selection, PNG decode/cache behavior, oversized PNG rejection, and direct rendering into the caller-owned panel buffer.
- GitHub Actions [run #63](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/43908601639) for commit `93e89c7f6e79262e2598fce362550a326ddde560` passed Format, Check, Test, and Clippy. The test step passed 84 Rust tests plus 3 config-watcher tests (87 total). Run #61 initially failed at Format; run #62 passed Format, Check and Test but Clippy found one manual-contains warning and one unnecessary mutability warning. Both were fixed before run #63.
- The hosted editing environment has no local `rustc` or `cargo`, so the four commands could not be run locally; GitHub Actions executed the checks. No NixOS/Hyprland compositor session, output-hotplug test, or RSS/PSS/CPU benchmark was available. Runtime compatibility and measured memory improvement remain unverified.


## Latest implementation update (2026-11-17)

- Rechecked the fork's `rust-port/phase-0` branch against `main`: before this run it was 19 commits ahead and zero behind, at `cf062c404282adaa23e34f19e573007ea8b47131`. The new implementation commit is `7d2ed61deecd42c222e1b526973f19c55b874101`. The latest upstream `end-4/dots-hyprland` revision remains `21733c3ca7f4edc20faafedea844dc96002c6f7f` (2026-11-15); the newest upstream commits since that revision have not changed the inspected AppSearch/workspace QML behavior.
- Re-inspected upstream `dots/.config/quickshell/ii/services/AppSearch.qml` and `dots/.config/quickshell/ii/modules/ii/bar/Workspaces.qml` at `21733c3`. AppSearch uses the desktop-entry icon followed by substitutions and guesses; Workspaces resolves the biggest workspace client's class through AppSearch and Quickshell's icon theme resolver.
- Added freedesktop `index.theme` parsing to the Rust icon resolver. The configured theme now uses declared `Directories` and `ScaledDirectories`, then follows `Inherits` recursively before the common fallbacks, preserving local-theme-first lookup order.
- Added hard bounds for index.theme file size (256 KiB), theme count (32), inheritance depth (8), directory entries per theme (64), total search directories (512), and individual list items (128 bytes). Theme names and directory entries must be safe relative path components; traversal attempts and cyclic inheritance are rejected.
- Added tests for custom theme directory metadata, inherited theme fallback, local-theme priority, inheritance cycles, path traversal rejection, and hicolor fallback.
- Updated `rust/README.md` and this status record. Original upstream QML, configuration, scripts, assets and license are unchanged.
- Source implementation commit: [`7d2ed61`](https://github.com/okillsee08-oss/dots-hyprland/commit/7d2ed61deecd42c222e1b526973f19c55b874101); rustfmt correction commit: [`cc8bccd`](https://github.com/okillsee08-oss/dots-hyprland/commit/cc8bccdd0bc00782a706995b25c22192f614899b). GitHub Actions [run #69](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/43934159223) initially failed at Format only. The formatting differences were corrected in `cc8bccd`; GitHub Actions [run #71](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/43934309864) passed Format, Check, Test, and Clippy. The Test step passed 86 Rust workspace tests plus 3 config-watcher tests (89 total). This hosted environment has no local Rust toolchain, so the commands were executed by Actions rather than locally.
- Opened and merged [PR #1](https://github.com/okillsee08-oss/dots-hyprland/pull/1), integrating the tested native shell top-bar foundations into `main`. GitHub confirmed the merge commit [`4d62484`](https://github.com/okillsee08-oss/dots-hyprland/commit/4d62484c4bd45c1498ecb858b74e2898aa12ad5c). The source code had passed Actions run #71 (Format, Check, Test, Clippy; 89 tests) before integration; the final branch-head change after that run only updated documentation. The development branch was fast-forwarded to the merge commit without rewriting history.
- No real NixOS/Hyprland compositor session or RSS/PSS/CPU/frame-time benchmark has been performed. Runtime compatibility and a measured memory improvement remain unverified.


## Battery state projection update (2026-11-17)

- Rechecked the fork before implementation: `rust-port/phase-0` was one documentation commit ahead of `main` and zero behind; `main` was at `4d62484c4bd45c1498ecb858b74e2898aa12ad5c`. The upstream head remains `21733c3ca7f4edc20faafedea844dc96002c6f7f` (2026-11-15); the newest upstream commits since then are distribution-packaging and README changes, not changes to the battery QML being ported.
- Inspected upstream `services/Battery.qml`, `modules/ii/bar/BatteryIndicator.qml`, `BatteryPopup.qml`, `modules/common/Icons.qml`, and `modules/common/Config.qml`.
- Added `crates/ii-shell/src/battery.rs`, a pure UPower state projection. It maps all seven UPower device states, identifies laptop batteries using the upstream `type == Battery && powerSupply` rule, normalizes UPower's 0–100 percentage to Quickshell's 0.0–1.0 fraction, applies the upstream low/critical/suspend/full defaults (20/5/3/101), and derives the same charging, plugged-in, warning, suspend-eligibility and full-charge flags.
- The model converts UPower's signed EnergyRate (positive while discharging, negative while charging) into Quickshell's opposite-signed changeRate convention using the device state; it also rejects non-finite/out-of-range percentages, drops invalid/non-positive time estimates, derives health from full/design energy when available, and reproduces the eight upstream battery icon thresholds (7/21/35/50/64/78/93).
- Automatic suspend is exposed only as a boolean state decision. The model does not invoke `systemctl`, send notifications, play sounds, or open a D-Bus connection. The live UPower adapter, one shared D-Bus event path, panel rendering, and side effects remain outstanding.
- Added 13 unit tests covering state mapping, percentage/rate normalization, charging versus pending-charge semantics, pending-discharge, threshold defaults, automatic-suspend gating, non-battery devices, invalid numeric inputs, missing health/time estimates, zero-health handling, and icon boundaries.
- Updated `rust/README.md` and this status record. No upstream QML, config, scripts, assets, or license files were modified.
- Implementation: [`6a60d5d`](https://github.com/okillsee08-oss/dots-hyprland/commit/6a60d5d5a38d45708327f8c6d2bb9e70033e9d68); module export: [`2e0fd4a`](https://github.com/okillsee08-oss/dots-hyprland/commit/2e0fd4a6eb11ffb36ea8c78abe1a9e4f6df66823); README: [`1fab04b`](https://github.com/okillsee08-oss/dots-hyprland/commit/1fab04bade583be66476010bdee93be7e10ba61e). Formatting was corrected in [`ac0575f`](https://github.com/okillsee08-oss/dots-hyprland/commit/ac0575fa8c96619af0880d7ac832a225d4604f98) and [`101c602`](https://github.com/okillsee08-oss/dots-hyprland/commit/101c602d2f8ddaf3bd00d0ded6d05c61cb202bdf) after two Format-only failures (runs #77 and #79). GitHub Actions [run #81](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/44101860434) then passed Format, Check, Test, and Clippy with `-D warnings`; the test step passed 99 Rust workspace tests plus 3 config-watcher tests (102 total). The hosted environment has no local Rust toolchain, so these commands ran in GitHub Actions rather than locally. [PR #2](https://github.com/okillsee08-oss/dots-hyprland/pull/2) was merged after GitHub reported it mergeable and both Actions runs #81 and #82 passed. Merge commit: [`ce87038`](https://github.com/okillsee08-oss/dots-hyprland/commit/ce87038c55c21a686864ecb40333a8b0308fe09b). The development branch was fast-forwarded to that merge commit without rewriting history.
- No NixOS/Hyprland compositor test or RSS/PSS/CPU/frame-time benchmark has been performed. Runtime compatibility and memory improvement remain unverified.


## UPower system-bus adapter update (2026-11-18)

- Rechecked `rust-port/phase-0` at `99f4d53e6d2900ed2d217569ee9292114cae14a8`; it was one documentation commit ahead of `main` at `ce87038c55c21a686864ecb40333a8b0308fe09b`. The latest upstream head remains `21733c3ca7f4edc20faafedea844dc96002c6f7f` (2026-11-15); no newer upstream commits were present in the repository history on this review.
- Re-inspected upstream `services/Battery.qml`, `modules/ii/bar/BatteryIndicator.qml`, `BatteryPopup.qml`, `modules/common/Icons.qml`, and `modules/common/Config.qml`. The reference uses the aggregate UPower display device, exposes state/percentage/rate/time/health values, and reacts to property changes rather than polling.
- Verified the UPower D-Bus contract against the upstream reference manual: UPower is on the system bus; the manager exposes `GetDisplayDevice()`; the returned object implements `org.freedesktop.UPower.Device`; and device changes arrive through `org.freedesktop.DBus.Properties.PropertiesChanged`. See https://upower.freedesktop.org/docs/UPower/ and https://upower.freedesktop.org/docs/Device.html.
- Added `bus::connect_system()` while retaining `connect_session()`. UPower is not a session-bus service, so the runtime now has explicit constructors for the shared system and session connections.
- Added `crates/ii-shell/src/upower.rs`. `UPowerAdapter::new` receives a caller-owned shared system-bus connection and resolves the display object with `GetDisplayDevice`. Snapshot reads use a single `Properties.GetAll` request. `run_monitor` installs a server-filtered `PropertiesChanged` subscription for the UPower Device interface before reading its initial snapshot, then emits a complete new snapshot after each relevant signal. No per-widget connection, worker thread, timer, or polling loop is created by the adapter. The currently enabled zbus async-io backend still has its own internal executor; disabling or integrating that executor and measuring its overhead remain open.
- Added bounded decoding behavior: required identity/state/percentage properties are type-checked; optional rate/time/health properties default only when absent; malformed types fail explicitly; unknown future properties are ignored. The adapter keeps UPower's raw EnergyRate sign convention (positive discharge, negative charge), while the existing pure model converts it to Quickshell's positive-charge/negative-discharge changeRate convention.
- Added six unit tests for full decoding, rate-sign conversion, missing optional values, missing required values, wrong D-Bus types, and forward-compatible unknown properties.
- Updated `rust/README.md`, this status file, the workspace dependency manifest, and module exports. Original upstream QML, configuration, scripts, assets, and license remain untouched.
- Historical note (2026-11-18): at the time of this entry the adapter was not yet wired into the shell poll loop. The handoff and initial percentage presentation were implemented and merged in the 2026-11-19 update below. No real system-bus/UPower service or NixOS/Hyprland runtime test has been performed; RSS/PSS and wakeup measurements remain outstanding.
- Source implementation: [`0c41abc`](https://github.com/okillsee08-oss/dots-hyprland/commit/0c41abc2d47a87ac53630ffeac9d4910cb21190c); rustfmt correction: [`435245e`](https://github.com/okillsee08-oss/dots-hyprland/commit/435245ec0426b7e44e2e7078ee1a468518d1b1f1); signal-stream fix and final source/docs: [`de6a716`](https://github.com/okillsee08-oss/dots-hyprland/commit/de6a7169eec3d3610b6ea10d5320c98d2bb376fa).
- GitHub Actions run [#83](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/44324912631) failed at Format only. Run [#84](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/44325143422) passed Format but failed at Check because zbus SignalStream yields `Message` directly, not `Result<Message>`; the unnecessary `?` was removed. Run [#85](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/44325381660) for `de6a716` passed Format, Check, Test, and Clippy. The Test step passed 105 Rust workspace tests plus 3 config-watcher tests (108 total).
- The hosted environment has no local `rustc` or `cargo`; `rustc --version` was unavailable, so the four Rust checks ran in GitHub Actions. No live system-bus/UPower integration test, NixOS/Hyprland compositor test, or RSS/PSS/wakeup benchmark was performed. Historical note (2026-11-18): the adapter was then unmerged; it was integrated with the shell event loop by PR #3 on 2026-11-19.


## UPower event-loop handoff update (2026-11-19)

- At the beginning of this run, verified `rust-port/phase-0` at `d68b9383b6c9afb309df8722becfdb570ab37abd`, identical to the branch ref. Fork `main` was `ce87038c55c21a686864ecb40333a8b0308fe09b`; the feature branch was five commits ahead and zero behind. The latest upstream commit remains `21733c3ca7f4edc20faafedea844dc96002c6f7f` (2026-11-15); the recent upstream changes are packaging/README updates and do not change the battery QML.
- Re-inspected upstream `services/Battery.qml`, `BatteryIndicator.qml`, `BatteryPopup.qml`, `Icons.qml`, and `Config.qml` at the upstream revision. The service consumes the aggregate UPower display device, while the indicator displays normalized percentage and charging state; the popup reads time/rate/health.
- Added `battery_runtime.rs`: one process-wide worker owns the shared system-bus connection and the existing `UPowerAdapter`. It retries bus/device/signal failures with bounded exponential backoff from 250 ms to 30 s and can be cancelled and joined during shutdown.
- Added a single-slot `Mutex<Option<BatterySnapshot>>` mailbox plus nonblocking Linux eventfd. New snapshots replace stale pending values; eventfd wakeups coalesce, so the UI thread receives the latest state without an unbounded channel or blocking D-Bus call.
- Registered the eventfd in the existing Wayland/Hyprland/config `poll(2)` loop. The shell drains the latest snapshot and redraws only when the battery state changes. The panel now renders a compact battery percentage label to the left of the clock and reserves space so the active-window title does not overlap it.
- Added tests for latest-value coalescing, eventfd readiness/draining, empty reads, and fresh wakeups after a drain. No upstream QML, configuration, scripts, assets or license files were modified.
- Implementation commits: runtime module [`04f12e0`](https://github.com/okillsee08-oss/dots-hyprland/commit/04f12e0e5ddeb6a57c1fafd91168bb074b66f1a0); eventfd-test syntax correction [`6c85acf`](https://github.com/okillsee08-oss/dots-hyprland/commit/6c85acf898ca9dae377428cbe90e31e3d2612b1f); rustfmt corrections [`8aa57fc`](https://github.com/okillsee08-oss/dots-hyprland/commit/8aa57fc8c4476fc738c14c46516ef1ae3fdc7858); shell-loop integration [`e5c6391`](https://github.com/okillsee08-oss/dots-hyprland/commit/e5c6391973d80a3eaef26a385b93e1b2fd5cd25e); Clippy correction [`c686628`](https://github.com/okillsee08-oss/dots-hyprland/commit/c686628ab8d1cc4c61d6e09bfa6ddf0bec85b42e). Workspace/crate dependency, module-export and README commits: [`4149221`](https://github.com/okillsee08-oss/dots-hyprland/commit/41492217026f39b7be3d1f9d798db1dca88faced), [`8777d94`](https://github.com/okillsee08-oss/dots-hyprland/commit/8777d94ee4408d003d6889ecbbe52a533eb589af), [`a3c17dd`](https://github.com/okillsee08-oss/dots-hyprland/commit/a3c17dd73b0de092a7619e13a848217811432d59), and [`82ebefe`](https://github.com/okillsee08-oss/dots-hyprland/commit/82ebefe990bff036099b030e7b566e16bd6de5bc).
- GitHub Actions [run #92](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/44571108401) found a Rust parser error and rustfmt differences; [run #95](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/44571292601) found remaining rustfmt differences. [Run #97](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/44571432344) passed Format, Check and Test (107 Rust tests plus 3 config-watcher tests) but Clippy found one `unnecessary_lazy_evaluations` warning. That was fixed with `Option::and`. [Run #99](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/44571932886) for `c686628` passed Format, Check, Test and Clippy with `-D warnings`; 107 Rust workspace tests and 3 config-watcher tests passed (110 total). The hosted environment has no local Rust toolchain; verification ran in GitHub Actions.
- Opened and merged [PR #3](https://github.com/okillsee08-oss/dots-hyprland/pull/3) after run #99 passed. GitHub confirmed merge commit [`f12657e`](https://github.com/okillsee08-oss/dots-hyprland/commit/f12657e999c4014d943767ae7431195d619b2803). The development branch was fast-forwarded to that merge commit without rewriting history; `main` and `rust-port/phase-0` were aligned at the time of this update.
- Post-merge documentation reconciliation: [`fddad31`](https://github.com/okillsee08-oss/dots-hyprland/commit/fddad31e02a6235e397797b2127fbb930a2a46b0) and [`b0f7ec4`](https://github.com/okillsee08-oss/dots-hyprland/commit/b0f7ec403ace210c18f971b3d873ec0938ee125e). The current `rust-port/phase-0` branch is two documentation-only commits ahead of `main` and zero behind.
- No live system-bus/UPower service integration test, actual NixOS/Hyprland compositor test, or RSS/PSS/CPU-wakeup/frame-time benchmark has been performed. Runtime compatibility and measured memory improvement remain unverified.
- Remaining battery parity: the native panel currently shows percentage text rather than the upstream clipped progress bar and charging bolt; the popup rows, config-aware thresholds, notifications, sounds, and guarded suspend action remain unported. The adapter has decoding/unit tests but no live UPower service integration test. No NixOS/Hyprland compositor test or RSS/PSS/CPU-wakeup/frame-time benchmark has been performed.


## UPower private-bus integration coverage (2026-11-20)

- At the start of this run, verified `rust-port/phase-0` at `0c5006d53be3eadae937e5f0de40910e6ffd42e6`, with `main` at `f12657e999c4014d943767ae7431195d619b2803`; the feature branch was three commits ahead and zero behind, with only historical documentation changes. The latest upstream `end-4/dots-hyprland` head is still `21733c3ca7f4edc20faafedea844dc96002c6f7f` (2026-11-15); no newer upstream commit was found.
- Re-inspected upstream `services/Battery.qml`, `modules/ii/bar/BatteryIndicator.qml`, `BatteryPopup.qml`, `modules/common/Icons.qml`, and `modules/common/Config.qml` at that revision. The service reads the aggregate display device, the popup uses time/rate/health values, and the indicator uses normalized percentage plus charging state. No upstream QML, config, scripts, assets or license files were changed.
- Added a Linux-only integration test in `rust/crates/ii-shell/src/upower.rs`. It starts a private `dbus-daemon`, registers mock `org.freedesktop.UPower` manager and `org.freedesktop.UPower.Device` objects, and runs the production `UPowerAdapter` over a separate client connection. It verifies `GetDisplayDevice`, the initial `Properties.GetAll` snapshot, a `PropertiesChanged` update that changes state/percentage/rate, and an invalidated-property signal that must trigger a fresh complete `GetAll` snapshot. The test stops its monitor thread and terminates the private daemon; it does not contact or alter the host's real buses.
- Updated `.github/workflows/rust-port.yml` to install `dbus-daemon` explicitly on Ubuntu runners and documented the test in `rust/README.md`.
- GitHub Actions runs [#100](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/44777623819), [#101](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/44777640716), [#102](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/44777676971), [#103](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/44777698797), and [#104](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/44777774785) failed at the Format step only while the test module was being formatted; compilation, tests and Clippy were skipped on those attempts. The formatting differences were corrected. Actions [run #105](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/44777877632) for source commit `2de29e7ef2a9cd983722d1f3fef23d33af7dc329` passed Format, Check, Test and Clippy. The Test step passed 108 Rust workspace tests plus 3 config-watcher tests (111 total), including the new private-bus integration test.
- The hosted environment has no local `cargo` or `rustc`; all four Rust checks ran in GitHub Actions. The private mock service uses a session bus daemon to exercise the D-Bus contract; this is not a live system-bus/UPower test and does not prove the system service's policy or runtime behavior.
- The tested source commit is `2de29e7ef2a9cd983722d1f3fef23d33af7dc329`; the PR head after README/status documentation was `9d190002a0e1db5b0da48ceeb11d78f39f88daad`, ten commits ahead and zero behind `main`. The compared change set contained only the UPower integration test, its CI dependency, and README/status documentation.
- Remaining validation: exercise the adapter against a real system UPower service, run the shell on NixOS/Hyprland, and measure RSS/PSS, CPU wakeups and frame times. The native indicator remains a percentage label; the upstream clipped progress bar/bolt, popup presentation, config-aware thresholds, notifications/sounds and guarded suspend action remain parity work.


## UPower integration test merged to main (2026-11-20)

- [PR #4](https://github.com/okillsee08-oss/dots-hyprland/pull/4), "Add private D-Bus integration coverage for UPower", was merged after the PR-triggered Actions [run #106](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/44778255478) passed Format, Check, Test and Clippy. Its Test step passed 108 Rust workspace tests plus 3 config-watcher tests (111 total).
- GitHub confirmed merge commit [564c4ea](https://github.com/okillsee08-oss/dots-hyprland/commit/564c4ea153c15953e127e8473f468ff45b303d62). The rust-port/phase-0 ref was then advanced from PR head 9d19000 to that merge commit using a normal fast-forward update; no force-push or history rewrite was used.
- This is a validation/test-infrastructure integration, not a new live battery service feature. The mock service runs on an isolated session bus and does not replace a real UPower system-bus or NixOS/Hyprland runtime test.

## Native clipped battery indicator (2026-11-21)

- At the start of this run, verified rust-port/phase-0 at
  8f9820b27e80406f23f9013ca8a3eeaae26ecc13; main is at
  564c4ea153c15953e127e8473f468ff45b303d62. The feature branch was one
  documentation-only commit ahead and zero behind. The latest upstream
  end-4/dots-hyprland revision remains
  21733c3ca7f4edc20faafedea844dc96002c6f7f (2026-11-15); no newer upstream
  commit was found.
- Re-inspected upstream services/Battery.qml,
  modules/ii/bar/BatteryIndicator.qml, BatteryPopup.qml,
  modules/common/widgets/ClippedProgressBar.qml,
  modules/common/widgets/MaterialSymbol.qml, modules/common/Icons.qml,
  modules/common/Appearance.qml, and modules/common/Config.qml.
  The upstream indicator uses a 30-by-18 logical clipped progress area,
  error-colored fill when low and not charging, and a charging bolt below
  100%; its text is clipped out of the fill. The native implementation uses
  the same logical dimensions and state conditions, a dark contrasting text
  foreground, and a small fixed vector bolt.
- Added draw_battery_indicator to render.rs. It paints a rounded track,
  proportional fill and charging bolt directly into the caller-owned SHM
  buffer; it allocates no intermediate pixmap, icon or path and creates no
  worker/timer. The bar is reserved at a fixed width before the clock so the
  active-window title cannot overlap it.
- Added TextStyle and TextEngine::draw_text_styled so the percentage can use a contrasting
  foreground while reusing the process-wide cosmic-text font system and glyph
  cache. Existing text rendering behavior remains unchanged.
- Added pixel-level tests for proportional fill/track colors, low-battery
  color, charging-bolt visibility (including fully charged state), zero scale,
  unavailable devices and undersized buffers. Added a custom text-color test.
- Updated rust/README.md and this status record. No upstream QML,
  configuration, scripts, assets or license files were changed.
- Source commits: 4bafe5460687369ce0f2a27522f99afb4bcf558f (colored text),
  e88175683b785ebfc1f1cb9147297d863133b490 (indicator renderer),
  b29895308477320fa56b0cffcdff036bdbaf2819 (panel integration),
  706fda1224dc088ed58de0ff9d07e84e671bdba7 (edge-case assertions),
  82c52b0cdbe38b5d27a3c2183cefe047d4753567 (TextStyle refactor),
  5a4ad6116ad49d44491a9710d11c298929ffd4f1 (panel TextStyle use),
  d97ee7f10964639802ecc56a7df99e48c210d32c (rounded-rectangle parameter
  struct), and 9af2b4d8c539982f996d8b1a343cfd23026d0211 (final test import
  formatting). README commit: 6e057ac88f512c70ef9a9db89baeb7b0d236c40c.
- GitHub Actions run #112 initially stopped at Format; run #116 passed Format
  and Check but exposed missing battery test imports; run #118 stopped at
  Format; run #120 passed Format, Check and Test (113 workspace tests plus
  3 config-watcher tests) but Clippy identified two too-many-arguments
  functions. Those issues were corrected by the TextStyle and
  RoundedRectPixels value types. Final PR verification, Actions run #126,
  passed Format, Check, Test and Clippy with -D warnings. Its Test step passed
  113 Rust workspace tests and 3 config-watcher tests (116 total).
- This hosted environment has no cargo or rustc, so local Rust commands could
  not run; the successful checks were executed by GitHub Actions. No actual
  NixOS/Hyprland runtime or RSS/PSS benchmark is claimed. PR #5 was merged after confirming the final PR head was mergeable and the
  required GitHub Actions checks passed.
- Remaining battery parity includes parsing the user-configured thresholds,
  the detailed popup, low/critical/full notifications and sounds, and guarded
  automatic suspend. A real system UPower service, NixOS/Hyprland compositor,
  and RSS/PSS/wakeup/frame-time benchmarks remain untested.


## Native clipped battery indicator merged to main (2026-11-21)

- [PR #5](https://github.com/okillsee08-oss/dots-hyprland/pull/5),
  "Add native clipped battery indicator", was merged after GitHub Actions
  [run #126](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/44929534845)
  passed Format, Check, Test and Clippy with warnings denied. The Test step
  passed 113 Rust workspace tests plus 3 config-watcher tests (116 total).
- GitHub confirmed merge commit
  [fd5caff](https://github.com/okillsee08-oss/dots-hyprland/commit/fd5caffd0249bd12136e50ff2cbe3ba6afd41c39).
  The development branch was advanced to that merge commit using a normal
  fast-forward ref update with an expected-head check; no force-push or
  history rewrite was used.
- The native panel now paints the clipped battery track/fill and charging
  bolt directly into the existing SHM buffer and overlays percentage text
  through the shared font engine. The existing upstream QML remains intact.
- This is a visual parity milestone only. Configurable thresholds, the
  detailed battery popup, notifications, sounds, guarded suspend, real
  system UPower validation, NixOS/Hyprland runtime testing, and measured
  RSS/PSS/wakeup/frame-time comparisons remain outstanding.


## Battery indicator upstream-size correction (2026-11-21)

- A closer check of the inspected upstream ClippedProgressBar.qml showed that
  its default valueBarWidth/valueBarHeight are 30 × 18 logical pixels; the
  earlier 56 × 22 estimate was incorrect. Corrected the native constants to
  those upstream defaults and kept the indicator vertically centered in the
  36-pixel panel.
- Repositioned the charging bolt and percentage overlay to fit the smaller
  track. Added an outside-track pixel assertion so the fixed width is tested.
- Updated rust/README.md and this status record. The original upstream QML
  remains unchanged. PR #6 was merged after GitHub Actions run #133 passed
  Format, Check, Test and Clippy. GitHub confirmed merge commit
  [5c06faa](https://github.com/okillsee08-oss/dots-hyprland/commit/5c06faac36a995818b4573f6de77ccf0c8499e73).



## Upstream battery threshold configuration (2026-11-21)

- Re-inspected upstream `modules/common/Config.qml`, `services/Battery.qml`,
  and `modules/settings/GeneralConfig.qml` at upstream revision
  [21733c3](https://github.com/end-4/dots-hyprland/commit/21733c3ca7f4edc20faafedea844dc96002c6f7f).
  The upstream root-level `battery` object defines `low=20`,
  `critical=5`, `full=101`, `automaticSuspend=true`, and `suspend=3`.
  The settings UI permits 0–100 for low/critical/suspend and 0–101 for full.
- Added `BatteryConfig` in `config.rs` to read those exact keys from the
  existing `~/.config/illogical-impulse/config.json` without writing or
  normalizing the user-owned file. Missing, malformed, wrong-typed, negative,
  or out-of-range values fall back independently to upstream defaults.
- Added `BatterySnapshot::with_thresholds` so threshold-derived flags can be
  recomputed from the current immutable snapshot without another D-Bus round
  trip. The existing inotify config watcher now reloads battery thresholds
  together with workspace-bar settings; the panel redraws immediately when
  they change. The UPower monitor is not restarted.
- The monitor starts with the configured thresholds, and each incoming
  snapshot is reprojected on the Wayland thread using the latest config. This
  keeps the D-Bus worker independent of user-config reloads. Automatic suspend
  remains a model flag only; no suspend command is executed.
- Added parser tests for all configured keys, accepted boundaries, malformed
  values, partial configuration and invalid JSON, plus a model test proving
  threshold changes immediately alter low/critical/suspend flags without
  changing battery measurements.
- No upstream QML, configuration, scripts, assets or license files were
  modified. Real UPower, NixOS/Hyprland and memory/performance validation remain
  outstanding.
- [PR #7](https://github.com/okillsee08-oss/dots-hyprland/pull/7) was merged
  after GitHub Actions [run #141](https://github.com/okillsee08-oss/dots-hyprland/actions/runs/44931226139)
  passed Format, Check, Test and Clippy with warnings denied. The Test step
  passed 118 Rust workspace tests and 3 config-watcher tests (121 total).
- GitHub confirmed merge commit
  [2e31957](https://github.com/okillsee08-oss/dots-hyprland/commit/2e31957d3174bb20a0eeef2d50c50b0946f033e7).
  The development branch was advanced to the merge commit with a normal
  expected-head fast-forward update; no force-push or history rewrite was used.
- Initial formatting-only failures on the earlier attempts were corrected
  before the final successful PR run. The hosted environment has no local
  cargo/rustc, so all Rust checks ran in GitHub Actions.


## Native battery popup content model (2026-11-22)

- At the start of this run, verified `main` at
  [2e31957](https://github.com/okillsee08-oss/dots-hyprland/commit/2e31957d3174bb20a0eeef2d50c50b0946f033e7)
  and `rust-port/phase-0` at
  [c085034](https://github.com/okillsee08-oss/dots-hyprland/commit/c085034e9e15e853de602a35812dc4139f5a15b0).
  The development branch was one documentation-only commit ahead and zero
  behind. The latest upstream revision remains
  [21733c3](https://github.com/end-4/dots-hyprland/commit/21733c3ca7f4edc20faafedea844dc96002c6f7f)
  (2026-11-15); no newer upstream commit was found.
- Re-inspected upstream `services/Battery.qml`,
  `modules/ii/bar/BatteryIndicator.qml`,
  `modules/ii/bar/BatteryPopup.qml`,
  `modules/common/widgets/ClippedProgressBar.qml`, and
  `modules/common/Config.qml`. The popup has a time-to-full/empty row, a
  charging/discharging wattage or fully-charged row, and an always-visible
  health row. The time row is hidden for full state, absent/non-positive
  estimates, and very low power rate.
- Added `rust/crates/ii-shell/src/battery_popup.rs` and exported the module.
  `BatteryPopupModel` projects the existing immutable `BatterySnapshot`
  into typed time and power rows plus a health percentage. It preserves the
  upstream hour/minute, two-decimal watt, and one-decimal health formatting,
  including the 0.0% fallback when health is unsupported. It rejects
  unavailable devices and non-finite input values.
- The Rust battery model stores a signed change rate (positive while charging,
  negative while discharging), so the popup's time-row threshold uses the
  absolute rate magnitude while the label direction comes from UPower state.
  This avoids accidentally hiding the discharging estimate due to sign.
- Added eight unit tests for charging/discharging selection, fully charged
  presentation, the 0.01 W visibility boundary, missing estimates, minute
  formatting, unsupported/invalid health, unavailable devices, and non-finite
  power rates.
- Updated `rust/README.md`. This is only a unit-test-covered content-model
  foundation pending CI execution; no visible popup surface, renderer, hover interaction, or
  activation behavior has been implemented yet. It performs no D-Bus I/O,
  rendering, notification, sound, or power action.
- Source commits:
  [ffe9697](https://github.com/okillsee08-oss/dots-hyprland/commit/ffe9697df3d5e70e0dcfaa2263c91c16ee43d74c)
  (new model),
  [788d061](https://github.com/okillsee08-oss/dots-hyprland/commit/788d0610336c9c6a00a5166a2f9169b341a66188)
  (module export), and
  [14b19f2](https://github.com/okillsee08-oss/dots-hyprland/commit/14b19f28c31777fbb67bf3ca7ccb29c747efa6ec)
  (README), and
  [49db6de](https://github.com/okillsee08-oss/dots-hyprland/commit/49db6de73e8d07789866ddd743a339b4b8fa060a)
  (README wording clarification).
- Open review:
  [PR #8](https://github.com/okillsee08-oss/dots-hyprland/pull/8) remains open
  and unmerged. The Rust workflow is configured for push, pull-request, and
  manual dispatch, but GitHub rejected a manual dispatch with HTTP 422:
  Actions has been disabled for this repository. No Actions run or combined
  commit status is available for this PR. Both the local and remote execution
  sandboxes lack `cargo` and `rustc`, so `cargo fmt --check`,
  `cargo check`, `cargo test`, and Clippy could not be run. Do not merge
  until Actions is re-enabled and all four required checks pass.
- No component was merged into `main` in this run. The popup model remains
  on `rust-port/phase-0` pending verification. No real UPower, NixOS/Hyprland,
  accessibility, or memory/performance validation is claimed.
- Next milestone: implement a lazily created, output-aware Wayland layer
  surface for the popup. It should reuse the existing snapshot and text engine,
  allocate buffers only while visible, close on pointer leave/outside click,
  and preserve keyboard/pointer accessibility without a timer or new worker.


## Battery popup signed-rate correction (2026-11-23)

- Re-checked the current upstream `services/Battery.qml` and
  `modules/ii/bar/BatteryPopup.qml` at
  [upstream revision 21733c3](https://github.com/end-4/dots-hyprland/commit/21733c3ca7f4edc20faafedea844dc96002c6f7f).
  `Battery.energyRate` is `UPower.displayDevice.changeRate`. The official
  Quickshell UPowerDevice reference specifies that `changeRate` is positive
  while charging and negative while discharging
  ([UPowerDevice documentation](https://quickshell.org/docs/v0.3.2/types/Quickshell.Services.UPower/UPowerDevice/)).
- The popup model previously converted the signed discharging rate to a
  positive magnitude for its power row. It now preserves the signed value,
  matching upstream's `Battery.energyRate.toFixed(2)` presentation
  (for example, `-12.00W` while discharging and `4.25W` while charging).
  Added/updated assertions for both the signed discharging value and the
  low-rate boundary.
- The time-row threshold intentionally continues to use absolute rate
  magnitude. The upstream QML checks `power <= 0.01`; because discharging
  `changeRate` is negative, that condition also hides valid time-to-empty
  estimates. The native model keeps the useful time-to-empty row visible
  above the 0.01 W magnitude threshold. This is a deliberate correctness fix,
  not a literal reproduction of that sign-sensitive QML condition.
- Updated `rust/README.md` to document the signed watt display and the
  time-row correction. No upstream QML, configuration, scripts, assets or
  license files were modified.
- The change is part of open [PR #8](https://github.com/okillsee08-oss/dots-hyprland/pull/8).
  GitHub Actions remains disabled for this repository: the PR head has no
  workflow runs or combined status checks. This execution environment has no
  `cargo` or `rustc`, so `cargo fmt --check`, `cargo check`,
  `cargo test`, and Clippy could not be executed. The new/updated tests are
  therefore added but unverified; PR #8 must not be merged until Actions is
  re-enabled and all four checks pass.
- No component was merged into `main` in this run. No live UPower service,
  NixOS/Hyprland compositor, accessibility, or memory/performance validation
  is claimed. Next: restore CI, verify PR #8, then implement the lazy,
  output-aware Wayland popup surface using the shared text engine and existing
  panel/event-loop resources.
