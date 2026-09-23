//! GPU waterfall (FR-PAN-12/13): the waterfall is drawn by a fragment shader from a ring texture,
//! so the per-frame cost no longer grows with history depth or pane width.
//!
//! What used to happen every frame — rasterise every row on the CPU, build a fresh image handle,
//! upload the whole image — now happens once per *new row*: its bins are copied into one row of an
//! `R32Float` ring texture. A small per-row parameter texture carries each row's own centre offset
//! and span, so retuning still scrolls the history (FR-PAN-06). The fragment shader does the
//! column→bin lookup ([`k4_stream::gpu_waterfall::shader_bin`] is its tested mirror) and colours
//! through a 256-entry LUT built from the same [`k4_stream::dbm_to_color`] as before.
//!
//! The CPU rasteriser in `spectrum.rs` stays: it is the fallback when there is no wgpu adapter (a
//! shader widget draws nothing under iced's software renderer), and the reference the golden test
//! holds this against.

use std::fmt;
use std::time::{Duration, Instant};

use iced::advanced::Shell;
use iced::mouse;
use iced::widget::shader::{self, wgpu, Viewport};
use iced::window::RedrawRequest;
use iced::Rectangle;
use k4_stream::gpu_waterfall::{
    head_slot, ring_slot, row_offset_hz, rows_to_upload, waterfall_lut_rgba, LUT_LEN,
};

use crate::worker::{PanHandle, PanShared, SPECTRUM_WIDTH, WATERFALL_ROWS};

/// How long the redraw chain outlives the last new row. Rows arrive tens of times a second, so
/// this only ever lapses when the stream has stopped (or the pan is idle), which is when the UI
/// should stop spending frames on it.
const KEEP_ALIVE: Duration = Duration::from_millis(300);

/// Ring texture width: the most bins a row can carry.
const RING_W: u32 = SPECTRUM_WIDTH as u32;

const WGSL: &str = r#"
struct U {
    head: u32,
    rows_cap: u32,
    rows_shown: u32,
    ring_w: u32,
    min_db: f32,
    top_db: f32,
    view_span: f32,
    pad: f32,
};
@group(0) @binding(0) var ring: texture_2d<f32>;    // R32Float   ring_w x rows_cap   dBm bins
@group(0) @binding(1) var params: texture_2d<f32>;  // Rgba32Float rows_cap x 1        offset_hz, span_hz, bins, -
@group(0) @binding(2) var lut: texture_2d<f32>;     // Rgba8Unorm  256 x 1             colour map
@group(0) @binding(3) var<uniform> u: U;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VOut {
    let x = f32(i & 1u);
    let y = f32((i >> 1u) & 1u);
    var o: VOut;
    o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, y);
    return o;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    // Newest row at the top; `rows_shown` rows are stretched over the band (nearest row).
    let r = min(u32(in.uv.y * f32(u.rows_shown)), u.rows_shown - 1u);
    let slot = (u.head + u.rows_cap - r) % u.rows_cap;
    let p = textureLoad(params, vec2<i32>(i32(slot), 0), 0);
    let bins = u32(p.z);
    if (bins == 0u || p.y <= 0.0 || u.view_span <= 0.0) {
        return vec4<f32>(0.0);
    }
    // Same lookup as `column_to_bin`: the fragment centre is the column centre, and the row is
    // pinned to the absolute frequencies it was sampled at.
    let dx = (in.uv.x - 0.5) * u.view_span;
    let frac = (dx - p.x) / p.y + 0.5;
    if (frac < 0.0 || frac >= 1.0) {
        return vec4<f32>(0.0);
    }
    let bin = min(u32(frac * f32(bins)), bins - 1u);
    let v = textureLoad(ring, vec2<i32>(i32(bin), i32(slot)), 0).r;
    var t = 0.0;
    if (u.top_db > u.min_db) {
        t = (v - u.min_db) / (u.top_db - u.min_db);
    }
    t = select(0.0, clamp(t, 0.0, 1.0), t == t);
    let idx = i32(t * 255.0 + 0.5);
    return textureLoad(lut, vec2<i32>(idx, 0), 0);
}
"#;

/// Everything one pane keeps on the GPU.
struct PaneGpu {
    ring: wgpu::Texture,
    params: wgpu::Texture,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Rows already in the ring: the pane's `total` at the last upload.
    uploaded: u64,
    /// Rows to draw (0 = nothing to draw yet).
    shown: u32,
}

/// The pipeline and LUT are shared by both panes.
pub struct Gpu {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    lut_view: wgpu::TextureView,
    panes: [Option<PaneGpu>; 2],
}

/// What a pane needs to draw one frame.
#[derive(Debug, Clone, Copy)]
pub struct View {
    /// Pan centre / span the axis is drawn for, Hz. `span_hz == 0` = unknown: draw nothing.
    pub center_hz: i64,
    pub span_hz: u32,
    /// dBm at the top of the window, and its height in dB.
    pub top_dbm: f32,
    pub range_db: f32,
}

