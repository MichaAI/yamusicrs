//! Рендер векторной графики в RGBA для kitty: блоб/фон и анимированные агенты (Lottie).
//!
//! Сцена строится один раз через трейт `velato::RenderSink`, а рисует её либо GPU
//! (`vello` + `wgpu`, кадр читается обратно из текстуры), либо CPU (`vello_cpu`), если
//! видеокарты нет или она не поднялась.

mod cpu;

use std::time::Instant;

use velato::RenderSink;
use velato::vello::kurbo::{Affine, Circle, Point};
use velato::vello::peniko::{self, Color, Gradient};

pub use velato::Composition;

/// Параметры блоба: два цвета, фаза анимации и «энергия» (0 — спокойно, 1 — играет).
#[derive(Clone, Copy)]
pub struct Blob {
    pub colors: [[f32; 3]; 2],
    pub phase: f32,
    pub energy: f32,
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: velato::vello::Renderer,
}

pub struct Gfx {
    gpu: Option<Gpu>,
    cpu: cpu::Cpu,
    lottie: velato::Renderer,
}

impl Gfx {
    /// Пытается поднять GPU; если не вышло — тихо работает на CPU.
    pub fn new(want_gpu: bool) -> Self {
        let t = Instant::now();
        let gpu = if want_gpu { pollster::block_on(init_gpu()) } else { None };
        match &gpu {
            Some(_) => tracing::info!("рендер: GPU ({} мс)", t.elapsed().as_millis()),
            None => tracing::info!("рендер: CPU (vello_cpu)"),
        }
        Self {
            gpu,
            cpu: cpu::Cpu::default(),
            lottie: velato::Renderer::new(),
        }
    }

    pub fn backend(&self) -> &'static str {
        if self.gpu.is_some() { "GPU" } else { "CPU" }
    }

    pub fn blob(&mut self, w: u32, h: u32, b: &Blob) -> Vec<u8> {
        match &mut self.gpu {
            Some(g) => {
                let mut scene = velato::vello::Scene::new();
                draw_blob(&mut scene, w, h, b);
                g.render(&scene, w, h).unwrap_or_else(|| vec![0; (w * h * 4) as usize])
            }
            None => self.cpu.render(w, h, |s| draw_blob(s, w, h, b)),
        }
    }

    /// Кадр Lottie-анимации, вписанный в квадрат `size`.
    pub fn lottie(&mut self, comp: &Composition, frame: f64, size: u32) -> Vec<u8> {
        let k = size as f64 / comp.width.max(comp.height).max(1) as f64;
        let tf = Affine::scale(k);
        match &mut self.gpu {
            Some(g) => {
                let mut scene = velato::vello::Scene::new();
                self.lottie.append(comp, frame, tf, 1.0, &mut scene);
                g.render(&scene, size, size)
                    .unwrap_or_else(|| vec![0; (size * size * 4) as usize])
            }
            None => {
                let lr = &mut self.lottie;
                self.cpu.render(size, size, |s| lr.append(comp, frame, tf, 1.0, s))
            }
        }
    }
}

/// Мягкая «аура»: несколько больших радиальных градиентов гуляют по кругу и перетекают
/// цветами, сверху — бледный блик. Никаких пикселей: всё векторное и сглаженное.
fn draw_blob(s: &mut impl RenderSink, w: u32, h: u32, b: &Blob) {
    let (w, h) = (w as f64, h as f64);
    let r = w.min(h) * 0.5;
    let c = Point::new(w / 2.0, h / 2.0);
    let t = b.phase as f64;
    let e = 0.6 + 0.4 * b.energy as f64;
    let col = |i: usize, a: f32| {
        let k = b.colors[i];
        Color::from_rgba8(k[0] as u8, k[1] as u8, k[2] as u8, (a * 255.0) as u8)
    };
    // (скорость x, скорость y, фаза, радиус, цвет)
    const BALLS: [(f64, f64, f64, f64, usize); 5] = [
        (0.31, 0.53, 0.0, 0.62, 0),
        (0.47, 0.29, 2.1, 0.55, 1),
        (0.23, 0.41, 4.2, 0.50, 0),
        (0.39, 0.61, 1.3, 0.45, 1),
        (0.17, 0.37, 5.1, 0.40, 0),
    ];
    for (fx, fy, ph, rr, ci) in BALLS {
        let p = Point::new(
            c.x + r * 0.38 * e * (t * fx + ph).sin(),
            c.y + r * 0.38 * e * (t * fy + ph * 1.7).cos(),
        );
        let rad = r * rr * (0.9 + 0.1 * (t * 0.7 + ph).sin());
        let g = Gradient::new_radial(p, rad as f32).with_stops([
            (0.0, col(ci, 0.85)),
            (0.45, col(ci, 0.55)),
            (1.0, col(ci, 0.0)),
        ]);
        s.draw(
            None,
            Affine::IDENTITY,
            &peniko::Brush::Gradient(g),
            &Circle::new(p, rad),
        );
    }
    // Блик: смещается против движения, даёт ощущение объёма.
    let hp = Point::new(
        c.x - r * 0.25 * (t * 0.21).cos(),
        c.y - r * 0.3 + r * 0.1 * (t * 0.33).sin(),
    );
    let hr = r * 0.35;
    let hl = Gradient::new_radial(hp, hr as f32).with_stops([
        (0.0, Color::from_rgba8(255, 255, 255, 70)),
        (1.0, Color::from_rgba8(255, 255, 255, 0)),
    ]);
    s.draw(
        None,
        Affine::IDENTITY,
        &peniko::Brush::Gradient(hl),
        &Circle::new(hp, hr),
    );
}

async fn init_gpu() -> Option<Gpu> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            ..Default::default()
        })
        .await
        .ok()?;
    let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor::default()).await.ok()?;
    let renderer = velato::vello::Renderer::new(
        &device,
        velato::vello::RendererOptions {
            use_cpu: false,
            antialiasing_support: velato::vello::AaSupport::area_only(),
            num_init_threads: std::num::NonZeroUsize::new(1),
            pipeline_cache: None,
        },
    )
    .ok()?;
    Some(Gpu {
        device,
        queue,
        renderer,
    })
}

impl Gpu {
    /// Рендер в текстуру и чтение обратно (vello отдаёт уже непредумноженную альфу).
    fn render(&mut self, scene: &velato::vello::Scene, w: u32, h: u32) -> Option<Vec<u8>> {
        let size = wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        };
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                scene,
                &view,
                &velato::vello::RenderParams {
                    base_color: Color::TRANSPARENT,
                    width: w,
                    height: h,
                    antialiasing_method: velato::vello::AaConfig::Area,
                },
            )
            .ok()?;
        let row = (w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        enc.copy_texture_to_buffer(
            tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            size,
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            let start = y * row as usize;
            out.extend_from_slice(&data[start..start + (w * 4) as usize]);
        }
        drop(data);
        buf.unmap();
        Some(out)
    }
}
