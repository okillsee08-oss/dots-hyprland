use std::{
    collections::HashMap,
    error::Error,
    ffi::CString,
    io,
    os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::mpsc::{SyncSender, TrySendError, sync_channel},
    thread::{self, JoinHandle},
    time::Instant,
};

use ii_shell::{
    config::WorkspaceBarConfig,
    hyprland_ipc::{HyprlandIpc, WorkspaceCommand, WorkspaceDirection},
    hyprland_state::HyprlandState,
    output_model::PanelGeometry,
    render, text,
    workspace_model::WorkspaceModel,
};
use smithay_client_toolkit::reexports::client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData},
    delegate_registry,
    output::{OutputHandler, OutputInfo, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{BTN_BACK, BTN_LEFT, PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{
        Shm, ShmHandler,
        slot::{Buffer, SlotPool},
    },
};

const PANEL_HEIGHT: u32 = 36;
const INITIAL_WIDTH: u32 = 1280;
const PANEL_NAMESPACE: &str = "ii-rust-panel";
const BUFFERS_PER_OUTPUT: usize = 2;
const COMMAND_QUEUE_CAPACITY: usize = 8;

/// One small worker keeps synchronous command-socket I/O out of Wayland
/// callbacks and the display poll loop. The bounded queue prevents input spam
/// from creating an unbounded backlog or a thread per panel/widget.
struct HyprlandCommandWorker {
    sender: Option<SyncSender<WorkspaceCommand>>,
    thread: Option<JoinHandle<()>>,
}

impl HyprlandCommandWorker {
    fn spawn() -> io::Result<Self> {
        let (sender, receiver) = sync_channel::<WorkspaceCommand>(COMMAND_QUEUE_CAPACITY);
        let worker = thread::Builder::new()
            .name("ii-hyprland-command".into())
            .stack_size(256 * 1024)
            .spawn(move || run_command_worker(receiver))?;
        Ok(Self {
            sender: Some(sender),
            thread: Some(worker),
        })
    }

    fn sender(&self) -> SyncSender<WorkspaceCommand> {
        self.sender
            .as_ref()
            .expect("command worker sender is available while running")
            .clone()
    }
}

impl Drop for HyprlandCommandWorker {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.thread.take() {
            let _ = worker.join();
        }
    }
}