impl Gpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("k4 waterfall"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("k4 waterfall layout"),
            entries: &[
                tex(0),
                tex(1),
                tex(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("k4 waterfall pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("k4 waterfall"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: "vs_main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Cells no row covers are alpha 0, so they show what is under the widget.
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        // The LUT bytes are sRGB-encoded colours, exactly what the CPU path's image carries. iced
        // renders to an sRGB surface (unless built with `web-colors`), and writing to one encodes
        // linear -> sRGB, so the LUT must be an *Srgb* texture there or the picture is
        // gamma-encoded twice and washed out. On a non-sRGB target a plain Unorm LUT is right.
        let lut_format = if format.is_srgb() {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };
        let lut = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("k4 waterfall lut"),
            size: wgpu::Extent3d {
                width: LUT_LEN as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: lut_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        write_texture(queue, &lut, 0, LUT_LEN as u32, &waterfall_lut_rgba());
        let lut_view = lut.create_view(&wgpu::TextureViewDescriptor::default());
        Gpu {
            pipeline,
            layout,
            lut_view,
            panes: [None, None],
        }
    }

    fn pane(&mut self, device: &wgpu::Device, rx: usize) -> &mut PaneGpu {
        let layout = &self.layout;
        let lut_view = &self.lut_view;
        self.panes[rx].get_or_insert_with(|| {
            let make = |label, w, h, format| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            };
            let ring = make(
                "k4 waterfall ring",
                RING_W,
                WATERFALL_ROWS as u32,
                wgpu::TextureFormat::R32Float,
            );
            let params = make(
                "k4 waterfall params",
                WATERFALL_ROWS as u32,
                1,
                wgpu::TextureFormat::Rgba32Float,
            );
            let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("k4 waterfall uniforms"),
                size: 32,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let ring_view = ring.create_view(&wgpu::TextureViewDescriptor::default());
            let params_view = params.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("k4 waterfall bind group"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&ring_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&params_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(lut_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: uniforms.as_entire_binding(),
                    },
                ],
            });
            PaneGpu {
                ring,
                params,
                uniforms,
                bind_group,
                uploaded: 0,
                shown: 0,
            }
        })
    }

    /// Bring pane `rx`'s GPU state up to date with the shared history: upload the rows that
    /// arrived since last time, refresh the per-row geometry, and set the uniforms.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rx: usize,
        pan: &PanShared,
        view: View,
    ) {
        let p = self.pane(device, rx);
        let rows = pan.rows(rx);
        let total = pan.total(rx);
        for up in rows_to_upload(p.uploaded, total, rows.len()) {
            let bins = &rows[up.index].bins;
            let n = bins.len().min(SPECTRUM_WIDTH);
            write_texture(
                queue,
                &p.ring,
                ring_slot(up.seq, WATERFALL_ROWS) as u32,
                n as u32,
                bytemuck::cast_slice(&bins[..n]),
            );
        }
        p.uploaded = total;
        p.shown = if view.span_hz == 0 {
            0
        } else {
            rows.len() as u32
        };
        if p.shown == 0 {
            return;
        }
        // Each row's geometry relative to the current view (a few hundred bytes), so a retune
        // re-maps the whole history without re-uploading a single bin.
        let mut params = vec![0f32; WATERFALL_ROWS * 4];
        for (k, row) in rows.iter().enumerate() {
            let slot = ring_slot(total - 1 - k as u64, WATERFALL_ROWS);
            params[slot * 4] = row_offset_hz(view.center_hz, row.center_hz);
            params[slot * 4 + 1] = row.span_hz as f32;
            params[slot * 4 + 2] = row.bins.len().min(SPECTRUM_WIDTH) as f32;
        }
        write_texture_rgba32f(queue, &p.params, &params);
        let head = head_slot(total, WATERFALL_ROWS) as u32;
        let min_db = view.top_dbm - view.range_db;
        let mut u = [0u8; 32];
        u[0..4].copy_from_slice(&head.to_le_bytes());
        u[4..8].copy_from_slice(&(WATERFALL_ROWS as u32).to_le_bytes());
        u[8..12].copy_from_slice(&p.shown.to_le_bytes());
        u[12..16].copy_from_slice(&RING_W.to_le_bytes());
        u[16..20].copy_from_slice(&min_db.to_le_bytes());
        u[20..24].copy_from_slice(&view.top_dbm.to_le_bytes());
        u[24..28].copy_from_slice(&(view.span_hz as f32).to_le_bytes());
        queue.write_buffer(&p.uniforms, 0, &u);
    }

    /// Draw pane `rx` into the current viewport of `pass` (a no-op until there is something to draw).
    pub fn draw<'a>(&'a self, rx: usize, pass: &mut wgpu::RenderPass<'a>) {
        let Some(p) = self.panes[rx].as_ref().filter(|p| p.shown > 0) else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &p.bind_group, &[]);
        pass.draw(0..4, 0..1);
    }
}

