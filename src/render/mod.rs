//! Блоб Моей волны и ореол «Сцены»: шейдер WGSL на видеокарте (wgpu) с откатом на CPU
//! (та же формула в [`blob_pixel`]). Кадр — непредумноженный RGBA для kitty.

use std::time::Instant;

/// Параметры блоба: четыре цвета палитры (0..255), фаза анимации и «энергия» (0..1).
#[derive(Clone, Copy)]
pub struct Blob {
    pub colors: [[f32; 3]; 4],
    pub phase: f32,
    pub energy: f32,
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
            out.extend(blob_px(u, v, aspect, b).map(|c| c as u8));
        }
    }
    out
}

fn hash(x: f32, y: f32) -> f32 {
    ((x * 127.1 + y * 311.7).sin() * 43758.545).rem_euclid(1.0)
}

fn noise(x: f32, y: f32) -> f32 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
    mix(
        mix(hash(ix, iy), hash(ix + 1.0, iy), ux),
        mix(hash(ix, iy + 1.0), hash(ix + 1.0, iy + 1.0), ux),
        uy,
    )
}

fn fbm(x: f32, y: f32) -> f32 {
    0.55 * noise(x, y) + 0.30 * noise(x * 2.03 + 7.1, y * 2.03 + 7.1) + 0.15 * noise(x * 4.01 + 3.7, y * 4.01 + 3.7)
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn palette(c: &[[f32; 3]; 4], x: f32) -> [f32; 3] {
    let s = x.rem_euclid(1.0) * 4.0;
    let i = s.floor() as usize;
    let f = smooth(0.0, 1.0, s.fract());
    let (a, b) = (c[i % 4], c[(i + 1) % 4]);
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * f)
}

/// Пиксель блоба в точке (u, v) ∈ [0,1]² холста с пропорциями `aspect`: RGBA 0..255.
/// Повторяет шейдер blob.wgsl.
pub fn blob_px(u: f32, v: f32, aspect: f32, b: &Blob) -> [f32; 4] {
    let c = b.colors.map(|k| k.map(|x| x / 255.0));
    let (t, e) = (b.phase, b.energy);
    let (mut qx, mut qy) = (u * 2.0 - 1.0, v * 2.0 - 1.0);
    if aspect > 1.0 {
        qx *= aspect;
    } else {
        qy /= aspect;
    }
    let mut f = 0.0;
    for k in 0..5 {
        let fk = k as f32;
        let amp = 0.28 + 0.10 * e;
        let cx = amp * (t * (0.21 + 0.07 * fk) + fk * 1.9).sin();
        let cy = amp * (t * (0.17 + 0.05 * fk) + fk * 2.7).cos();
        let r = 0.36 + 0.10 * (t * 0.33 + fk * 1.3).sin();
        f += (-((qx - cx).powi(2) + (qy - cy).powi(2)) / (r * r)).exp();
    }
    f += (fbm(qx * 2.2 + t * 0.15, qy * 2.2 + t * 0.15) - 0.5) * 0.35;
    let a = smooth(0.42, 0.62, f);
    let w1 = (
        fbm(qx * 1.6 + t * 0.11, qy * 1.6 - t * 0.07),
        fbm(qx * 1.6 + 5.2 - t * 0.09, qy * 1.6 + 1.3 + t * 0.05),
    );
    let w2 = (
        fbm(qx * 1.8 + w1.0 * 2.4 + 1.7, qy * 1.8 + w1.1 * 2.4 + 1.7),
        fbm(qx * 1.8 + w1.0 * 2.4 + 9.2, qy * 1.8 + w1.1 * 2.4 + 9.2),
    );
    let val = fbm(qx * 1.3 + w2.0 * 2.2 + t * 0.04, qy * 1.3 + w2.1 * 2.2 + t * 0.04);
    let bands = 0.5 + 0.5 * (val * 9.0 + t * 0.6 + w1.0 * 4.0).sin();
    let p1 = palette(&c, val * 1.6 + t * 0.03);
    let p2 = palette(&c, val * 1.6 + 0.5 + t * 0.03);
    let col = [0, 1, 2].map(|k| (p1[k] + (p2[k] - p1[k]) * bands * 0.45).clamp(0.0, 1.0) * 255.0);
    [col[0], col[1], col[2], a * 0.95 * 255.0]
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
        for c in b.colors {
            for v in [c[0] / 255.0, c[1] / 255.0, c[2] / 255.0, 1.0] {
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