fn run_command_worker(receiver: std::sync::mpsc::Receiver<WorkspaceCommand>) {
    while let Ok(command) = receiver.recv() {
        let result = HyprlandIpc::connect_from_environment()
            .and_then(|ipc| ipc.dispatch_workspace(&command));
        if let Err(error) = result {
            eprintln!("ii-shell: workspace command failed: {error}");
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let connection = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init(&connection)?;
    let queue_handle = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &queue_handle)?;
    let layer_shell = LayerShell::bind(&globals, &queue_handle)?;
    let shm = Shm::bind(&globals, &queue_handle)?;

    let pool = SlotPool::new((INITIAL_WIDTH * PANEL_HEIGHT * 4) as usize, &shm)?;
    let command_worker = HyprlandCommandWorker::spawn()?;
    let workspace_shown_count = WorkspaceBarConfig::load().shown;
    let mut config_watcher = match workspace_config_path() {
        Some(path) => match WorkspaceConfigWatcher::new(&path) {
            Ok(Some(watcher)) => {
                eprintln!(
                    "ii-shell: watching {} for workspace bar changes",
                    path.display()
                );
                Some(watcher)
            }
            Ok(None) => {
                eprintln!("ii-shell: config directory is not present; live reload is unavailable");
                None
            }
            Err(error) => {
                eprintln!("ii-shell: could not watch workspace config: {error}");
                None
            }
        },
        None => None,
    };
    let mut shell = RustShell {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &queue_handle),
        pointers: Vec::new(),
        command_sender: command_worker.sender(),
        output_state: OutputState::new(&globals, &queue_handle),
        compositor,
        layer_shell,
        shm,
        pool,
        text: text::TextEngine::new(),
        panels: HashMap::new(),
        workspace_models: HashMap::new(),
        workspace_shown_count,
        workspace_models_dirty: false,
        closed: false,
    };

    let mut reconnect = ii_shell::hyprland_runtime::ReconnectBackoff::default();
    let mut hyprland = match ii_shell::hyprland_runtime::HyprlandRuntime::connect_from_environment()
    {
        Ok(runtime) => {
            reconnect.connected();
            eprintln!("ii-shell: connected to Hyprland event and command sockets");
            Some(runtime)
        }
        Err(error) => {
            reconnect.schedule_retry(Instant::now());
            let retry_in_ms = reconnect.poll_timeout_ms(Instant::now());
            eprintln!("ii-shell: Hyprland IPC unavailable; retrying in {retry_in_ms} ms: {error}");
            None
        }
    };

    eprintln!("ii-shell: waiting for Wayland outputs");
    while !shell.closed {
        event_queue.dispatch_pending(&mut shell)?;
        if shell.workspace_models_dirty {
            if let Some(runtime) = hyprland.as_ref() {
                shell.update_workspace_models(runtime.state());
                shell.request_redraw_all(&queue_handle)?;
            }
        }
        shell.draw_pending_outputs(&queue_handle)?;

        event_queue.flush()?;
        let read_guard = loop {
            if let Some(guard) = event_queue.prepare_read() {
                break guard;
            }
            event_queue.dispatch_pending(&mut shell)?;
            shell.draw_pending_outputs(&queue_handle)?;
        };

        let mut descriptors = [
            libc::pollfd {
                fd: event_queue.as_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: hyprland.as_ref().map_or(-1, |runtime| runtime.event_fd()),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: config_watcher
                    .as_ref()
                    .map_or(-1, |watcher| watcher.as_raw_fd()),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let poll_timeout_ms = if hyprland.is_none() {
            reconnect.poll_timeout_ms(Instant::now())
        } else {
            -1
        };
        let ready = unsafe {
            libc::poll(
                descriptors.as_mut_ptr(),
                descriptors.len() as libc::nfds_t,
                poll_timeout_ms,
            )
        };
        if ready < 0 {
            drop(read_guard);
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }

        let wayland_ready =
            descriptors[0].revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0;
        if wayland_ready {
            read_guard.read()?;
        } else {
            drop(read_guard);
        }

        event_queue.dispatch_pending(&mut shell)?;

        let hyprland_ready =
            descriptors[1].revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0;
        if hyprland_ready {
            let mut state_changed = false;
            let mut stream_closed = false;
            if let Some(runtime) = hyprland.as_mut() {
                match runtime.drain_ready_events() {
                    Ok(report) => {
                        state_changed |= report.state_changed;
                        stream_closed = report.stream_closed;
                        for error in report.parse_errors {
                            eprintln!("ii-shell: ignored malformed Hyprland event: {error}");
                        }
                    }
                    Err(error) => {
                        eprintln!("ii-shell: Hyprland event stream failed: {error}");
                        stream_closed = true;
                    }
                }

                if !runtime.pending_refresh().is_empty() {
                    match runtime.refresh_pending() {
                        Ok(report) => state_changed |= report.state_changed,
                        Err(error) => {
                            eprintln!("ii-shell: Hyprland state refresh failed: {error}");
                        }
                    }
                }
            }

            if stream_closed {
                hyprland = None;
                shell.clear_workspace_models();
                shell.request_redraw_all(&queue_handle)?;
                reconnect.schedule_retry(Instant::now());
                let retry_in_ms = reconnect.poll_timeout_ms(Instant::now());
                eprintln!(
                    "ii-shell: Hyprland event stream disconnected; reconnecting in {retry_in_ms} ms"
                );
            }

            if state_changed {
                if let Some(runtime) = hyprland.as_ref() {
                    shell.update_workspace_models(runtime.state());
                }
                shell.request_redraw_all(&queue_handle)?;
            }
        }

        let config_ready =
            descriptors[2].revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0;
        if config_ready {
            let mut reload_config = false;
            let mut watcher_failed = false;
            if let Some(watcher) = config_watcher.as_ref() {
                match watcher.drain_changes() {
                    Ok(changed) => reload_config = changed,
                    Err(error) => {
                        eprintln!("ii-shell: workspace config watch failed: {error}");
                        watcher_failed = true;
                    }
                }
            }
            if watcher_failed {
                config_watcher = None;
            }
            if reload_config {
                let next_count = WorkspaceBarConfig::load().shown;
                if next_count != shell.workspace_shown_count {
                    eprintln!(
                        "ii-shell: workspace slot count changed from {} to {}",
                        shell.workspace_shown_count, next_count
                    );
                    shell.workspace_shown_count = next_count;
                    if let Some(runtime) = hyprland.as_ref() {
                        shell.update_workspace_models(runtime.state());
                    }
                    shell.request_redraw_all(&queue_handle)?;
                }
            }
        }

        if hyprland.is_none() && reconnect.is_due(Instant::now()) {
            match ii_shell::hyprland_runtime::HyprlandRuntime::connect_from_environment() {
                Ok(runtime) => {
                    eprintln!("ii-shell: reconnected to Hyprland event and command sockets");
                    shell.update_workspace_models(runtime.state());
                    hyprland = Some(runtime);
                    reconnect.connected();
                    shell.request_redraw_all(&queue_handle)?;
                }
                Err(error) => {
                    reconnect.schedule_retry(Instant::now());
                    let retry_in_ms = reconnect.poll_timeout_ms(Instant::now());
                    eprintln!(
                        "ii-shell: Hyprland reconnect failed; retrying in {retry_in_ms} ms: {error}"
                    );
                }
            }
        }

        shell.draw_pending_outputs(&queue_handle)?;
    }
    Ok(())
}

struct PanelSurface {
    output: wl_output::WlOutput,
    output_name: String,
    layer: LayerSurface,
    fallback_width: Option<u32>,
    scale_factor: u32,
    geometry: Option<PanelGeometry>,
    buffers: Vec<Buffer>,
    frame_scheduler: ii_shell::frame_scheduler::FrameScheduler,
}

struct RustShell {
    registry_state: RegistryState,
    seat_state: SeatState,
    pointers: Vec<(wl_seat::WlSeat, wl_pointer::WlPointer)>,
    command_sender: SyncSender<WorkspaceCommand>,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: SlotPool,
    text: text::TextEngine,
    panels: HashMap<u32, PanelSurface>,
    workspace_models: HashMap<u32, WorkspaceModel>,
    workspace_shown_count: usize,
    workspace_models_dirty: bool,
    closed: bool,
}

impl RustShell {
    fn queue_workspace_command(&self, command: WorkspaceCommand) {
        match self.command_sender.try_send(command) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                eprintln!("ii-shell: workspace command queue is full; dropping stale input");
            }
            Err(TrySendError::Disconnected(_)) => {
                eprintln!("ii-shell: workspace command worker is unavailable");
            }
        }
    }

    fn release_pointer_for_seat(&mut self, seat: &wl_seat::WlSeat) {
        while let Some(index) = self
            .pointers
            .iter()
            .position(|(pointer_seat, _)| pointer_seat == seat)
        {
            let (_, pointer) = self.pointers.remove(index);
            pointer.release();
        }
    }

    fn add_output(
        &mut self,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
        info: OutputInfo,
    ) -> Result<(), Box<dyn Error>> {
        let output_id = info.id;
        if self.panels.contains_key(&output_id) {
            return self.update_output_metrics(qh, output_id, &info);
        }

        let scale_factor = normalized_scale(info.scale_factor);
        let fallback_width = output_logical_width(&info, scale_factor);
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Top,
            Some(format!("{PANEL_NAMESPACE}-{output_id}")),
            Some(&output),
        );

        layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, PANEL_HEIGHT);
        layer.set_exclusive_zone(PANEL_HEIGHT as i32);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.wl_surface().set_buffer_scale(scale_factor as i32);
        layer.commit();

        eprintln!(
            "ii-shell: created panel for output {} ({}) at scale {}",
            info.name.as_deref().unwrap_or("unnamed"),
            output_id,
            scale_factor
        );

        self.panels.insert(
            output_id,
            PanelSurface {
                output,
                output_name: info.name.clone().unwrap_or_default(),
                layer,
                fallback_width,
                scale_factor,
                geometry: None,
                buffers: Vec::with_capacity(BUFFERS_PER_OUTPUT),
                frame_scheduler: ii_shell::frame_scheduler::FrameScheduler::default(),
            },
        );
        self.workspace_models_dirty = true;
        Ok(())
    }

    fn update_workspace_models(&mut self, state: &HyprlandState) {
        self.workspace_models.clear();
        for (&output_id, panel) in &self.panels {
            let Some(monitor) = state
                .monitors
                .iter()
                .find(|monitor| monitor.name == panel.output_name)
            else {
                continue;
            };
            match WorkspaceModel::from_monitor(
                monitor,
                &state.workspaces,
                self.workspace_shown_count,
                false,
            ) {
                Ok(model) => {
                    self.workspace_models.insert(output_id, model);
                }
                Err(error) => {
                    eprintln!(
                        "ii-shell: workspace model for {} failed: {error}",
                        panel.output_name
                    );
                }
            }
        }
        self.workspace_models_dirty = false;
    }

    fn clear_workspace_models(&mut self) {
        self.workspace_models.clear();
        self.workspace_models_dirty = false;
    }

    fn update_output_metrics(
        &mut self,
        qh: &QueueHandle<Self>,
        output_id: u32,
        info: &OutputInfo,
    ) -> Result<(), Box<dyn Error>> {
        let scale_factor = normalized_scale(info.scale_factor);
        let fallback_width = output_logical_width(info, scale_factor);
        let geometry = {
            let Some(panel) = self.panels.get_mut(&output_id) else {
                return Ok(());
            };
            panel.fallback_width = fallback_width.or(panel.fallback_width);
            panel.scale_factor = scale_factor;
            panel
                .layer
                .wl_surface()
                .set_buffer_scale(scale_factor as i32);
            panel
                .geometry
                .map(|current| {
                    PanelGeometry::from_logical_size(
                        panel.fallback_width.unwrap_or(current.logical_width),
                        current.logical_height,
                        scale_factor,
                    )
                })
                .transpose()?
        };

        if let Some(geometry) = geometry {
            if self.set_panel_geometry(output_id, geometry)? {
                self.request_redraw(output_id, qh)?;
            }
        }
        Ok(())
    }

    fn update_panel_scale(
        &mut self,
        qh: &QueueHandle<Self>,
        output_id: u32,
        scale_factor: i32,
    ) -> Result<(), Box<dyn Error>> {
        let Ok(scale_factor) = u32::try_from(scale_factor) else {
            return Ok(());
        };
        if scale_factor == 0 {
            return Ok(());
        }

        let geometry = {
            let Some(panel) = self.panels.get_mut(&output_id) else {
                return Ok(());
            };
            panel.scale_factor = scale_factor;
            panel
                .layer
                .wl_surface()
                .set_buffer_scale(scale_factor as i32);
            panel
                .geometry
                .map(|current| {
                    PanelGeometry::from_logical_size(
                        current.logical_width,
                        current.logical_height,
                        scale_factor,
                    )
                })
                .transpose()?
        };

        if let Some(geometry) = geometry {
            if self.set_panel_geometry(output_id, geometry)? {
                self.request_redraw(output_id, qh)?;
            }
        }
        Ok(())
    }

    fn configure_panel(
        &mut self,
        qh: &QueueHandle<Self>,
        output_id: u32,
        configured_size: (u32, u32),
    ) -> Result<(), Box<dyn Error>> {
        let geometry = {
            let panel = self
                .panels
                .get(&output_id)
                .ok_or("configured layer surface has no output owner")?;
            PanelGeometry::from_configure(
                configured_size,
                panel.fallback_width,
                panel.scale_factor,
                PANEL_HEIGHT,
            )?
        };
        self.set_panel_geometry(output_id, geometry)?;
        self.request_redraw(output_id, qh)
    }

    /// Allocate a bounded pair of buffers for a new geometry. Buffer objects
    /// are retained and reused only after wl_buffer.release makes their slots
    /// writable again.
    fn set_panel_geometry(
        &mut self,
        output_id: u32,
        geometry: PanelGeometry,
    ) -> Result<bool, Box<dyn Error>> {
        let stride = i32::try_from(
            geometry
                .buffer_width
                .checked_mul(4)
                .ok_or("panel stride overflow")?,
        )?;
        let width = i32::try_from(geometry.buffer_width)?;
        let height = i32::try_from(geometry.buffer_height)?;

        let (pool, panels) = (&mut self.pool, &mut self.panels);
        let panel = panels
            .get_mut(&output_id)
            .ok_or("panel output disappeared during configure")?;
        if panel.geometry == Some(geometry) {
            return Ok(false);
        }

        let mut buffers = Vec::with_capacity(BUFFERS_PER_OUTPUT);
        for _ in 0..BUFFERS_PER_OUTPUT {
            let buffer = {
                let (buffer, _canvas) =
                    pool.create_buffer(width, height, stride, wl_shm::Format::Argb8888)?;
                buffer
            };
            buffers.push(buffer);
        }
        panel.buffers = buffers;
        panel.geometry = Some(geometry);
        Ok(true)
    }

    fn request_redraw(
        &mut self,
        output_id: u32,
        qh: &QueueHandle<Self>,
    ) -> Result<(), Box<dyn Error>> {
        let Some(panel) = self.panels.get_mut(&output_id) else {
            return Ok(());
        };
        panel.frame_scheduler.request_redraw();
        self.draw_output(output_id, qh)
    }

    fn request_redraw_all(&mut self, qh: &QueueHandle<Self>) -> Result<(), Box<dyn Error>> {
        let output_ids: Vec<_> = self.panels.keys().copied().collect();
        for output_id in output_ids {
            self.request_redraw(output_id, qh)?;
        }
        Ok(())
    }

    fn draw_pending_outputs(&mut self, qh: &QueueHandle<Self>) -> Result<(), Box<dyn Error>> {
        let output_ids: Vec<_> = self
            .panels
            .iter()
            .filter_map(|(id, panel)| panel.frame_scheduler.can_draw().then_some(*id))
            .collect();
        for output_id in output_ids {
            self.draw_output(output_id, qh)?;
        }
        Ok(())
    }

    fn draw_output(
        &mut self,
        output_id: u32,
        qh: &QueueHandle<Self>,
    ) -> Result<(), Box<dyn Error>> {
        let (pool, text, panels, workspace_models) = (
            &mut self.pool,
            &mut self.text,
            &mut self.panels,
            &self.workspace_models,
        );
        let Some(panel) = panels.get_mut(&output_id) else {
            return Ok(());
        };
        if !panel.frame_scheduler.can_draw() {
            return Ok(());
        }
        let Some(geometry) = panel.geometry else {
            return Ok(());
        };

        let Some(buffer_index) = panel
            .buffers
            .iter()
            .position(|buffer| buffer.canvas(pool).is_some())
        else {
            // Keep the invalidation pending until wl_buffer.release is handled.
            return Ok(());
        };

        let buffer = &panel.buffers[buffer_index];
        {
            let pixels = buffer
                .canvas(pool)
                .ok_or("SHM buffer became busy before drawing")?;
            render::draw_panel(pixels, geometry.buffer_width, geometry.buffer_height);
            if let Some(model) = workspace_models.get(&output_id) {
                render::draw_workspace_strip(
                    pixels,
                    geometry.buffer_width,
                    geometry.buffer_height,
                    model,
                    geometry.scale_factor,
                );
            }
            let scale = geometry.scale_factor as i32;
            let special_width = workspace_models
                .get(&output_id)
                .and_then(|model| model.special_workspace_name.as_deref())
                .map_or(0, |name| {
                    render::special_workspace_width(name, scale as u32) as i32
                });
            if let Some(name) = workspace_models
                .get(&output_id)
                .and_then(|model| model.special_workspace_name.as_deref())
            {
                let label = name.chars().take(20).collect::<String>();
                let label_x = workspace_models
                    .get(&output_id)
                    .map_or(8, |model| (8 + model.slots.len() as u32 * 26 + 5) as i32)
                    * scale;
                text.draw_text(
                    pixels,
                    geometry.buffer_width,
                    geometry.buffer_height,
                    &label,
                    (label_x, 5 * scale),
                    11.0 * scale as f32,
                );
            }
            let title_x = workspace_models
                .get(&output_id)
                .map_or(20, |model| (8 + model.slots.len() as u32 * 26 + 8) as i32)
                * scale
                + special_width;
            text.draw_text(
                pixels,
                geometry.buffer_width,
                geometry.buffer_height,
                "illogical impulse · Rust",
                (title_x, 0),
                14.0 * scale as f32,
            );
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
        }

        let surface = panel.layer.wl_surface().clone();
        buffer.attach_to(&surface)?;
        surface.damage_buffer(
            0,
            0,
            geometry.buffer_width as i32,
            geometry.buffer_height as i32,
        );
        surface.frame(qh, FrameCallbackData(surface.clone()));
        panel.layer.commit();
        panel.frame_scheduler.frame_submitted();
        Ok(())
    }
}