/// Copy one row of texels (`width` of them, 4 bytes each) into texture row `y`.
fn write_texture(queue: &wgpu::Queue, tex: &wgpu::Texture, y: u32, width: u32, bytes: &[u8]) {
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d { x: 0, y, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
}

fn write_texture_rgba32f(queue: &wgpu::Queue, tex: &wgpu::Texture, floats: &[f32]) {
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(floats),
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(WATERFALL_ROWS as u32 * 16),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: WATERFALL_ROWS as u32,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
}

/// Whether a wgpu adapter is available — i.e. whether iced will be rendering through wgpu, so a
/// shader widget will actually draw. Under iced's software fallback it would not, and the CPU
/// waterfall is used instead. `K4_WATERFALL=cpu` (or `gpu`) overrides, for diagnosis.
pub fn gpu_available() -> bool {
    match std::env::var("K4_WATERFALL").as_deref() {
        Ok("cpu") => return false,
        Ok("gpu") => return true,
        _ => {}
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
    iced::futures::executor::block_on(
        instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )
    .is_some()
}

// ---------------------------------------------------------------------------------------------
// The iced widget.

/// The waterfall of one pane, as a `shader` widget.
pub struct WaterfallProgram {
    pub pan: PanHandle,
    pub rx: usize,
    pub view: View,
}

/// Window frames drawn with a waterfall in them, for the opt-in `K4_FPS=1` frame-rate line.
pub static FRAMES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The `now` of the last redraw counted in [`FRAMES`].
static LAST_FRAME: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

/// Whether a redraw at `now` is a window frame not yet counted.
///
/// Every pane gets the **same** `RedrawRequested(now)` for one window redraw — iced makes one
/// `Instant` per redraw and hands that event to the whole widget tree — so counting once per pane,
/// as the first version did, reported a dual A+B view at twice its real frame rate (found on the
/// radio: 40 in dual, 20 in single, for the same stream).
fn first_sight(last: &mut Option<Instant>, now: Instant) -> bool {
    if *last == Some(now) {
        false
    } else {
        *last = Some(now);
        true
    }
}

/// With `K4_FPS=1` in the environment, print `FPS <frames in the last second>` once a second to
/// stderr. A measurement aid: how often the window really redraws is what GPU load follows.
pub fn spawn_fps_report() {
    if std::env::var_os("K4_FPS").is_none() {
        return;
    }
    let _ = std::thread::Builder::new().name("fps".into()).spawn(|| {
        let mut last = 0;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let now = FRAMES.load(std::sync::atomic::Ordering::Relaxed);
            eprintln!("FPS {}", now - last);
            last = now;
        }
    });
}

/// The fastest the chain redraws, however fast rows arrive: about 125 frames a second.
pub const MIN_PERIOD: Duration = Duration::from_millis(8);
/// The slowest the chain redraws while rows are arriving: the UI tick's own 10 Hz.
pub const MAX_PERIOD: Duration = Duration::from_millis(100);
/// The row interval assumed until one has been measured.
const INITIAL_PERIOD: Duration = Duration::from_millis(33);
/// How much of each new measurement of the row interval is taken in (the rest is the old estimate).
const SMOOTHING: f64 = 0.25;

/// Redraw-chain bookkeeping (FR-PAN-13).
///
/// The window is redrawn as a whole, so what GPU load follows is how *often* it is redrawn. Rows
/// arrive at 20–30 a second; a frame between two rows shows nothing new. The chain therefore asks
/// for its next frame one row interval ahead, not for the next vsync.
#[derive(Default)]
pub struct RedrawState {
    seen_total: u64,
    last_change: Option<Instant>,
    /// Estimated time between rows, from how many arrived between frames.
    period: Option<Duration>,
}

impl RedrawState {
    /// Called on every redraw with the pane's current row count: when should the *next* frame be
    /// drawn? `None` ends the chain — nothing has ever arrived, or the stream has been quiet for
    /// longer than [`KEEP_ALIVE`] — and the UI tick's own redraw restarts it when rows resume.
    /// Otherwise one estimated row interval from `now`, kept between [`MIN_PERIOD`] and
    /// [`MAX_PERIOD`].
    pub fn next_frame_at(&mut self, total: u64, now: Instant) -> Option<Instant> {
        if total != self.seen_total {
            // Rows since the last frame. A total that went *down* (a cleared history) counts as one.
            let rows = total
                .checked_sub(self.seen_total)
                .filter(|r| *r > 0)
                .unwrap_or(1);
            if let Some(prev) = self.last_change {
                let gap = now.saturating_duration_since(prev);
                // A gap longer than the grace is a stream that stopped and restarted, not a row
                // interval.
                if gap < KEEP_ALIVE {
                    let sample = gap.as_secs_f64() / rows as f64;
                    let old = self.period.unwrap_or(INITIAL_PERIOD).as_secs_f64();
                    let blended = old * (1.0 - SMOOTHING) + sample * SMOOTHING;
                    self.period = Some(
                        Duration::from_secs_f64(blended.max(0.0)).clamp(MIN_PERIOD, MAX_PERIOD),
                    );
                }
            }
            self.seen_total = total;
            self.last_change = Some(now);
        }
        let active = self
            .last_change
            .is_some_and(|t| now.saturating_duration_since(t) < KEEP_ALIVE);
        // The stored estimate is always inside [MIN_PERIOD, MAX_PERIOD], so it needs no clamp here.
        active.then(|| now + self.period.unwrap_or(INITIAL_PERIOD))
    }
}

impl<Message> shader::Program<Message> for WaterfallProgram {
    type State = RedrawState;
    type Primitive = WaterfallPrimitive;

    /// Keep frames coming at the rate rows arrive, independent of the 100 ms UI tick (FR-PAN-13).
    /// Each redraw asks for the next one a row interval ahead while the row count is still moving;
    /// once the stream has been quiet for [`KEEP_ALIVE`] the chain stops, so an idle or
    /// disconnected pan costs nothing. The tick's own redraw restarts it when rows resume.
    fn update(
        &self,
        state: &mut RedrawState,
        event: shader::Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
        shell: &mut Shell<'_, Message>,
    ) -> (iced::event::Status, Option<Message>) {
        if let shader::Event::RedrawRequested(now) = event {
            if LAST_FRAME
                .lock()
                .is_ok_and(|mut last| first_sight(&mut last, now))
            {
                FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            let total = self.pan.lock().map(|p| p.total(self.rx)).unwrap_or(0);
            if let Some(at) = state.next_frame_at(total, now) {
                shell.request_redraw(RedrawRequest::At(at));
            }
        }
        (iced::event::Status::Ignored, None)
    }

    fn draw(
        &self,
        _state: &RedrawState,
        _cursor: mouse::Cursor,
        _bounds: Rectangle,
    ) -> WaterfallPrimitive {
        WaterfallPrimitive {
            pan: PanHandle::clone(&self.pan),
            rx: self.rx,
            view: self.view,
        }
    }
}

pub struct WaterfallPrimitive {
    pan: PanHandle,
    rx: usize,
    view: View,
}

impl fmt::Debug for WaterfallPrimitive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WaterfallPrimitive")
            .field("rx", &self.rx)
            .finish()
    }
}

