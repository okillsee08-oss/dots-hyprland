use std::{error::Error, num::NonZeroU32};

use ii_shell::{render, text};
use smithay_client_toolkit::reexports::client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};

const PANEL_HEIGHT: u32 = 36;
const INITIAL_WIDTH: u32 = 1280;
const PANEL_NAMESPACE: &str = "ii-rust-panel";

fn main() -> Result<(), Box<dyn Error>> {
    let connection = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init(&connection)?;
    let queue_handle = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &queue_handle)?;
    let layer_shell = LayerShell::bind(&globals, &queue_handle)?;
    let shm = Shm::bind(&globals, &queue_handle)?;

    let surface = compositor.create_surface(&queue_handle);
    let layer = layer_shell.create_layer_surface(
        &queue_handle,
        surface,
        Layer::Top,
        Some(PANEL_NAMESPACE),
        None,
    );

    layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
    layer.set_size(0, PANEL_HEIGHT);
    layer.set_exclusive_zone(PANEL_HEIGHT as i32);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.commit();

    let pool = SlotPool::new((INITIAL_WIDTH * PANEL_HEIGHT * 4) as usize, &shm)?;

    let mut shell = RustShell {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &queue_handle),
        shm,
        pool,
        layer,
        text: text::TextEngine::new(),
        width: INITIAL_WIDTH,
        height: PANEL_HEIGHT,
        closed: false,
    };

    eprintln!("ii-shell: waiting for the first layer-shell configure");
    while !shell.closed {
        event_queue.blocking_dispatch(&mut shell)?;
    }
    Ok(())
}

struct RustShell {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    text: text::TextEngine,
    width: u32,
    height: u32,
    closed: bool,
}

impl RustShell {
    fn draw(&mut self) -> Result<(), Box<dyn Error>> {
        let width = self.width.max(1);
        let height = self.height.max(1);
        let stride = i32::try_from(width.checked_mul(4).ok_or("panel width overflow")?)?;

        let (buffer, pixels) = self.pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
        )?;

        render::draw_panel(pixels, width, height);
        self.text.draw_text(
            pixels,
            width,
            height,
            "illogical impulse · Rust",
            (20, 0),
            14.0,
        );

        // tiny-skia and cosmic-text write premultiplied RGBA. WL_SHM_FORMAT_ARGB8888
        // is BGRA in little-endian memory, so swap red/blue in place: no staging image.
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }

        buffer.attach_to(self.layer.wl_surface())?;
        self.layer
            .wl_surface()
            .damage_buffer(0, 0, width as i32, height as i32);
        self.layer.commit();
        Ok(())
    }
}

impl CompositorHandler for RustShell {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
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
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
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
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for RustShell {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.closed = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        self.width = NonZeroU32::new(configure.new_size.0).map_or(INITIAL_WIDTH, NonZeroU32::get);
        self.height = NonZeroU32::new(configure.new_size.1).map_or(PANEL_HEIGHT, NonZeroU32::get);

        if let Err(error) = self.draw() {
            eprintln!("ii-shell: panel render failed: {error}");
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
