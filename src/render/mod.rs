//! Блоб Моей волны и ореол «Сцены»: шейдер WGSL на видеокарте (wgpu) с откатом на CPU
//! (та же формула в [`blob_pixel`]). Кадр — непредумноженный RGBA для kitty.

use std::time::Instant;

/// Параметры блоба: четыре цвета палитры (0..255), фаза анимации и «энергия» (0..1).
#[derive(Clone, Copy)]
pub struct Blob {
    pub colors: [[f32; 3]; 4],
    pub phase: f32,
    pub energy: f32,
    /// Уровней на канал для дизеринга Байера; 0 — без дизеринга.
    pub dither: f32,
}

impl Blob {
    /// Пастель: цвета разбавлены к светлому и чуть приглушены (0..1).
    fn pastel(&self) -> [[f32; 3]; 4] {
        self.colors.map(|c| {
            let c = c.map(|v| v / 255.0);
            let grey = (c[0] + c[1] + c[2]) / 3.0;
            c.map(|v| {
                let desat = v + (grey - v) * 0.25;
                desat + (1.0 - desat) * 0.3
            })
        })
    }
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

pub struct Gfx {
    gpu: Option<Gpu>,
}

impl Gfx {
    pub fn new(want_gpu: bool) -> Self {
        let t = Instant::now();
        let gpu = if want_gpu { pollster::block_on(init_gpu()) } else { None };
        match &gpu {
            Some(_) => tracing::info!("рендер блоба: GPU ({} мс)", t.elapsed().as_millis()),
            None => tracing::info!("рендер блоба: CPU"),
        }
        Self { gpu }
    }

    pub fn backend(&self) -> &'static str {
        if self.gpu.is_some() { "GPU" } else { "CPU" }
    }

    pub fn blob(&mut self, w: u32, h: u32, b: &Blob) -> Vec<u8> {
        if let Some(g) = &self.gpu
            && let Some(px) = g.render(w, h, b)
        {
            return px;
        }
        cpu_blob(w, h, b)
    }
}

fn cpu_blob(w: u32, h: u32, b: &Blob) -> Vec<u8> {
    let aspect = w as f32 / h.max(1) as f32;
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let v = (y as f32 + 0.5) / h as f32;
            let p = dither(blob_px(u, v, aspect, b), x, y, b.dither);
            out.extend(p.map(|c| c as u8));
        }
    }
    out
}

const BAYER: [f32; 16] = [
    0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0,
];

/// Упорядоченный дизеринг (Байер 4×4): цвет и альфа квантуются до `levels` уровней.
fn dither(p: [f32; 4], x: u32, y: u32, levels: f32) -> [f32; 4] {
    if levels < 2.0 {
        return p;
    }
    let th = (BAYER[((y % 4) * 4 + x % 4) as usize] + 0.5) / 16.0;
    let l = levels - 1.0;
    p.map(|c| ((c / 255.0 * l + th).floor() / l).clamp(0.0, 1.0) * 255.0)
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Шар k: (x, y, радиус) в момент t. 0..4 — основные, 5..6 — капли-спутники.
fn ball(k: usize, t: f32, e: f32) -> [f32; 3] {
    let fk = k as f32;
    if k < 5 {
        let amp = 0.26 + 0.10 * e;
        [
            amp * (t * (0.23 + 0.06 * fk) + fk * 2.1).sin(),
            amp * 0.9 * (t * (0.17 + 0.05 * fk) + fk * 1.3 + 1.2).sin(),
            0.24 + 0.05 * (t * 0.31 + fk * 1.7).sin(),
        ]
    } else {
        let amp = 0.55 + 0.08 * (t * 0.4 + fk).sin();
        let a = t * (0.45 + 0.1 * fk) + fk * 3.0;
        [amp * a.cos(), amp * 0.8 * (a * 1.3).sin(), 0.09]
    }
}

/// Пиксель блоба в точке (u, v) ∈ [0,1]² холста с пропорциями `aspect`: RGBA 0..255.
/// Повторяет шейдер blob.wgsl.
pub fn blob_px(u: f32, v: f32, aspect: f32, b: &Blob) -> [f32; 4] {
    let c = b.pastel();
    let (t, e) = (b.phase, b.energy);
    let (mut qx, mut qy) = (u * 2.0 - 1.0, v * 2.0 - 1.0);
    if aspect > 1.0 {
        qx *= aspect;
    } else {
        qy /= aspect;
    }
    let (mut f, mut wsum, mut acc) = (0.0f32, 0.0f32, [0.0f32; 3]);
    for k in 0..7 {
        let [bx, by, r] = ball(k, t, e);
        let d2 = ((qx - bx).powi(2) + (qy - by).powi(2)).max(1e-4);
        let val = r * r / d2;
        f += val;
        let w = val.powi(3);
        wsum += w;
        for i in 0..3 {
            acc[i] += w * c[k % 4][i];
        }
    }
    let col = acc.map(|x| x / wsum.max(1e-6));
    let body = smooth(0.30, 1.35, f);
    let edge = u.min(1.0 - u).min(v.min(1.0 - v));
    let a = body * 0.94 * smooth(0.0, 0.12, edge);
    let rgb = col.map(|x| x.clamp(0.0, 1.0) * 255.0);
    [rgb[0], rgb[1], rgb[2], a * 255.0]
}

/// Квадратный холст, (u, v) ∈ [-1,1]² — для фолбэка полублоками и подбора контраста.
pub fn blob_pixel(u: f32, v: f32, b: &Blob) -> [f32; 4] {
    blob_px((u + 1.0) / 2.0, (v + 1.0) / 2.0, 1.0, b)
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
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("blob"),
        source: wgpu::ShaderSource::Wgsl(include_str!("blob.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("blob"),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    Some(Gpu {
        device,
        queue,
        pipeline,
        layout,
    })
}

impl Gpu {
    fn render(&self, w: u32, h: u32, b: &Blob) -> Option<Vec<u8>> {
        let mut u = Vec::with_capacity(80);
        for c in b.pastel() {
            for v in [c[0], c[1], c[2], 1.0] {
                u.extend_from_slice(&v.to_le_bytes());
            }
        }
        for v in [b.phase, b.energy, w as f32 / h.max(1) as f32, b.dither] {
            u.extend_from_slice(&v.to_le_bytes());
        }
        let ubuf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&ubuf, 0, &u);
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: ubuf.as_entire_binding(),
            }],
        });
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
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
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
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
            let s = y * row as usize;
            out.extend_from_slice(&data[s..s + (w * 4) as usize]);
        }
        drop(data);
        buf.unmap();
        Some(out)
    }
}