/// Where the pane's waterfall lands, in physical pixels — worked out in `prepare`, used in `render`.
struct Placement(Vec<Option<[f32; 4]>>);

impl shader::Primitive for WaterfallPrimitive {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        storage: &mut shader::Storage,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        if !storage.has::<Gpu>() {
            storage.store(Gpu::new(device, queue, format));
            storage.store(Placement(vec![None, None]));
        }
        let sf = viewport.scale_factor() as f32;
        storage.get_mut::<Placement>().expect("stored with Gpu").0[self.rx] = Some([
            bounds.x * sf,
            bounds.y * sf,
            bounds.width * sf,
            bounds.height * sf,
        ]);
        let gpu = storage.get_mut::<Gpu>().expect("stored above");
        if let Ok(pan) = self.pan.lock() {
            gpu.prepare(device, queue, self.rx, &pan, self.view);
        }
    }

    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        storage: &shader::Storage,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let (Some(gpu), Some(place)) = (storage.get::<Gpu>(), storage.get::<Placement>()) else {
            return;
        };
        let Some(Some(r)) = place.0.get(self.rx) else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("k4 waterfall pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_viewport(r[0], r[1], r[2], r[3], 0.0, 1.0);
        pass.set_scissor_rect(
            clip_bounds.x,
            clip_bounds.y,
            clip_bounds.width,
            clip_bounds.height,
        );
        gpu.draw(self.rx, &mut pass);
    }
}

// ---------------------------------------------------------------------------------------------
// Golden test: the GPU output, pixel for pixel, against the CPU rasteriser it replaces.

#[cfg(test)]
mod golden {
    use super::*;
    use crate::spectrum::waterfall_rgba;
    use crate::worker::PanRow;

    // Deliberately awkward sizes. At a round geometry (640 px, 300 rows, span 24 000, 1024 bins) a
    // pixel centre lands *exactly* on a bin or row boundary every few pixels, and there f32 and the
    // CPU's f64 legitimately break the tie in opposite directions, so the two disagree without
    // either being wrong. H = 320 makes `(y + 0.5) * rows / H` never an integer for 64 (or 5) rows;
    // an odd width and non-round spans make `frac * bins` essentially never one.
    const W: u32 = 637;
    const H: u32 = 320;
    const SPAN: u32 = 47_993;
    const TOP: f32 = -40.0;
    const RANGE: f32 = 90.0;