fn normalized_scale(scale_factor: i32) -> u32 {
    u32::try_from(scale_factor)
        .ok()
        .filter(|scale| *scale > 0)
        .unwrap_or(1)
}

fn output_logical_width(info: &OutputInfo, scale_factor: u32) -> Option<u32> {
    if let Some(width) = info
        .logical_size
        .and_then(|(width, _)| u32::try_from(width).ok())
        .filter(|width| *width > 0)
    {
        return Some(width);
    }

    info.modes
        .iter()
        .find(|mode| mode.current)
        .and_then(|mode| u32::try_from(mode.dimensions.0).ok())
        .map(|width| width.div_ceil(scale_factor))
}

impl CompositorHandler for RustShell {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        let output_id = self
            .panels
            .iter()
            .find_map(|(id, panel)| (panel.layer.wl_surface() == surface).then_some(*id));
        if let Some(output_id) = output_id {
            if let Err(error) = self.update_panel_scale(qh, output_id, new_factor) {
                eprintln!("ii-shell: output scale update failed: {error}");
                self.closed = true;
            }
        }
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        let output_id = self
            .panels
            .iter()
            .find_map(|(id, panel)| (panel.layer.wl_surface() == surface).then_some(*id));
        let Some(output_id) = output_id else {
            return;
        };

