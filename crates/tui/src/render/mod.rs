//! Блоб Моей волны и ореол «Сцены»: шейдер WGSL на видеокарте (wgpu) с откатом на CPU
//! (та же формула в [`blob_pixel`]). Кадр — непредумноженный RGBA для kitty.

use std::time::Instant;

/// Параметры блоба: четыре цвета палитры (0..255), фаза анимации и «энергия» (0..1).
#[derive(Clone, Copy)]
pub struct Blob {
    pub colors: [[f32; 3]; 4],
    pub phase: f32,
    pub energy: f32,
    /// Сила дизеринга в ступенях 8 бит; 0 — без дизеринга.
    pub dither: f32,
    /// Столбики спектра поверх блоба (тот же проход шейдера).
    pub bars: Option<Bars>,
    /// Без блоба: только столбики.
    pub bars_only: bool,
}

/// Число столбиков спектра.
pub const BARS: usize = 32;

/// Спектр в полосе кадра: `rect` = (x0, y0, x1, y1) в долях кадра, `levels` 0..1.
#[derive(Clone, Copy)]
pub struct Bars {
    pub rect: [f32; 4],
    pub levels: [f32; BARS],
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
            None if want_gpu => tracing::warn!("рендер блоба: CPU — GPU не поднялся (см. выше)"),
            None => tracing::info!("рендер блоба: CPU (GPU выключен или терминал без kitty-графики)"),
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
    let bs = balls(b.phase, b.energy);
    for y in 0..h {
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let v = (y as f32 + 0.5) / h as f32;
            let mut p = if b.bars_only { [0.0; 4] } else { blob_px(u, v, aspect, b, &bs) };
            if let Some(bars) = &b.bars {
                p = with_bars(p, u, v, aspect, h as f32, bars, &b.pastel(), !b.bars_only);
            }
            let p = dither(p, x, y, b.dither);
            out.extend(p.map(|c| c as u8));
        }
    }
    out
}

/// Расстояние до столбика `i` (скруглённая капсула), `dy` сдвигает его вниз — для тени.
fn bar_sd(u: f32, v: f32, aspect: f32, b: &Bars, i: usize, dy: f32) -> f32 {
    let [x0, y0, x1, y1] = b.rect;
    let slot = (x1 - x0) / BARS as f32;
    let bw = slot * aspect * 0.62;
    let cx = x0 + (i as f32 + 0.5) * slot;
    let hgt = (b.levels[i] * (y1 - y0)).max(bw);
    let (dx, dy) = ((u - cx) * aspect, v - dy - (y1 - hgt * 0.5));
    let q = (dx.abs() - bw * 0.5 + bw * 0.5, dy.abs() - hgt * 0.5 + bw * 0.5);
    (q.0.max(0.0).powi(2) + q.1.max(0.0).powi(2)).sqrt() + q.0.max(q.1).min(0.0) - bw * 0.5
}

/// Накладывает столбик спектра (и тень под ним, если `shadow`) на пиксель блоба — как в blob.wgsl.
#[allow(clippy::too_many_arguments)]
fn with_bars(mut px: [f32; 4], u: f32, v: f32, aspect: f32, h: f32, b: &Bars, c: &[[f32; 3]; 4], shadow: bool) -> [f32; 4] {
    let [x0, y0, x1, y1] = b.rect;
    let slot = (x1 - x0) / BARS as f32;
    let bw = slot * aspect * 0.62;
    let xu = (u - x0) / slot;
    if !(0.0..BARS as f32).contains(&xu) || v < y0 || v > y1 + bw * 0.5 {
        return px;
    }
    let i = xu.floor() as usize;
    if shadow {
        let ds = bar_sd(u, v, aspect, b, i, bw * 0.18);
        let sh = (1.0 - smooth(-bw * 0.1, bw * 0.28, ds)) * 0.35;
        let a = px[3] / 255.0;
        let sa = sh + a * (1.0 - sh);
        for k in 0..3 {
            px[k] = px[k] * a * (1.0 - sh) / sa.max(1e-6);
        }
        px[3] = sa * 255.0;
    }
    let cov = (0.5 - bar_sd(u, v, aspect, b, i, 0.0) * h).clamp(0.0, 1.0) * 0.9;
    if cov <= 0.0 {
        return px;
    }
    let t = ((y1 - v) / (y1 - y0)).clamp(0.0, 1.0);
    let col = |k: usize| {
        let m = c[1][k] + (c[2][k] - c[1][k]) * t;
        (m + (1.0 - m) * 0.2) * 255.0
    };
    let a = px[3] / 255.0;
    let ra = cov + a * (1.0 - cov);
    let mix = |k: usize| (col(k) * cov + px[k] * a * (1.0 - cov)) / ra.max(1e-6);
    [mix(0), mix(1), mix(2), ra * 255.0]
}