    /// Deterministic bins in [-130, -40] dBm with structure (a ramp plus noise), so a wrong bin or a
    /// wrong row is visible as a colour difference and not hidden in a flat field.
    fn bins(seed: u32, n: usize) -> Vec<f32> {
        let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
        (0..n)
            .map(|i| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                let noise = (s % 1000) as f32 / 1000.0;
                -130.0 + 90.0 * (0.5 * (i as f32 / n as f32) + 0.5 * noise)
            })
            .collect()
    }

    fn row(seed: u32, center_hz: i64, span_hz: u32, n: usize) -> PanRow {
        PanRow {
            bins: bins(seed, n),
            center_hz,
            span_hz,
        }
    }

    struct Rig {
        device: wgpu::Device,
        queue: wgpu::Queue,
    }

    fn rig() -> Rig {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let adapter = iced::futures::executor::block_on(
            instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
        )
        .expect("a wgpu adapter is required for this test (it is #[ignore]d for that reason)");
        let (device, queue) = iced::futures::executor::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor::default(), None),
        )
        .expect("wgpu device");
        Rig { device, queue }
    }

    /// Render pane 0 into a W×H target of `format`, cleared to black, and read the bytes back.
    fn render(
        rig: &Rig,
        gpu: &mut Gpu,
        format: wgpu::TextureFormat,
        pan: &PanShared,
        view: View,
    ) -> Vec<u8> {
        gpu.prepare(&rig.device, &rig.queue, 0, pan, view);
        let target = rig.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("golden target"),
            size: wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let tview = target.create_view(&wgpu::TextureViewDescriptor::default());
        // Readback rows must be a multiple of 256 bytes; an odd width is not, so pad and strip.
        let pitch = (W * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback = rig.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("golden readback"),
            size: u64::from(pitch * H),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = rig
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("golden pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &tview,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_viewport(0.0, 0.0, W as f32, H as f32, 0.0, 1.0);
            gpu.draw(0, &mut pass);
        }
        enc.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(pitch),
                    rows_per_image: Some(H),
                },
            },
            wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
        );
        rig.queue.submit([enc.finish()]);
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        rig.device.poll(wgpu::Maintain::Wait);
        let mapped = readback.slice(..).get_mapped_range();
        let mut data = Vec::with_capacity((W * H * 4) as usize);
        for y in 0..H as usize {
            let start = y * pitch as usize;
            data.extend_from_slice(&mapped[start..start + (W * 4) as usize]);
        }
        drop(mapped);
        readback.unmap();
        data
    }

    /// What the app drew before: the CPU image, stretched over the band with nearest sampling;
    /// alpha 0 texels show the (black) background.
    fn reference(rows: &[PanRow], view: View) -> Vec<u8> {
        let rgba = waterfall_rgba(
            rows,
            view.center_hz,
            view.span_hz,
            view.top_dbm,
            view.range_db,
            W as usize,
        );
        let n = rows.len();
        let mut out = vec![0u8; (W * H * 4) as usize];
        for py in 0..H as usize {
            let r = (((py as f32 + 0.5) / H as f32) * n as f32) as usize;
            let r = r.min(n - 1);
            for px in 0..W as usize {
                let s = (r * W as usize + px) * 4;
                let d = (py * W as usize + px) * 4;
                if rgba[s + 3] != 0 {
                    out[d..d + 3].copy_from_slice(&rgba[s..s + 3]);
                }
                out[d + 3] = 255;
            }
        }
        out
    }

    /// (worst channel error, pixels off by more than `tol`, opaque pixels)
    fn compare(gpu: &[u8], cpu: &[u8], tol: i32) -> (i32, usize, usize) {
        let (mut worst, mut off, mut opaque) = (0, 0, 0);
        for (g, c) in gpu.chunks(4).zip(cpu.chunks(4)) {
            let e = (0..3)
                .map(|i| (g[i] as i32 - c[i] as i32).abs())
                .max()
                .unwrap();
            worst = worst.max(e);
            if e > tol {
                off += 1;
            }
            if c[0] != 0 || c[1] != 0 || c[2] != 0 {
                opaque += 1;
            }
        }
        (worst, off, opaque)
    }

    /// Which history rows the differing pixels belong to: the first thing to know when the two
    /// disagree. One line per row that has any.
    fn explain(gpu: &[u8], cpu: &[u8], rows: &[PanRow], view: View, tol: i32) -> String {
        let n = rows.len();
        // (differing, gpu blank but cpu not, cpu blank but gpu not)
        let mut per_row = vec![(0usize, 0usize, 0usize); n];
        for py in 0..H as usize {
            let r = ((((py as f32 + 0.5) / H as f32) * n as f32) as usize).min(n - 1);
            for px in 0..W as usize {
                let i = (py * W as usize + px) * 4;
                let e = (0..3)
                    .map(|k| (gpu[i + k] as i32 - cpu[i + k] as i32).abs())
                    .max()
                    .unwrap();
                if e > tol {
                    per_row[r].0 += 1;
                    let g_blank = gpu[i..i + 3] == [0, 0, 0];
                    let c_blank = cpu[i..i + 3] == [0, 0, 0];
                    if g_blank && !c_blank {
                        per_row[r].1 += 1;
                    }
                    if c_blank && !g_blank {
                        per_row[r].2 += 1;
                    }
                }
            }
        }
        let mut out = String::new();
        // Per scanline: whole-scanline disagreements (a row-boundary tie) look very different from
        // scattered ones (a column/bin problem).
        let mut lines = String::new();
        for py in 0..H as usize {
            let mut c = 0;
            for px in 0..W as usize {
                let i = (py * W as usize + px) * 4;
                if (0..3)
                    .map(|k| (gpu[i + k] as i32 - cpu[i + k] as i32).abs())
                    .max()
                    .unwrap()
                    > tol
                {
                    c += 1;
                }
            }
            if c > 0 {
                lines += &format!(" {py}:{c}");
            }
        }
        out += &format!("  differing pixels per scanline (y:count):{lines}\n");
        for (r, (d, gb, cb)) in per_row.iter().enumerate() {
            if *d > 0 {
                let row = &rows[r];
                out += &format!(
                    "  row {r:>2}: {d:>4} px differ (gpu blank {gb}, cpu blank {cb})  centre {:+} Hz, span {}, bins {}\n",
                    row.center_hz - view.center_hz,
                    row.span_hz,
                    row.bins.len()
                );
            }
        }
        out
    }

    /// Pixels where the two disagree by more than `tol` **and** that are not near a rounding
    /// boundary. A disagreement is only acceptable where the exact position sits (within f32
    /// rounding) on the edge between two bins, two rows, or the edge of a row's range, because
    /// there either answer is right. Anything else is a real defect. Returns those pixels.
    fn unexplained(
        gpu: &[u8],
        cpu: &[u8],
        rows: &[PanRow],
        view: View,
        tol: i32,
    ) -> Vec<(usize, usize)> {
        let n = rows.len();
        let mut bad = Vec::new();
        for py in 0..H as usize {
            let v = (py as f64 + 0.5) / H as f64 * n as f64;
            let row_tie = (v - v.round()).abs() < 1e-3;
            let r = (v as usize).min(n - 1);
            let row = &rows[r];
            for px in 0..W as usize {
                let i = (py * W as usize + px) * 4;
                let e = (0..3)
                    .map(|k| (gpu[i + k] as i32 - cpu[i + k] as i32).abs())
                    .max()
                    .unwrap();
                if e <= tol {
                    continue;
                }
                let hz = (view.center_hz as f64 - view.span_hz as f64 / 2.0)
                    + (px as f64 + 0.5) * view.span_hz as f64 / W as f64;
                let frac =
                    (hz - (row.center_hz as f64 - row.span_hz as f64 / 2.0)) / row.span_hz as f64;
                let scaled = frac * row.bins.len() as f64;
                let bin_tie = (scaled - scaled.round()).abs() < 2e-3;
                let range_edge = frac.abs() < 1e-5 || (frac - 1.0).abs() < 1e-5;
                if !(row_tie || bin_tie || range_edge) {
                    bad.push((px, py));
                }
            }
        }
        bad
    }

    fn check_explained(got: &[u8], rows: &[PanRow], view: View, what: &str) {
        let cpu = reference(rows, view);
        let bad = unexplained(got, &cpu, rows, view, 3);
        assert!(
            bad.is_empty(),
            "{what}: {} pixels disagree with the CPU rasteriser away from any rounding boundary, e.g. {:?}",
            bad.len(),
            &bad[..bad.len().min(8)]
        );
    }

    fn push_all(pan: &mut PanShared, rows: &[PanRow]) {
        for r in rows {
            pan.push(0, r.clone());
        }
    }

    /// Mixed geometry: rows retuned by different amounts, one at another span, short rows.
    fn mixed(n: usize, seed0: u32) -> Vec<PanRow> {
        let c = 14_074_000i64;
        (0..n)
            .map(|i| {
                let seed = seed0 + i as u32;
                match i % 6 {
                    0 => row(seed, c, SPAN, 1024),
                    1 => row(seed, c + i64::from(SPAN) / 3, SPAN, 1024), // retuned up by a third of the span
                    2 => row(seed, c - i64::from(SPAN) / 2, SPAN, 1024), // retuned down by half a span
                    3 => row(seed, c, SPAN / 2 + 3, 1024),               // narrower span
                    4 => row(seed, c + i as i64 * 137, SPAN * 2 + 11, 512), // wider span, fewer bins
                    _ => row(seed, c, SPAN, 37),                            // a very short row
                }
            })
            .collect()
    }

    fn view() -> View {
        View {
            center_hz: 14_074_000,
            span_hz: 48_000,
            top_dbm: TOP,
            range_db: RANGE,
        }
    }

    /// The GPU waterfall matches the CPU rasteriser it replaces, for every history shape that can
    /// occur — and on both an sRGB and a linear target, which is where a double gamma would show.
    /// trace: FR-PAN-12, FR-PAN-06
    #[test]
    #[ignore = "needs a GPU adapter: cargo test -p k4remote -- --ignored gpu_golden"]
    fn fr_pan_12_gpu_golden_matches_cpu_rasteriser() {
        let rig = rig();
        for format in [
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ] {
            // A: a full history with mixed geometry.
            let mut gpu = Gpu::new(&rig.device, &rig.queue, format);
            let all = mixed(WATERFALL_ROWS, 1);
            let mut pan = PanShared::default();
            push_all(&mut pan, &all);
            let rows: Vec<PanRow> = pan.rows(0).iter().cloned().collect(); // newest first, as the app holds them
            let got = render(&rig, &mut gpu, format, &pan, view());
            let (worst, off, opaque) = compare(&got, &reference(&rows, view()), 3);
            check_explained(&got, &rows, view(), "full history");
            eprintln!(
                "[{format:?}] full history: worst {worst}, off {off}, opaque {opaque}/{}",
                W * H
            );
            assert!(
                opaque as u32 > W * H / 2,
                "a vacuous comparison proves nothing ({opaque})"
            );
            let cpu = reference(&rows, view());
            assert!(
                off * 500 < (W * H) as usize,
                "[{format:?}] full: {off} pixels differ, worst {worst}\n{}",
                explain(&got, &cpu, &rows, view(), 3)
            );

            // B: more rows than the ring holds, uploaded incrementally across the wrap.
            let mut gpu = Gpu::new(&rig.device, &rig.queue, format);
            let all = mixed(100, 7);
            let mut pan = PanShared::default();
            push_all(&mut pan, &all[..30]);
            let _ = render(&rig, &mut gpu, format, &pan, view()); // first upload
            push_all(&mut pan, &all[30..]); // 70 more: the ring wraps
            let rows: Vec<PanRow> = pan.rows(0).iter().cloned().collect();
            let got = render(&rig, &mut gpu, format, &pan, view());
            let (worst, off, _) = compare(&got, &reference(&rows, view()), 3);
            check_explained(&got, &rows, view(), "wrapped ring");
            eprintln!("[{format:?}] wrapped ring: worst {worst}, off {off}");
            assert_eq!(rows.len(), WATERFALL_ROWS);
            assert!(
                off * 500 < (W * H) as usize,
                "[{format:?}] wrapped: {off} pixels differ, worst {worst}"
            );

            // C: a partial history (5 rows, stretched over the band like the CPU path).
            let mut gpu = Gpu::new(&rig.device, &rig.queue, format);
            let mut pan = PanShared::default();
            push_all(&mut pan, &mixed(5, 3));
            let rows: Vec<PanRow> = pan.rows(0).iter().cloned().collect();
            let got = render(&rig, &mut gpu, format, &pan, view());
            let (worst, off, _) = compare(&got, &reference(&rows, view()), 3);
            check_explained(&got, &rows, view(), "5 rows");
            eprintln!("[{format:?}] 5 rows: worst {worst}, off {off}");
            assert!(
                off * 500 < (W * H) as usize,
                "[{format:?}] partial: {off} pixels differ, worst {worst}"
            );

            // D: retune with no new rows. Only geometry changes, so nothing is re-uploaded, and the
            // history must still scroll to match the CPU path (FR-PAN-06).
            let mut gpu = Gpu::new(&rig.device, &rig.queue, format);
            let mut pan = PanShared::default();
            push_all(&mut pan, &mixed(WATERFALL_ROWS, 11));
            let rows: Vec<PanRow> = pan.rows(0).iter().cloned().collect();
            let _ = render(&rig, &mut gpu, format, &pan, view());
            let retuned = View {
                center_hz: 14_074_000 + 12_345,
                ..view()
            };
            let got = render(&rig, &mut gpu, format, &pan, retuned);
            let (worst, off, opaque) = compare(&got, &reference(&rows, retuned), 3);
            check_explained(&got, &rows, retuned, "retuned");
            eprintln!("[{format:?}] retuned: worst {worst}, off {off}, opaque {opaque}");
            assert!(opaque as u32 > W * H / 4);
            assert!(
                off * 500 < (W * H) as usize,
                "[{format:?}] retune: {off} pixels differ, worst {worst}"
            );
            // …and it really did move: the retuned picture differs from the untuned one.
            let before = render(&rig, &mut gpu, format, &pan, view());
            assert_ne!(before, got, "a retune must change the picture");
        }
    }

    /// No span known (`span_hz == 0`) draws nothing, and an empty history draws nothing: the
    /// canvas's placeholder / axis cover that case, and the shader must not scribble over it.
    /// trace: FR-PAN-12
    #[test]
    #[ignore = "needs a GPU adapter: cargo test -p k4remote -- --ignored gpu_golden"]
    fn fr_pan_12_gpu_draws_nothing_without_span_or_rows() {
        let rig = rig();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut gpu = Gpu::new(&rig.device, &rig.queue, format);
        let empty = PanShared::default();
        let blank = render(&rig, &mut gpu, format, &empty, view());
        assert!(
            blank.chunks(4).all(|p| p[..3] == [0, 0, 0]),
            "empty history draws nothing"
        );
        let mut pan = PanShared::default();
        push_all(&mut pan, &mixed(8, 5));
        let no_span = View {
            span_hz: 0,
            ..view()
        };
        let out = render(&rig, &mut gpu, format, &pan, no_span);
        assert!(
            out.chunks(4).all(|p| p[..3] == [0, 0, 0]),
            "no span, no waterfall"
        );
    }
}