        let redraw_pending = self
            .panels
            .get_mut(&output_id)
            .is_some_and(|panel| panel.frame_scheduler.frame_completed());
        if redraw_pending {
            if let Err(error) = self.draw_output(output_id, qh) {
                eprintln!("ii-shell: frame redraw failed: {error}");
                self.closed = true;
            }
        }
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for RustShell {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let Some(info) = self.output_state.info(&output) else {
            return;
        };
        if let Err(error) = self.add_output(qh, output, info) {
            eprintln!("ii-shell: failed to create output panel: {error}");
            self.closed = true;
        }
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let Some(info) = self.output_state.info(&output) else {
            return;
        };
        let result = if self.panels.contains_key(&info.id) {
            self.update_output_metrics(qh, info.id, &info)
        } else {
            self.add_output(qh, output, info)
        };
        if let Err(error) = result {
            eprintln!("ii-shell: failed to update output panel: {error}");
            self.closed = true;
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let output_id = self.output_state.info(&output).map(|info| info.id);
        if let Some(output_id) = output_id {
            self.panels.remove(&output_id);
            self.workspace_models.remove(&output_id);
        } else {
            self.panels.retain(|_, panel| panel.output != output);
            self.workspace_models
                .retain(|id, _| self.panels.contains_key(id));
        }
    }
}

impl LayerShellHandler for RustShell {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        let output_id = self
            .panels
            .iter()
            .find_map(|(id, panel)| (panel.layer == *layer).then_some(*id));
        if let Some(output_id) = output_id {
            self.panels.remove(&output_id);
            self.workspace_models.remove(&output_id);
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let output_id = self
            .panels
            .iter()
            .find_map(|(id, panel)| (panel.layer == *layer).then_some(*id));
        let Some(output_id) = output_id else {
            return;
        };

        if let Err(error) = self.configure_panel(qh, output_id, configure.new_size) {
            eprintln!("ii-shell: panel configure failed: {error}");
            self.closed = true;
        }
    }
}

impl ShmHandler for RustShell {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl SeatHandler for RustShell {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer
            && !self
                .pointers
                .iter()
                .any(|(pointer_seat, _)| pointer_seat == &seat)
        {
            match self.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => self.pointers.push((seat, pointer)),
                Err(error) => eprintln!("ii-shell: could not acquire Wayland pointer: {error}"),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            self.release_pointer_for_seat(&seat);
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.release_pointer_for_seat(&seat);
    }
}

impl PointerHandler for RustShell {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let output_id = self.panels.iter().find_map(|(id, panel)| {
                (panel.layer.wl_surface() == &event.surface).then_some(*id)
            });
            let Some(output_id) = output_id else {
                continue;
            };

