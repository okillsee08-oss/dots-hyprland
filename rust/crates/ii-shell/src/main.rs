use std::{collections::HashMap, error::Error, io, os::fd::AsRawFd};

use ii_shell::{output_model::PanelGeometry, render, text};
use smithay_client_toolkit::reexports::client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData},
    delegate_registry,
    output::{OutputHandler, OutputInfo, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
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

fn main() -> Result<(), Box<dyn Error>> {
    let connection = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init(&connection)?;
    let queue_handle = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &queue_handle)?;
    let layer_shell = LayerShell::bind(&globals, &queue_handle)?;
    let shm = Shm::bind(&globals, &queue_handle)?;

    let pool = SlotPool::new((INITIAL_WIDTH * PANEL_HEIGHT * 4) as usize, &shm)?;
    let mut shell = RustShell {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &queue_handle),
        compositor,
        layer_shell,
        shm,
        pool,
        text: text::TextEngine::new(),
        panels: HashMap::new(),
        closed: false,
    };

    let mut hyprland = match ii_shell::hyprland_runtime::HyprlandRuntime::connect_from_environment()
    {
        Ok(runtime) => {
            eprintln!("ii-shell: connected to Hyprland event and command sockets");
            Some(runtime)
        }
        Err(error) => {
            eprintln!("ii-shell: Hyprland IPC unavailable; event updates disabled: {error}");
            None
        }
    };

    eprintln!("ii-shell: waiting for Wayland outputs");
    while !shell.closed {
        event_queue.dispatch_pending(&mut shell)?;
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
                fd: event_queue.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: hyprland.as_ref().map_or(-1, |runtime| runtime.event_fd()),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let ready = unsafe {
            libc::poll(
                descriptors.as_mut_ptr(),
                descriptors.len() as libc::nfds_t,
                -1,
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
                eprintln!("ii-shell: Hyprland event stream disconnected; live updates stopped");
                hyprland = None;
            }

            if state_changed {
                shell.request_redraw_all(&queue_handle)?;
            }
        }

        shell.draw_pending_outputs(&queue_handle)?;
    }
    Ok(())
}

struct PanelSurface {
    output: wl_output::WlOutput,
    layer: LayerSurface,
    fallback_width: Option<u32>,
    scale_factor: u32,
    geometry: Option<PanelGeometry>,
    buffers: Vec<Buffer>,
    frame_scheduler: ii_shell::frame_scheduler::FrameScheduler,
}

struct RustShell {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: SlotPool,
    text: text::TextEngine,
    panels: HashMap<u32, PanelSurface>,
    closed: bool,
}

impl RustShell {
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
                layer,
                fallback_width,
                scale_factor,
                geometry: None,
                buffers: Vec::with_capacity(BUFFERS_PER_OUTPUT),
                frame_scheduler: ii_shell::frame_scheduler::FrameScheduler::default(),
            },
        );
        Ok(())
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
        let (pool, text, panels) = (&mut self.pool, &mut self.text, &mut self.panels);
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
            let scale = geometry.scale_factor as i32;
            text.draw_text(
                pixels,
                geometry.buffer_width,
                geometry.buffer_height,
                "illogical impulse · Rust",
                (20 * scale, 0),
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
        } else {
            self.panels.retain(|_, panel| panel.output != output);
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

delegate_registry!(RustShell);

impl ProvidesRegistryState for RustShell {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState];
}

smithay_client_toolkit::delegate_dispatch2!(RustShell);