#[cfg(test)]
mod redraw_tests {
    use super::*;

    /// The redraw chain against a simulated display: rows arrive at a steady rate, each redraw
    /// asks for the next one as the state says, and every frame is counted. Returns the frames
    /// drawn in `seconds` of a stream that runs the whole time.
    fn simulate(row_interval: Duration, seconds: u64) -> u64 {
        let t0 = Instant::now();
        let end = Duration::from_secs(seconds);
        let mut st = RedrawState::default();
        // The UI tick's first redraw after rows begin is the chain's start.
        let mut at = row_interval;
        let mut frames = 0;
        while at < end {
            let total = (at.as_nanos() / row_interval.as_nanos()) as u64;
            frames += 1;
            match st.next_frame_at(total, t0 + at) {
                Some(next) => at = next - t0,
                None => break,
            }
        }
        frames
    }

    /// FR-PAN-13: the chain stops when nothing arrives, runs while rows do, ends a grace after the
    /// last, restarts when rows resume, and a cleared history counts as activity.
    /// trace: FR-PAN-13
    #[test]
    fn fr_pan_13_redraw_chain_follows_the_row_stream() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut st = RedrawState::default();

        // Nothing has ever arrived: never spend a frame on it.
        assert_eq!(st.next_frame_at(0, ms(0)), None);
        assert_eq!(st.next_frame_at(0, ms(5_000)), None);