            let action = self.workspace_models.get(&output_id).and_then(|model| {
                let index = model.workspace_index_at(event.position.0, event.position.1);
                match &event.kind {
                    PointerEventKind::Press { button, .. } if *button == BTN_LEFT => index
                        .and_then(|index| model.workspace_id_at(index))
                        .map(|workspace_id| WorkspaceCommand::Focus {
                            monitor_name: model.monitor_name.clone(),
                            workspace_id,
                        }),
                    PointerEventKind::Press { button, .. } if *button == BTN_BACK => {
                        index.map(|_| WorkspaceCommand::ToggleSpecial {
                            monitor_name: model.monitor_name.clone(),
                        })
                    }
                    PointerEventKind::Axis { vertical, .. } => {
                        index?;
                        let amount = if vertical.value120 != 0 {
                            f64::from(vertical.value120)
                        } else if vertical.discrete != 0 {
                            f64::from(vertical.discrete)
                        } else {
                            vertical.absolute
                        };
                        let direction = if amount > 0.0 {
                            Some(WorkspaceDirection::Next)
                        } else if amount < 0.0 {
                            Some(WorkspaceDirection::Previous)
                        } else {
                            None
                        }?;
                        Some(WorkspaceCommand::Relative {
                            monitor_name: model.monitor_name.clone(),
                            direction,
                        })
                    }
                    _ => None,
                }
            });

