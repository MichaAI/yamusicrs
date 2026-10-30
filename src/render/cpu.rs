//! Откат на CPU: `velato::RenderSink` поверх `vello_cpu`.

use velato::RenderSink;
use velato::model::{ImageAsset, fixed};
use velato::vello::kurbo::{Affine, Shape};
use velato::vello::peniko;
use vello_cpu::{Pixmap, RenderContext, Resources};

#[derive(Default)]
pub struct Cpu {
    ctx: Option<RenderContext>,
    res: Option<Resources>,
}

pub struct Sink<'a>(&'a mut RenderContext);

impl Cpu {
    /// Рисует сцену и возвращает непредумноженный RGBA (так ждёт kitty).
    pub fn render(&mut self, w: u32, h: u32, draw: impl FnOnce(&mut Sink<'_>)) -> Vec<u8> {
        let (w16, h16) = (w as u16, h as u16);
        let ctx = match &mut self.ctx {
            Some(c) if c.width() == w16 && c.height() == h16 => {
                c.reset();
                c
            }
            slot => slot.insert(RenderContext::new(w16, h16)),
        };
        draw(&mut Sink(ctx));
        ctx.flush();
        let mut pm = Pixmap::new(w16, h16);
        ctx.render(&mut pm, self.res.get_or_insert_with(Resources::new));
        let mut out = pm.data_as_u8_slice().to_vec();
        for px in out.chunks_exact_mut(4) {
            let a = px[3] as u32;
            if a > 0 && a < 255 {
                for c in &mut px[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        out
    }
}

impl RenderSink for Sink<'_> {
    fn push_layer(&mut self, blend: impl Into<peniko::BlendMode>, alpha: f32, transform: Affine, shape: &impl Shape) {
        self.0.set_transform(transform);
        let path = shape.to_path(0.1);
        self.0
            .push_layer(Some(&path), Some(blend.into()), Some(alpha), None, None);
    }

    fn push_clip_layer(&mut self, transform: Affine, shape: &impl Shape) {
        self.0.set_transform(transform);
        self.0.push_clip_layer(&shape.to_path(0.1));
    }

    fn pop_layer(&mut self) {
        self.0.pop_layer();
    }

    fn draw(&mut self, stroke: Option<&fixed::Stroke>, transform: Affine, brush: &fixed::Brush, shape: &impl Shape) {
        self.0.set_transform(transform);
        let paint: vello_cpu::PaintType = match brush {
            peniko::Brush::Solid(c) => (*c).into(),
            peniko::Brush::Gradient(g) => g.clone().into(),
            peniko::Brush::Image(_) => return,
        };
        self.0.set_paint(paint);
        let path = shape.to_path(0.1);
        match stroke {
            Some(s) => {
                self.0.set_stroke(s.clone());
                self.0.stroke_path(&path);
            }
            None => self.0.fill_path(&path),
        }
    }

    fn draw_image(&mut self, _image: &ImageAsset, _transform: Affine, _alpha: f64) {}
}
