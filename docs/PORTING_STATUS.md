# Rust port status

**Branch:** `rust-port/phase-0`  
**Upstream:** `end-4/dots-hyprland`  
**Reference revision at port start:** `2f2a376fdb2ad738bf55f693dd0b474b64bb0ab8`  
**Latest upstream inspected (2026-11-17):** `21733c3ca7f4edc20faafedea844dc96002c6f7f`

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
- [ ] Event-loop integration for D-Bus; one shared connection and no worker per feature.
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

1. Top bar: per-output surfaces, workspace grouping/occupancy model, native workspace indicators, pointer input, workspace switching, local clock, focused-monitor active-window presentation, and largest-window-per-workspace projection are implemented. Each output renders the configured number of 26 px slots (default 10), a 22 px active marker, connected occupied pills, compact occupied/empty dots, and a labeled pill for an active special workspace; it supports left-click focus, vertical relative-workspace switching, and back-button special-workspace toggling. The focused output shows the active Hyprland app class/title when its workspace has a client; other outputs show the largest client in their active workspace. Each visible workspace slot now exposes its largest client for icon rendering. Remaining work includes SVG/XPM rasterization and live theme-change detection, upstream monochrome/colorizer effects, rounded/animated rendering, keyboard/shortcut handling, right-click overview, configurable/localized date and clock popup, system tray, battery, network, volume, media, notifications and resource indicators.
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