            if let Some(action) = action {
                self.queue_workspace_command(action);
            }
        }
    }
}

delegate_registry!(RustShell);

impl ProvidesRegistryState for RustShell {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_dispatch2!(RustShell);

/// Resolve the existing Illogical Impulse config path without creating or
/// modifying any user-owned directories or files.
fn workspace_config_path() -> Option<PathBuf> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config_home.join("illogical-impulse/config.json"))
}

/// Event-driven watcher for the parent directory of the upstream config.
/// Watching the directory (rather than the file inode) also catches atomic
/// editor saves that replace config.json with a rename.
struct WorkspaceConfigWatcher {
    fd: OwnedFd,
    watch_descriptor: i32,
    target_name: Vec<u8>,
}

impl WorkspaceConfigWatcher {
    fn new(path: &Path) -> io::Result<Option<Self>> {
        let Some(parent) = path.parent() else {
            return Ok(None);
        };
        let Some(file_name) = path.file_name() else {
            return Ok(None);
        };
        if !parent.is_dir() {
            return Ok(None);
        }

        let directory = CString::new(parent.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "config path contains NUL"))?;
        let raw_fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC | libc::IN_NONBLOCK) };
        if raw_fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
        let mask = libc::IN_CLOSE_WRITE
            | libc::IN_MOVED_TO
            | libc::IN_CREATE
            | libc::IN_DELETE
            | libc::IN_ATTRIB
            | libc::IN_MOVED_FROM;
        let watch_descriptor =
            unsafe { libc::inotify_add_watch(fd.as_raw_fd(), directory.as_ptr(), mask) };
        if watch_descriptor < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(Some(Self {
            fd,
            watch_descriptor,
            target_name: file_name.as_bytes().to_vec(),
        }))
    }

    fn as_raw_fd(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    /// Drain all queued directory notifications. A config change is reported
    /// only for config.json (or an inotify overflow), not unrelated files.
    fn drain_changes(&self) -> io::Result<bool> {
        let mut changed = false;
        let mut buffer = [0_u8; 4096];
        let header_size = std::mem::size_of::<libc::inotify_event>();

        loop {
            let read = unsafe {
                libc::read(
                    self.fd.as_raw_fd(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                )
            };
            if read < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::WouldBlock {
                    break;
                }
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if read == 0 {
                break;
            }

            let bytes_read = read as usize;
            let mut offset = 0;
            while offset + header_size <= bytes_read {
                let event = unsafe {
                    std::ptr::read_unaligned(
                        buffer.as_ptr().add(offset).cast::<libc::inotify_event>(),
                    )
                };
                let name_start = offset + header_size;
                let name_end = name_start.saturating_add(event.len as usize);
                if name_end > bytes_read {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "truncated inotify event",
                    ));
                }

                if event.mask & libc::IN_Q_OVERFLOW != 0 {
                    changed = true;
                } else if event.wd == self.watch_descriptor && event.len > 0 {
                    let padded_name = &buffer[name_start..name_end];
                    let name = padded_name
                        .split(|byte| *byte == 0)
                        .next()
                        .unwrap_or(padded_name);
                    if name == self.target_name.as_slice() {
                        changed = true;
                    }
                }
                offset = name_end;
            }
        }

        Ok(changed)
    }
}