        // Rows arriving: the chain runs, frame after frame, even on frames with no new row.
        assert!(st.next_frame_at(1, ms(10_000)).is_some());
        assert!(
            st.next_frame_at(1, ms(10_016)).is_some(),
            "no new row yet, still in the grace"
        );
        assert!(st.next_frame_at(2, ms(10_033)).is_some());
        assert!(st.next_frame_at(2, ms(10_049)).is_some());

        // The stream stops. The chain outlives the last row by the grace, then ends.
        assert!(
            st.next_frame_at(2, ms(10_033 + 299)).is_some(),
            "just inside the grace"
        );
        assert_eq!(
            st.next_frame_at(2, ms(10_033 + 301)),
            None,
            "quiet for longer than the grace"
        );
        assert_eq!(
            st.next_frame_at(2, ms(60_000)),
            None,
            "and it stays stopped"
        );

        // Rows resume (the UI tick's own redraw notices): the chain restarts.
        assert!(st.next_frame_at(3, ms(60_100)).is_some());

        // A cleared history that starts counting again from a *lower* total also counts as change.
        let mut st = RedrawState::default();
        assert!(st.next_frame_at(500, ms(0)).is_some());
        assert_eq!(st.next_frame_at(500, ms(1_000)), None);
        assert!(
            st.next_frame_at(3, ms(2_000)).is_some(),
            "a changed total, even downwards, is activity"
        );
    }

    /// FR-PAN-13: the `K4_FPS` line counts **window** frames, not pane redraws. Both panes of a
    /// dual view receive the same `now` for one window redraw; counting per pane read a dual view
    /// at twice its real rate (40 against 20 in single view, on the radio, for the same stream).
    /// trace: FR-PAN-13
    #[test]
    fn fr_pan_13_the_frame_count_is_per_window_not_per_pane() {
        let t0 = Instant::now();
        let mut last = None;
        let mut counted = 0;
        for frame in 0..100u64 {
            let now = t0 + Duration::from_millis(50 * frame);
            // Two panes, one window redraw: the same `now` twice.
            for _pane in 0..2 {
                if first_sight(&mut last, now) {
                    counted += 1;
                }
            }
        }
        assert_eq!(
            counted, 100,
            "a dual view must count each window frame once"
        );

        // A single pane is unchanged: every distinct redraw counts.
        let mut last = None;
        let single = (0..100u64)
            .filter(|f| first_sight(&mut last, t0 + Duration::from_millis(50 * f)))
            .count();
        assert_eq!(single, 100);
    }

    /// FR-PAN-13: frames follow the *row* rate, not the display's. A stream of 30 rows a second
    /// is drawn at about 30 frames a second — every row shown, and half the frames of a 60 Hz
    /// chain — a faster stream never exceeds the cap, and a slower one never drops below the UI
    /// tick's 10 Hz.
    /// trace: FR-PAN-13
    #[test]
    fn fr_pan_13_frames_follow_the_row_rate_not_the_display() {
        let ms = Duration::from_millis;
        // 30 rows/s (a 33 ms interval, as the K4 sends): 30 frames/s ± 20 %, over a minute.
        let frames = simulate(ms(33), 60);
        let per_sec = frames as f64 / 60.0;
        assert!(
            (24.0..=36.0).contains(&per_sec),
            "{per_sec:.1} frames/s for 30 rows/s (a 60 Hz chain draws 60)"
        );
        // 20 rows/s and 25 rows/s: the same.
        for (interval, rows) in [(50u64, 20.0), (40, 25.0)] {
            let per_sec = simulate(ms(interval), 60) as f64 / 60.0;
            assert!(
                (rows * 0.8..=rows * 1.2).contains(&per_sec),
                "{per_sec:.1} frames/s for {rows} rows/s"
            );
        }
        // 500 rows/s: capped at one frame per MIN_PERIOD (125/s), not one per row.
        let per_sec = simulate(ms(2), 20) as f64 / 20.0;
        assert!(per_sec <= 126.0, "{per_sec:.1} frames/s exceeds the cap");
        assert!(
            per_sec >= 100.0,
            "{per_sec:.1} frames/s is far below the cap"
        );
        // 4 rows/s: the grace (300 ms) is longer than the gap (250 ms), so the chain runs on at no
        // slower than 10 frames/s — never slower than the UI tick.
        let per_sec = simulate(ms(250), 20) as f64 / 20.0;
        assert!(
            (9.0..=11.0).contains(&per_sec),
            "{per_sec:.1} frames/s for 4 rows/s"
        );
    }

    /// FR-PAN-13: the widget asks for its next frame *at* the time the state gives, not for the
    /// next vsync — the whole gain rests on this line, which no pure test can see. Structural, like
    /// the emergency-stop guard, and reading only the code above this module so the needles below do
    /// not match this test's own text.
    /// trace: FR-PAN-13
    #[test]
    fn fr_pan_13_the_widget_requests_the_time_the_state_gives() {
        let whole = include_str!("waterfall_gpu.rs");
        let code = &whole[..whole
            .find(concat!("mod redraw", "_tests {"))
            .expect("the test module")];
        let start = code
            .find("fn update(")
            .expect("the shader program's update");
        let update = &code[start..start + code[start..].find("fn draw(").expect("draw follows")];
        assert!(
            update.contains("state.next_frame_at(total, now)"),
            "update does not ask the state:\n{update}"
        );
        assert!(
            update.contains("RedrawRequest::At(at)"),
            "update does not request the time it was given:\n{update}"
        );
        assert!(
            !update.contains("RedrawRequest::NextFrame"),
            "update asks for the next vsync, which is the display's rate, not the rows':\n{update}"
        );
        assert!(
            !code.contains("RedrawRequest::NextFrame"),
            "something else in the widget asks for every frame"
        );
        // The `K4_FPS` count goes through the per-window dedupe, and nowhere else.
        assert!(
            update.contains("first_sight(&mut last, now)"),
            "update counts frames without the per-window dedupe:\n{update}"
        );
        assert_eq!(
            code.matches("FRAMES.fetch_add").count(),
            1,
            "the frame counter is incremented in more than one place"
        );
    }

    /// FR-PAN-13: the estimate settles on the rate the stream changes to, ignores the gap when a
    /// stream restarts after silence, and stays inside its bounds whatever it is fed.
    /// trace: FR-PAN-13
    #[test]
    fn fr_pan_13_the_row_interval_is_learned_and_bounded() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut st = RedrawState::default();
        // A steady 20 ms stream, one row per frame: the next frame is asked for ~20 ms ahead.
        let mut next = Duration::ZERO;
        for i in 1..=60u64 {
            let at = ms(i * 20);
            next = st.next_frame_at(i, at).unwrap() - at;
        }
        assert!(
            (17..=23).contains(&(next.as_millis() as u64)),
            "learned {next:?}, wanted ~20 ms"
        );
        // It then slows to 50 ms and the estimate follows.
        let base = 60 * 20;
        for i in 1..=60u64 {
            let at = ms(base + i * 50);
            next = st.next_frame_at(60 + i, at).unwrap() - at;
        }
        assert!(
            (44..=56).contains(&(next.as_millis() as u64)),
            "learned {next:?}, wanted ~50 ms"
        );
        // One odd frame — three rows in a single gap — nudges the estimate, it does not swing it:
        // the smoothing takes in a quarter of a measurement, not all of it.
        let mut smooth = RedrawState::default();
        for i in 1..=40u64 {
            smooth.next_frame_at(i, ms(i * 33));
        }
        let steady = smooth.next_frame_at(41, ms(41 * 33)).unwrap() - ms(41 * 33);
        let burst = smooth.next_frame_at(44, ms(42 * 33)).unwrap() - ms(42 * 33);
        assert!(
            burst.as_secs_f64() > steady.as_secs_f64() * 0.7,
            "one burst moved {steady:?} to {burst:?}"
        );
        assert!(
            burst < steady,
            "a burst still shortens the estimate a little"
        );
        // A long silence, then rows again: the silence is not taken as a row interval.
        let resume = base + 60 * 50 + 30_000;
        let after = st.next_frame_at(121, ms(resume)).unwrap() - ms(resume);
        assert!(
            (44..=56).contains(&(after.as_millis() as u64)),
            "a restart after silence moved the estimate to {after:?}"
        );
        // Bounds: an absurdly fast burst (10 000 rows in one frame) and a slow crawl both stay in
        // [MIN_PERIOD, MAX_PERIOD]; a total that jumps backwards does not panic.
        let mut st = RedrawState::default();
        st.next_frame_at(1, ms(0));
        for i in 1..=100u64 {
            let at = ms(i * 10);
            let d = st.next_frame_at(1 + i * 10_000, at).unwrap() - at;
            assert!((MIN_PERIOD..=MAX_PERIOD).contains(&d), "{d:?}");
        }
        let mut st = RedrawState::default();
        st.next_frame_at(1, ms(0));
        for i in 1..=100u64 {
            let at = ms(i * 250);
            let d = st.next_frame_at(1 + i, at).unwrap() - at;
            assert!((MIN_PERIOD..=MAX_PERIOD).contains(&d), "{d:?}");
        }
        st.next_frame_at(u64::MAX, ms(100_000));
        st.next_frame_at(0, ms(100_010));
        st.next_frame_at(u64::MAX, ms(100_020));
    }
}
