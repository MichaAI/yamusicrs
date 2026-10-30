//! Блоб Моей волны и ореол «Сцены»: шейдер WGSL на видеокарте (wgpu) с откатом на CPU
//! (та же формула в [`blob_pixel`]). Кадр — непредумноженный RGBA для kitty.

use std::time::Instant;

/// Параметры блоба: цвета (0..255), фаза анимации и «энергия» (0 — спокойно, 1 — играет).
#[derive(Clone, Copy)]
pub struct Blob {
    pub colors: [[f32; 3]; 2],
    pub phase: f32,
    pub energy: f32,
}

impl Blob {
    /// Три цвета: два заданных и промежуточный между ними.
    fn palette(&self) -> [[f32; 3]; 3] {
        let [a, b] = self.colors.map(|c| c.map(|v| v / 255.0));
        let mid = [0, 1, 2].map(|i| (a[i] + b[i]) * 0.5 * 1.08);
        [a, b, mid.map(|v| v.min(1.0))]
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
            out.extend(blob_pixel_aspect(u, v, aspect, b).map(|c| c as u8));
        }
    }
    out
}

const BALLS: [[f32; 4]; 5] = [
    [0.31, 0.53, 0.0, 0.50],
    [0.47, 0.29, 2.1, 0.43],
    [0.23, 0.41, 4.2, 0.38],
    [0.39, 0.61, 1.3, 0.33],
    [0.17, 0.37, 5.1, 0.30],
];

fn field(q: [f32; 2], t: f32, e: f32, pal: &[[f32; 3]; 3]) -> (f32, [f32; 3]) {
    let (mut f, mut acc) = (0.0, [0.0f32; 3]);
    for (k, b) in BALLS.iter().enumerate() {
        let amp = 0.30 + 0.12 * e;
        let c = [amp * (t * b[0] + b[2]).sin(), amp * (t * b[1] + b[2] * 1.7).cos()];
        let r = b[3] * (0.92 + 0.08 * (t * 0.7 + b[2]).sin());
        let d2 = (q[0] - c[0]).powi(2) + (q[1] - c[1]).powi(2);
        let v = (-d2 / (r * r)).exp();
        f += v;
        let col = pal[[0, 1, 2][k % 3]];
        for i in 0..3 {
            acc[i] += v * col[i];
        }
    }
    (f, acc.map(|a| a / f.max(1e-4)))
}

/// Пиксель блоба в точке (u, v) ∈ [0,1]² для холста с пропорциями `aspect` (w/h): RGBA 0..255.
pub fn blob_pixel_aspect(u: f32, v: f32, aspect: f32, b: &Blob) -> [f32; 4] {
    let pal = b.palette();
    let (t, e) = (b.phase, b.energy);
    let mut q = [u * 2.0 - 1.0, v * 2.0 - 1.0];
    if aspect > 1.0 {
        q[0] *= aspect;
    } else {
        q[1] /= aspect;
    }
    let (f, col) = field(q, t, e, &pal);
    let eps = 0.01;
    let gx = field([q[0] + eps, q[1]], t, e, &pal).0 - f;
    let gy = field([q[0], q[1] + eps], t, e, &pal).0 - f;
    let n = [-gx / eps, -gy / eps, 1.2];
    let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    let l = [-0.5f32, -0.6, 0.6];
    let ll = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
    let light = ((n[0] * l[0] + n[1] * l[1] + n[2] * l[2]) / (nl * ll)).clamp(0.0, 1.0);
    let spec = light.powf(18.0) * 0.18;
    let ss = |a: f32, b: f32, x: f32| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let a = (ss(0.30, 1.05, f).max(ss(0.05, 0.6, f) * 0.25) * 0.9).clamp(0.0, 1.0);
    let rgb = col.map(|c| ((c * (0.78 + 0.32 * light) + spec).clamp(0.0, 1.0)) * 255.0);
    [rgb[0], rgb[1], rgb[2], a * 255.0]
}

/// Совместимость со старыми вызовами (квадратный холст, (u, v) ∈ [-1,1]²).
pub fn blob_pixel(u: f32, v: f32, colors: [[f32; 3]; 2], t: f32) -> [f32; 4] {
    blob_pixel_aspect(
        (u + 1.0) / 2.0,
        (v + 1.0) / 2.0,
        1.0,
        &Blob {
            colors,
            phase: t,
            energy: 0.5,
        },
    )
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
        let pal = b.palette();
        let mut u = Vec::with_capacity(64);
        for c in pal {
            for v in [c[0], c[1], c[2], 1.0] {
                u.extend_from_slice(&v.to_le_bytes());
            }
        }
        for v in [b.phase, b.energy, w as f32 / h.max(1) as f32, 0.0] {
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