#[cfg(test)]
mod workspace_config_watcher_tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("ii-shell-config-watch-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn watches_in_place_writes_and_atomic_editor_replacements() {
        let directory = temp_dir();
        let config = directory.join("config.json");
        fs::write(&config, br#"{"bar":{"workspaces":{"shown":8}}}"#).unwrap();
        let watcher = WorkspaceConfigWatcher::new(&config)
            .unwrap()
            .expect("existing parent directory should be watched");

        fs::write(&config, br#"{"bar":{"workspaces":{"shown":6}}}"#).unwrap();
        assert!(watcher.drain_changes().unwrap());
        assert!(!watcher.drain_changes().unwrap());

        let replacement = directory.join(".config.json.tmp");
        fs::write(&replacement, br#"{"bar":{"workspaces":{"shown":4}}}"#).unwrap();
        fs::rename(&replacement, &config).unwrap();
        assert!(watcher.drain_changes().unwrap());

        drop(watcher);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn ignores_changes_to_unrelated_files() {
        let directory = temp_dir();
        let config = directory.join("config.json");
        fs::write(&config, b"{}").unwrap();
        let watcher = WorkspaceConfigWatcher::new(&config).unwrap().unwrap();

        fs::write(directory.join("theme.json"), b"{}").unwrap();
        assert!(!watcher.drain_changes().unwrap());

        drop(watcher);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn missing_config_directory_is_a_non_fatal_no_watch_result() {
        let directory = temp_dir();
        fs::remove_dir(&directory).unwrap();
        let config = directory.join("illogical-impulse/config.json");
        assert!(WorkspaceConfigWatcher::new(&config).unwrap().is_none());
    }
}