/// Дизеринг против «колец» на градиентах: шум в `amp` ступеней 8 бит
/// (interleaved gradient noise — тот же, что в шейдере).
fn dither(p: [f32; 4], x: u32, y: u32, amp: f32) -> [f32; 4] {
    if amp <= 0.0 {
        return p;
    }
    let ign = (52.9829189 * (0.06711056 * (x as f32 + 0.5) + 0.00583715 * (y as f32 + 0.5)).fract()).fract();
    let ign2 = (ign + 0.5).fract();
    let n = [ign, ign, ign, ign2];
    [0, 1, 2, 3].map(|i| (p[i] + (n[i] - 0.5) * amp).clamp(0.0, 255.0))
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

/// Расталкивание сдвигает шар не дальше этого (в координатах шейдера).
const PUSH_MAX: f32 = 0.1;

/// Шары кадра: (x, y, r) × 7. Основные пять мягко расталкиваются, чтобы не сходились почти
/// вплотную. Одни на весь кадр — считаются раз, а не в шейдере на каждый пиксель.
///
/// Расталкивание — гладкая функция положений без памяти, поэтому и без рывков:
/// - все сдвиги считаются от исходных положений разом (а не по очереди с уже сдвинутыми
///   соседями: там результат зависел от порядка шаров и дёргался, когда сходились трое);
/// - сила пары плавно уходит в ноль, когда центры совпадают, так что вектор сдвига
///   непрерывен и при проходе шаров друг сквозь друга не переворачивается скачком
///   (за кадр шар сдвигается не больше чем на ~0,005; раньше бывало до 0,4 — пол-блоба);
/// - общий сдвиг насыщается плавно (`tanh`), а не обрезается.
/// Та же формула — в `Blob.kt` (Android).
pub fn balls(t: f32, e: f32) -> [[f32; 3]; 7] {
    let base: [[f32; 3]; 7] = std::array::from_fn(|k| ball(k, t, e));
    let mut bs = base;
    for i in 0..5 {
        let mut push = [0.0f32; 2];
        for j in 0..5 {
            if i == j {
                continue;
            }
            let (dx, dy) = (base[i][0] - base[j][0], base[i][1] - base[j][1]);
            let l = (dx * dx + dy * dy).sqrt();
            let m = 0.75 * (base[i][2] + base[j][2]);
            if l < m {
                let s = l / m;
                // Сила пары m/2 · (1 − s)² · s²(3 − 2s): спад к границе касания и плавный ноль при
                // совпадении центров. Вектор = (dx, dy)/l · сила, одно s сокращается с l = s·m.
                let k = 0.5 * (1.0 - s) * (1.0 - s) * s * (3.0 - 2.0 * s);
                push[0] += dx * k;
                push[1] += dy * k;
            }
        }
        let l = (push[0] * push[0] + push[1] * push[1]).sqrt();
        if l > 1e-6 {
            let k = PUSH_MAX * (l / PUSH_MAX).tanh() / l;
            bs[i][0] += push[0] * k;
            bs[i][1] += push[1] * k;
        }
    }
    bs
}

/// Пиксель блоба в точке (u, v) ∈ [0,1]² холста с пропорциями `aspect`: RGBA 0..255.
/// Повторяет шейдер blob.wgsl; `bs` — шары кадра из [`balls`].
pub fn blob_px(u: f32, v: f32, aspect: f32, b: &Blob, bs: &[[f32; 3]; 7]) -> [f32; 4] {
    let c = b.pastel();
    let (mut qx, mut qy) = (u * 2.0 - 1.0, v * 2.0 - 1.0);
    if aspect > 1.0 {
        qx *= aspect;
    } else {
        qy /= aspect;
    }
    let (mut f, mut wsum, mut acc) = (0.0f32, 0.0f32, [0.0f32; 3]);
    for (k, &[bx, by, r]) in bs.iter().enumerate() {
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
    blob_px((u + 1.0) / 2.0, (v + 1.0) / 2.0, 1.0, b, &balls(b.phase, b.energy))
}

async fn init_gpu() -> Option<Gpu> {
    // Только Vulkan: GL-бэкенд тянет EGL и gallium с LLVM (~40 МБ памяти) ради того же шейдера.
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(desc.with_env());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            ..Default::default()
        })
        .await
        .inspect_err(|e| {
            tracing::warn!("GPU: нет Vulkan-адаптера ({e}); нужен libvulkan.so.1 в rpath — собирай через flake")
        })
        .ok()?;
    let info = adapter.get_info();
    // Программный рендер (lavapipe) ничем не лучше своего CPU-пути.
    if info.device_type == wgpu::DeviceType::Cpu {
        tracing::warn!("GPU: доступен только программный {}", info.name);
        return None;
    }
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .inspect_err(|e| tracing::warn!("GPU: {}: {e}", info.name))
        .ok()?;
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
        let mut u = Vec::with_capacity(352);
        for c in b.pastel() {
            for v in [c[0], c[1], c[2], 1.0] {
                u.extend_from_slice(&v.to_le_bytes());
            }
        }
        for v in [b.phase, b.energy, w as f32 / h.max(1) as f32, b.dither] {
            u.extend_from_slice(&v.to_le_bytes());
        }
        for [x, y, r] in balls(b.phase, b.energy) {
            for v in [x, y, r, 0.0] {
                u.extend_from_slice(&v.to_le_bytes());
            }
        }
        let r = b.bars.map_or([0.0; 4], |x| x.rect);
        for v in r {
            u.extend_from_slice(&v.to_le_bytes());
        }
        let lv = b.bars.map_or([0.0; BARS], |x| x.levels);
        for v in lv {
            u.extend_from_slice(&v.to_le_bytes());
        }
        let shadow = b.bars.is_some() && !b.bars_only;
        for v in [if b.bars_only { 1.0f32 } else { 0.0 }, if shadow { 1.0 } else { 0.0 }, 0.0, 0.0] {
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
