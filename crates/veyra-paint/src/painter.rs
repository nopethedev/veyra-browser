//! Paints a layout tree onto a canvas: backgrounds, gradients, borders,
//! shadows, images, text, form controls, clipping, scrolling, opacity and
//! 2D transforms.

use std::collections::HashMap;

use veyra_common::{Color, Point, Rect, Size};
use veyra_css::style::{BackgroundClip, BackgroundRepeat, BackgroundSize, Transform2D};
use veyra_css::style::TextDecoration;
use veyra_css::values::{BorderStyle, Overflow, Position, TextAlign};
use veyra_dom::NodeId;
use veyra_layout::{FormControlKind, FormInfo, LayoutBox, LayoutTree, TextFragment, TextMeasure};

use crate::canvas::Canvas;
use crate::fonts::FontSystem;
use crate::images::{Image, ImageCache};

pub struct PaintConfig<'a> {
    pub tree: &'a LayoutTree,
    pub fonts: &'a FontSystem,
    pub images: &'a ImageCache,
    pub viewport: Size,
    pub scroll: Point,
    pub scale: f32,
    pub background: Color,
}

impl<'a> PaintConfig<'a> {
    pub fn new(
        tree: &'a LayoutTree,
        fonts: &'a FontSystem,
        images: &'a ImageCache,
        viewport: Size,
    ) -> PaintConfig<'a> {
        PaintConfig {
            tree,
            fonts,
            images,
            viewport,
            scroll: Point::ZERO,
            scale: 1.0,
            background: Color::rgb(255, 255, 255),
        }
    }
}

fn has_background(b: &LayoutBox) -> bool {
    b.style.background_color.a > 0
        || b.style.background_gradient.is_some()
        || b.style.background_image.is_some()
}

pub fn paint_page(cfg: &PaintConfig) -> Canvas {
    let scale = cfg.scale.clamp(0.25, 4.0);
    let width = (cfg.viewport.w * scale).ceil().max(1.0) as usize;
    let height = (cfg.viewport.h * scale).ceil().max(1.0) as usize;
    let mut canvas = Canvas::new(width, height);

    // Propagate the body background to the canvas when the root has none.
    let mut background = cfg.background;
    if let Some(root) = &cfg.tree.root {
        if root.style.background_color.a == 0 {
            if let Some(body) = root.children.iter().find(|b| b.tag == "body") {
                if body.style.background_color.a > 0 {
                    background = body.style.background_color;
                }
            }
        }
    }
    canvas.fill(background);

    let mut texts: HashMap<Option<NodeId>, Vec<&TextFragment>> = HashMap::new();
    for t in &cfg.tree.texts {
        texts.entry(t.owner).or_default().push(t);
    }

    let mut painter = Painter {
        canvas,
        fonts: cfg.fonts,
        images: cfg.images,
        scale,
        scroll: cfg.scroll,
        offset: Point::ZERO,
        clip: vec![Rect::new(0.0, 0.0, width as f32, height as f32)],
        texts,
        all_texts: &cfg.tree.texts,
        layer_budget: 6,
    };
    if let Some(root) = &cfg.tree.root {
        painter.paint_box(root, false);
    }
    painter.canvas
}

struct Painter<'a> {
    canvas: Canvas,
    fonts: &'a FontSystem,
    images: &'a ImageCache,
    scale: f32,
    scroll: Point,
    offset: Point,
    clip: Vec<Rect>,
    texts: HashMap<Option<NodeId>, Vec<&'a TextFragment>>,
    all_texts: &'a [TextFragment],
    layer_budget: u32,
}

impl<'a> Painter<'a> {
    #[inline]
    fn map(&self, r: Rect) -> Rect {
        Rect::new(
            (r.x + self.offset.x - self.scroll.x) * self.scale,
            (r.y + self.offset.y - self.scroll.y) * self.scale,
            r.w * self.scale,
            r.h * self.scale,
        )
    }

    #[inline]
    fn clip_rect(&self) -> Rect {
        *self.clip.last().unwrap_or(&self.canvas.bounds())
    }

    fn paint_box(&mut self, b: &LayoutBox, skip_layer: bool) {
        let clip = self.clip_rect();
        let mapped = self.map(b.rect);
        let own_texts = b
            .node
            .and_then(|n| self.texts.get(&Some(n)))
            .is_some_and(|v| !v.is_empty());
        if !mapped.intersects(&clip) && b.children.is_empty() && b.image.is_none() && !own_texts {
            return;
        }

        let fixed = b.style.position == Position::Fixed;
        let saved_scroll = if fixed {
            let s = self.scroll;
            self.scroll = Point::ZERO;
            Some(s)
        } else {
            None
        };

        let wants_layer = b.style.opacity < 0.999 || b.style.transform.is_some();
        if !skip_layer && wants_layer && self.layer_budget > 0 {
            self.paint_layer(b);
            if let Some(s) = saved_scroll {
                self.scroll = s;
            }
            return;
        }

        if b.clip {
            self.clip.push(self.map(b.padding_box()).intersection(&clip));
        }

        self.paint_shadows(b);
        self.paint_background(b);
        self.paint_borders(b);

        if let Some(form) = b.form.clone() {
            self.paint_form(b, &form);
        }

        if let Some(url) = &b.image {
            if let Some(img) = self.images.get(url) {
                let dst = self.map(b.content);
                let radius = b.style.border_radius * self.scale;
                let clip = self.clip_rect();
                let scaled = dst.w < img.width as f32 || dst.h < img.height as f32;
                self.canvas
                    .draw_image(&img, dst, clip, radius, b.style.opacity, scaled);
            }
        }

        self.paint_children(b);

        if let Some(node) = b.node {
            let frags = self.texts.get(&Some(node)).cloned().unwrap_or_default();
            if b.style.background_clip == BackgroundClip::Text && has_background(b) {
                self.paint_clipped_text(b, &frags);
            } else {
                for t in frags {
                    self.paint_text(t);
                }
            }
        }

        if b.clip {
            self.clip.pop();
        }
        if let Some(s) = saved_scroll {
            self.scroll = s;
        }
    }

    fn paint_children(&mut self, b: &LayoutBox) {
        let mut negative = Vec::new();
        let mut normal = Vec::new();
        let mut positive = Vec::new();
        for c in &b.children {
            match c.style.position {
                Position::Static => normal.push(c),
                Position::Relative | Position::Sticky if c.style.z_index == 0 => normal.push(c),
                _ => {
                    if c.style.z_index < 0 {
                        negative.push(c);
                    } else {
                        positive.push(c);
                    }
                }
            }
        }
        negative.sort_by_key(|c| c.style.z_index);
        positive.sort_by_key(|c| c.style.z_index);
        for c in negative {
            self.paint_box(c, false);
        }
        for c in normal {
            self.paint_box(c, false);
        }
        for c in positive {
            self.paint_box(c, false);
        }
    }

    fn paint_shadows(&mut self, b: &LayoutBox) {
        let clip = self.clip_rect();
        for sh in &b.style.box_shadows {
            if sh.inset {
                continue;
            }
            let base = Rect::new(
                b.rect.x + sh.x - sh.spread,
                b.rect.y + sh.y - sh.spread,
                (b.rect.w + sh.spread * 2.0).max(1.0),
                (b.rect.h + sh.spread * 2.0).max(1.0),
            );
            let radius = (b.style.border_radius + sh.spread).max(0.0) * self.scale;
            let blur = sh.blur * self.scale;
            let mapped = self.map(base);
            if blur <= 0.5 {
                self.canvas.fill_round_rect(mapped, radius, sh.color, clip);
            } else {
                let steps = 6;
                for i in 0..steps {
                    let grow = blur * (1.0 - i as f32 / steps as f32) * 0.9;
                    let r = Rect::new(base.x - grow, base.y - grow, base.w + grow * 2.0, base.h + grow * 2.0);
                    let alpha = (sh.color.a as f32 * 0.32).clamp(0.0, 255.0) as u8;
                    let color = Color::rgba(sh.color.r, sh.color.g, sh.color.b, alpha);
                    let mapped = self.map(r);
                    self.canvas
                        .fill_round_rect(mapped, radius + grow * self.scale, color, clip);
                }
            }
        }
    }

    fn paint_background(&mut self, b: &LayoutBox) {
        // `background-clip: text` is deferred to the text pass, where the
        // background is masked by the glyphs instead of filling the box.
        if b.style.background_clip == BackgroundClip::Text {
            return;
        }
        self.paint_background_geometry(b);
    }

    fn paint_background_geometry(&mut self, b: &LayoutBox) {
        let rect = self.map(b.rect);
        let radius = b.style.border_radius * self.scale;
        let clip = self.clip_rect();
        if b.style.background_color.a > 0 {
            self.canvas
                .fill_round_rect(rect, radius, b.style.background_color, clip);
        }
        if let Some((angle, stops)) = &b.style.background_gradient {
            self.canvas
                .fill_gradient(rect, *angle, stops, clip, radius);
        }
        if let Some(url) = &b.style.background_image {
            if let Some(img) = self.images.get(url) {
                self.paint_background_image(b, &img, radius);
            }
        }
    }

    fn paint_background_image(&mut self, b: &LayoutBox, img: &Image, radius: f32) {
        let avail_w = b.rect.w.max(1.0);
        let avail_h = b.rect.h.max(1.0);
        let (iw0, ih0) = (img.width as f32, img.height as f32);
        if iw0 <= 0.0 || ih0 <= 0.0 {
            return;
        }
        let (mut iw, mut ih) = match b.style.background_size {
            BackgroundSize::Cover => {
                let s = (avail_w / iw0).max(avail_h / ih0);
                (iw0 * s, ih0 * s)
            }
            BackgroundSize::Contain => {
                let s = (avail_w / iw0).min(avail_h / ih0);
                (iw0 * s, ih0 * s)
            }
            BackgroundSize::Explicit(w, h) => {
                let wv = w.resolve(avail_w);
                let hv = h.resolve(avail_h);
                match (wv, hv) {
                    (Some(w), Some(h)) => (w, h),
                    (Some(w), None) => (w, w * ih0 / iw0),
                    (None, Some(h)) => (h * iw0 / ih0, h),
                    (None, None) => (iw0, ih0),
                }
            }
            BackgroundSize::Auto => (iw0, ih0),
        };
        if iw <= 0.0 || ih <= 0.0 {
            return;
        }
        if b.style.background_size == BackgroundSize::Contain {
            iw = iw.min(avail_w);
            ih = ih.min(avail_h);
        }
        let px = b
            .style
            .background_position
            .0
            .resolve(avail_w - iw)
            .unwrap_or(0.0)
            .clamp(-avail_w, avail_w);
        let py = b
            .style
            .background_position
            .1
            .resolve(avail_h - ih)
            .unwrap_or(0.0)
            .clamp(-avail_h, avail_h);

        let repeat_x = matches!(
            b.style.background_repeat,
            BackgroundRepeat::Repeat | BackgroundRepeat::RepeatX
        );
        let repeat_y = matches!(
            b.style.background_repeat,
            BackgroundRepeat::Repeat | BackgroundRepeat::RepeatY
        );
        let box_clip = self.map(b.rect).intersection(&self.clip_rect());
        if !repeat_x && !repeat_y {
            let dst = self.map(Rect::new(b.rect.x + px, b.rect.y + py, iw, ih));
            let clip = self.clip_rect();
            self.canvas
                .draw_image(img, dst, clip, radius, 1.0, true);
            return;
        }
        let mut tiles = 0;
        let mut y = py;
        while y < avail_h && tiles < 400 {
            let mut x = px;
            while x < avail_w && tiles < 400 {
                let dst = self.map(Rect::new(b.rect.x + x, b.rect.y + y, iw, ih));
                self.canvas.draw_image(img, dst, box_clip, 0.0, 1.0, true);
                tiles += 1;
                if !repeat_x {
                    break;
                }
                x += iw;
            }
            if !repeat_y {
                break;
            }
            y += ih;
        }
    }

    fn paint_borders(&mut self, b: &LayoutBox) {
        let clip = self.clip_rect();
        let rect = self.map(b.rect);
        let widths = [
            b.border.top * self.scale,
            b.border.right * self.scale,
            b.border.bottom * self.scale,
            b.border.left * self.scale,
        ];
        if widths.iter().all(|w| *w <= 0.0) {
            return;
        }
        let radius = b.style.border_radius * self.scale;
        if radius > 0.5 {
            let max = widths.iter().cloned().fold(0.0, f32::max);
            self.canvas
                .stroke_round_rect(rect, radius, max, b.style.border_color[0], clip);
            return;
        }
        for side in 0..4 {
            let width = widths[side];
            let style = b.style.border_style[side];
            if width <= 0.0 || matches!(style, BorderStyle::None | BorderStyle::Hidden) {
                continue;
            }
            let color = b.style.border_color[side];
            let r = match side {
                0 => Rect::new(rect.x, rect.y, rect.w, width),
                1 => Rect::new(rect.right() - width, rect.y, width, rect.h),
                2 => Rect::new(rect.x, rect.bottom() - width, rect.w, width),
                _ => Rect::new(rect.x, rect.y, width, rect.h),
            };
            match style {
                BorderStyle::Dashed => {
                    let along_x = side == 0 || side == 2;
                    let dash = (width * 3.0).max(4.0);
                    let gap = (width * 2.0).max(4.0);
                    let total = if along_x { r.w } else { r.h };
                    let mut t = 0.0;
                    while t < total {
                        let len = dash.min(total - t);
                        let d = if along_x {
                            Rect::new(r.x + t, r.y, len, r.h)
                        } else {
                            Rect::new(r.x, r.y + t, r.w, len)
                        };
                        self.canvas.fill_rect(d, color, clip);
                        t += dash + gap;
                    }
                }
                BorderStyle::Dotted => {
                    let along_x = side == 0 || side == 2;
                    let step = (width * 2.0).max(3.0);
                    let total = if along_x { r.w } else { r.h };
                    let mut t = 0.0;
                    while t < total {
                        let (cx, cy) = if along_x {
                            (r.x + t + width / 2.0, r.y + r.h / 2.0)
                        } else {
                            (r.x + r.w / 2.0, r.y + t + width / 2.0)
                        };
                        self.canvas.fill_circle(cx, cy, width / 2.0, color, clip);
                        t += step;
                    }
                }
                BorderStyle::Double => {
                    let w = (width / 3.0).max(1.0);
                    let inner = match side {
                        0 => Rect::new(r.x, r.bottom() - w, r.w, w),
                        1 => Rect::new(r.x, r.y, w, r.h),
                        2 => Rect::new(r.x, r.y, r.w, w),
                        _ => Rect::new(r.right() - w, r.y, w, r.h),
                    };
                    self.canvas.fill_rect(r, color, clip);
                    self.canvas.fill_rect(inner, color, clip);
                }
                BorderStyle::Inset | BorderStyle::Outset | BorderStyle::Groove | BorderStyle::Ridge => {
                    let light = color.mix(Color::WHITE, 0.35);
                    let dark = color.mix(Color::BLACK, 0.35);
                    let (a, b2) = if matches!(style, BorderStyle::Inset | BorderStyle::Groove) {
                        (dark, light)
                    } else {
                        (light, dark)
                    };
                    let half = if matches!(style, BorderStyle::Double | BorderStyle::Groove | BorderStyle::Ridge) {
                        width / 2.0
                    } else {
                        width
                    };
                    self.canvas.fill_rect(r, color, clip);
                    let inner = match side {
                        0 => Rect::new(r.x, r.y, r.w, half),
                        1 => Rect::new(r.right() - half, r.y, half, r.h),
                        2 => Rect::new(r.x, r.bottom() - half, r.w, half),
                        _ => Rect::new(r.x, r.y, half, r.h),
                    };
                    self.canvas.fill_rect(inner, a, clip);
                    if half < width {
                        let other = match side {
                            0 => Rect::new(r.x, r.y + half, r.w, width - half),
                            1 => Rect::new(r.right() - width, r.y, width - half, r.h),
                            2 => Rect::new(r.x, r.bottom() - width, r.w, width - half),
                            _ => Rect::new(r.x, r.y, width - half, r.h),
                        };
                        self.canvas.fill_rect(other, b2, clip);
                    }
                }
                _ => self.canvas.fill_rect(r, color, clip),
            }
        }
    }

    /// Paint a box's background through its own text (CSS `background-clip:
    /// text`), which is how gradient headline text is usually built.
    fn paint_clipped_text(&mut self, b: &LayoutBox, frags: &[&TextFragment]) {
        if frags.is_empty() {
            return;
        }
        let clip = self.clip_rect();
        let mapped = self.map(b.rect);
        let area = mapped.intersection(&clip);
        if area.w < 1.0 || area.h < 1.0 {
            return;
        }
        let pw = area.w.ceil() as usize;
        let ph = area.h.ceil() as usize;
        if pw == 0 || ph == 0 || pw.saturating_mul(ph) > 20_000_000 {
            return;
        }
        let local = Rect::new(0.0, 0.0, pw as f32, ph as f32);
        let saved_canvas = std::mem::replace(&mut self.canvas, Canvas::transparent(pw, ph));
        let saved_clip = std::mem::replace(&mut self.clip, vec![local]);
        let saved_offset = self.offset;
        let saved_scroll = self.scroll;
        self.offset = Point::new(
            self.offset.x - self.scroll.x - area.x / self.scale,
            self.offset.y - self.scroll.y - area.y / self.scale,
        );
        self.scroll = Point::ZERO;

        // 1. The box's background, in the same local space as the mask.
        self.paint_background_geometry(b);
        let bg = std::mem::replace(&mut self.canvas, saved_canvas);

        // 2. Glyph mask, drawn white so only its coverage matters.
        let page_canvas = std::mem::replace(&mut self.canvas, Canvas::transparent(pw, ph));
        for t in frags {
            let mut white = (*t).clone();
            white.color = Color::WHITE;
            white.decoration_color = None;
            self.paint_text(&white);
        }
        let mask = std::mem::replace(&mut self.canvas, page_canvas);
        self.offset = saved_offset;
        self.scroll = saved_scroll;
        self.clip = saved_clip;

        // 3. Composite: background colour where the glyphs are.
        let ox = area.x.floor() as i32;
        let oy = area.y.floor() as i32;
        for y in 0..ph {
            for x in 0..pw {
                let m = mask.get(x, y);
                if m.a == 0 {
                    continue;
                }
                let c = bg.get(x, y);
                if c.a == 0 {
                    continue;
                }
                let alpha = ((c.a as u32 * m.a as u32) / 255) as u8;
                self.canvas.blend(
                    ox + x as i32,
                    oy + y as i32,
                    Color::rgba(c.r, c.g, c.b, alpha),
                    1.0,
                );
            }
        }
    }

    fn paint_text(&mut self, t: &TextFragment) {
        let clip = self.clip_rect();
        let x = (t.rect.x + self.offset.x - self.scroll.x) * self.scale;
        let baseline = (t.baseline + self.offset.y - self.scroll.y) * self.scale;
        let mut spec = t.font.clone();
        spec.size *= self.scale;
        spec.letter_spacing *= self.scale;
        if spec.size < 1.0 {
            return;
        }
        let mut pen = x;
        for ch in t.text.chars() {
            if ch == '\n' {
                continue;
            }
            let g = self.fonts.glyph(&spec, ch);
            let gx = (pen + g.xmin as f32).round() as i32;
            let gy = (baseline - g.ymin as f32 - g.height as f32).round() as i32;
            self.canvas.blit_glyph(
                &g.bitmap,
                g.width,
                g.height,
                gx,
                gy,
                t.color,
                clip,
                t.opacity,
            );
            pen += g.advance + spec.letter_spacing;
        }
        let width = ((pen - x) / self.scale).max(t.rect.w);
        let color = t.decoration_color.unwrap_or(t.color);
        let thickness = (spec.size / 14.0).max(1.0);
        let mut line = |offset: f32| {
            let y = (baseline + offset).round();
            self.canvas.fill_rect(
                Rect::new(x, y, width * self.scale, thickness),
                color,
                clip,
            );
        };
        if t.decoration == TextDecoration::Underline {
            line(spec.size * 0.14);
        }
        if t.decoration == TextDecoration::Overline {
            line(-spec.size * 0.8);
        }
        if t.decoration == TextDecoration::LineThrough {
            line(-spec.size * 0.3);
        }
    }

    fn paint_form(&mut self, b: &LayoutBox, form: &FormInfo) {
        let clip = self.clip_rect();
        let content = self.map(b.content);
        let spec = {
            let mut f = veyra_layout::FontSpec {
                family: b.style.font_family.clone(),
                size: b.style.font_size * self.scale,
                weight: b.style.font_weight,
                italic: b.style.font_style == veyra_css::style::FontStyle::Italic,
                letter_spacing: b.style.letter_spacing * self.scale,
            };
            if f.size < 1.0 {
                f.size = 13.0 * self.scale;
            }
            f
        };
        match form.kind {
            FormControlKind::Checkbox => {
                let size = 13.0 * self.scale;
                let x = content.x + 1.0;
                let y = content.y + (content.h - size) / 2.0;
                let r = Rect::new(x, y, size, size);
                let border = if b.style.border_color[0].a > 0 {
                    b.style.border_color[0]
                } else {
                    Color::rgb(118, 118, 118)
                };
                self.canvas.fill_rect(r, Color::WHITE, clip);
                self.canvas.stroke_round_rect(r, 2.0 * self.scale, 1.0 * self.scale, border, clip);
                if form.checked {
                    let pen = 2.0 * self.scale;
                    self.canvas.draw_line(
                        x + pen,
                        y + size * 0.55,
                        x + size * 0.42,
                        y + size * 0.8,
                        pen,
                        Color::rgb(0, 90, 200),
                        clip,
                    );
                    self.canvas.draw_line(
                        x + size * 0.42,
                        y + size * 0.8,
                        x + size - pen,
                        y + size * 0.22,
                        pen,
                        Color::rgb(0, 90, 200),
                        clip,
                    );
                }
            }
            FormControlKind::Radio => {
                let size = 13.0 * self.scale;
                let cx = content.x + size / 2.0 + 1.0;
                let cy = content.y + content.h / 2.0;
                let border = if b.style.border_color[0].a > 0 {
                    b.style.border_color[0]
                } else {
                    Color::rgb(118, 118, 118)
                };
                self.canvas.fill_circle(cx, cy, size / 2.0, Color::WHITE, clip);
                self.canvas.stroke_round_rect(
                    Rect::new(cx - size / 2.0, cy - size / 2.0, size, size),
                    size / 2.0,
                    1.0 * self.scale,
                    border,
                    clip,
                );
                if form.checked {
                    self.canvas.fill_circle(cx, cy, size * 0.24, Color::rgb(0, 90, 200), clip);
                }
            }
            FormControlKind::Range => {
                let track_h = (4.0 * self.scale).max(2.0);
                let cy = content.y + content.h / 2.0;
                let track = Rect::new(content.x + 4.0, cy - track_h / 2.0, (content.w - 8.0).max(1.0), track_h);
                self.canvas.fill_round_rect(track, track_h / 2.0, Color::rgb(200, 200, 200), clip);
                let thumb_x = content.x + content.w * 0.5;
                self.canvas
                    .fill_circle(thumb_x, cy, (7.0 * self.scale).max(4.0), Color::rgb(0, 120, 215), clip);
            }
            FormControlKind::Button | FormControlKind::Submit | FormControlKind::File => {
                let text = if form.kind == FormControlKind::File {
                    form.value.clone()
                } else if form.value.is_empty() {
                    if form.kind == FormControlKind::Submit {
                        "Submit".to_string()
                    } else {
                        "Button".to_string()
                    }
                } else {
                    form.value.clone()
                };
                let (asc, _) = self.fonts.ascent_descent(&spec);
                let w = self.fonts.measure(&text, &spec);
                let x = content.x + (content.w - w).max(0.0) / 2.0;
                let baseline = content.y + (content.h - spec.size) / 2.0 + asc;
                self.draw_text(&text, x, baseline, &spec, b.style.color, 1.0, clip);
            }
            FormControlKind::Other => {}
            _ => {
                let mut text = form.value.clone();
                let is_placeholder = text.is_empty();
                if is_placeholder {
                    text = form.placeholder.clone();
                }
                if form.kind == FormControlKind::PasswordInput && !text.is_empty() {
                    text = "•".repeat(text.chars().count());
                }
                if text.is_empty() {
                    return;
                }
                let color = if is_placeholder {
                    Color::rgba(117, 117, 117, 220)
                } else {
                    b.style.color
                };
                let (asc, desc) = self.fonts.ascent_descent(&spec);
                let line_h = asc + desc;
                let pad = 2.0 * self.scale;
                let wrap = form.kind == FormControlKind::Textarea;
                let max_w = (content.w - pad * 2.0).max(1.0);
                let mut y = content.y + pad;
                let baseline = y + asc;
                if wrap {
                    let mut line = String::new();
                    let mut line_w = 0.0;
                    let mut lines: Vec<(String, f32)> = Vec::new();
                    for ch in text.chars() {
                        if ch == '\n' {
                            lines.push((line.clone(), line_w));
                            line.clear();
                            line_w = 0.0;
                            continue;
                        }
                        let cw = self.fonts.measure(&ch.to_string(), &spec);
                        if line_w + cw > max_w && !line.is_empty() {
                            lines.push((line.clone(), line_w));
                            line.clear();
                            line_w = 0.0;
                        }
                        line.push(ch);
                        line_w += cw;
                    }
                    if !line.is_empty() {
                        lines.push((line, line_w));
                    }
                    for (i, (l, _)) in lines.iter().enumerate() {
                        let bl = baseline + i as f32 * line_h;
                        if bl > content.bottom() {
                            break;
                        }
                        self.draw_text(l, content.x + pad, bl, &spec, color, 1.0, clip);
                    }
                } else {
                    let max_chars = text.chars().count();
                    let mut shown = text.clone();
                    while self.fonts.measure(&shown, &spec) > max_w && shown.chars().count() > 1 {
                        let keep = shown.chars().count().saturating_sub(1);
                        shown = shown.chars().take(keep).collect();
                    }
                    let _ = max_chars;
                    self.draw_text(&shown, content.x + pad, baseline, &spec, color, 1.0, clip);
                }
                if form.kind == FormControlKind::Select {
                    let cx = content.right() - 8.0 * self.scale;
                    let cy = content.y + content.h / 2.0;
                    let s = 3.0 * self.scale;
                    let c = Color::rgba(90, 90, 90, 255);
                    self.canvas.draw_line(cx - s, cy - s * 0.4, cx, cy + s * 0.6, 1.0 * self.scale, c, clip);
                    self.canvas.draw_line(cx, cy + s * 0.6, cx + s, cy - s * 0.4, 1.0 * self.scale, c, clip);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_text(
        &mut self,
        text: &str,
        x: f32,
        baseline: f32,
        spec: &veyra_layout::FontSpec,
        color: Color,
        opacity: f32,
        clip: Rect,
    ) {
        let mut pen = x;
        for ch in text.chars() {
            if ch == '\n' || ch == '\r' {
                continue;
            }
            let g = self.fonts.glyph(spec, ch);
            let gx = (pen + g.xmin as f32).round() as i32;
            let gy = (baseline - g.ymin as f32 - g.height as f32).round() as i32;
            self.canvas
                .blit_glyph(&g.bitmap, g.width, g.height, gx, gy, color, clip, opacity);
            pen += g.advance + spec.letter_spacing;
        }
    }

    fn paint_layer(&mut self, b: &LayoutBox) {
        let bounds = self.subtree_bounds(b);
        let blur_pad = b
            .style
            .box_shadows
            .iter()
            .map(|s| s.blur + s.spread.abs())
            .fold(0.0, f32::max);
        let pad = 4.0 + blur_pad;
        let bounds = Rect::new(bounds.x - pad, bounds.y - pad, bounds.w + pad * 2.0, bounds.h + pad * 2.0);
        let pw = ((bounds.w * self.scale).ceil() as usize).saturating_add(2);
        let ph = ((bounds.h * self.scale).ceil() as usize).saturating_add(2);
        if pw < 2 || ph < 2 || pw.saturating_mul(ph) > 40_000_000 {
            return;
        }

        let saved_canvas = std::mem::replace(&mut self.canvas, Canvas::transparent(pw, ph));
        let saved_offset = self.offset;
        let saved_scroll = self.scroll;
        let saved_clip =
            std::mem::replace(&mut self.clip, vec![Rect::new(0.0, 0.0, pw as f32, ph as f32)]);
        self.offset = Point::new(-bounds.x, -bounds.y);
        self.scroll = Point::ZERO;
        self.layer_budget = self.layer_budget.saturating_sub(1);
        self.paint_box(b, true);
        self.layer_budget += 1;
        self.offset = saved_offset;
        self.scroll = saved_scroll;
        self.clip = saved_clip;
        let layer = std::mem::replace(&mut self.canvas, saved_canvas);

        let opacity = b.style.opacity.clamp(0.0, 1.0);
        match b.style.transform {
            Some(t) if !t.is_identity() => {
                let ox = b.rect.x
                    + b.style
                        .transform_origin
                        .0
                        .resolve(b.rect.w)
                        .unwrap_or(b.rect.w / 2.0);
                let oy = b.rect.y
                    + b.style
                        .transform_origin
                        .1
                        .resolve(b.rect.h)
                        .unwrap_or(b.rect.h / 2.0);
                let tt = Transform2D::translate(ox, oy)
                    .multiply(&t)
                    .multiply(&Transform2D::translate(-ox, -oy));
                self.composite_transformed(&layer, bounds, &tt, opacity);
            }
            _ => {
                let dst = self.map(bounds);
                let clip = self.clip_rect();
                self.canvas.composite(&layer, dst, opacity, clip);
            }
        }
    }

    fn subtree_bounds(&self, b: &LayoutBox) -> Rect {
        let mut out = b.rect;
        let mut stack = vec![b];
        while let Some(cur) = stack.pop() {
            out = union(out, cur.rect);
            for c in &cur.children {
                stack.push(c);
            }
        }
        for t in self.all_texts {
            let inside = t.owner.is_some_and(|o| self.node_in_subtree(b, o));
            if inside {
                out = union(out, t.rect);
            }
        }
        out
    }

    fn node_in_subtree(&self, root: &LayoutBox, node: NodeId) -> bool {
        if root.node == Some(node) {
            return true;
        }
        let mut stack = vec![root];
        while let Some(cur) = stack.pop() {
            if cur.node == Some(node) {
                return true;
            }
            for c in &cur.children {
                stack.push(c);
            }
        }
        false
    }

    fn composite_transformed(&mut self, layer: &Canvas, bounds: Rect, t: &Transform2D, opacity: f32) {
        let inv = match t.inverse() {
            Some(i) => i,
            None => return,
        };
        let clip = self.clip_rect();
        let scale = self.scale;
        let corners = [
            (bounds.x, bounds.y),
            (bounds.right(), bounds.y),
            (bounds.x, bounds.bottom()),
            (bounds.right(), bounds.bottom()),
        ];
        let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
        let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
        for (x, y) in corners {
            let (tx, ty) = t.apply(x, y);
            let dx = (tx + self.offset.x - self.scroll.x) * scale;
            let dy = (ty + self.offset.y - self.scroll.y) * scale;
            min_x = min_x.min(dx);
            min_y = min_y.min(dy);
            max_x = max_x.max(dx);
            max_y = max_y.max(dy);
        }
        let area = Rect::new(min_x, min_y, max_x - min_x, max_y - min_y)
            .intersection(&clip)
            .intersection(&self.canvas.bounds());
        if area.is_empty() {
            return;
        }
        for y in (area.y.floor() as i32)..(area.bottom().ceil() as i32) {
            for x in (area.x.floor() as i32)..(area.right().ceil() as i32) {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                if px < area.x || px > area.right() || py < area.y || py > area.bottom() {
                    continue;
                }
                let page_x = px / scale - self.offset.x + self.scroll.x;
                let page_y = py / scale - self.offset.y + self.scroll.y;
                let (qx, qy) = inv.apply(page_x, page_y);
                let u = (qx - bounds.x) * scale;
                let v = (qy - bounds.y) * scale;
                if u < 0.0 || v < 0.0 || u >= layer.width as f32 || v >= layer.height as f32 {
                    continue;
                }
                let c = layer.get(u as usize, v as usize);
                if c.a > 0 {
                    self.canvas.blend(x, y, c, opacity);
                }
            }
        }
    }
}

fn union(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let right = a.right().max(b.right());
    let bottom = a.bottom().max(b.bottom());
    Rect::new(x, y, right - x, bottom - y)
}

/// Convenience: the clipping policy for a scroll container.
pub fn clips_content(style: &veyra_css::ComputedStyle) -> bool {
    !matches!(style.overflow_x, Overflow::Visible) || !matches!(style.overflow_y, Overflow::Visible)
}

/// Convenience for text alignment, used by the shell when drawing selection.
pub fn alignment_offset(align: TextAlign, line_width: f32, available: f32) -> f32 {
    match align {
        TextAlign::Center => ((available - line_width) / 2.0).max(0.0),
        TextAlign::Right | TextAlign::End => (available - line_width).max(0.0),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veyra_common::Url;
    use veyra_css::StyleEngine;
    use veyra_html::parse_document;
    use veyra_layout::{layout_document, NoImages};

    fn render(html: &str, css: &str) -> Canvas {
        let base = Url::parse("https://example.test/").unwrap();
        let (mut doc, _) = parse_document(html, base.clone());
        let mut styles = StyleEngine::new();
        styles.set_viewport(600.0, 400.0);
        styles.add_stylesheet(css, &base);
        styles.compute(&doc);
        let fonts = FontSystem::new();
        let images = ImageCache::new();
        let tree = layout_document(&doc, &styles, &fonts, &NoImages, Size::new(600.0, 400.0));
        let cfg = PaintConfig {
            tree: &tree,
            fonts: &fonts,
            images: &images,
            viewport: Size::new(600.0, 400.0),
            scroll: Point::ZERO,
            scale: 1.0,
            background: Color::rgb(255, 255, 255),
        };
        paint_page(&cfg)
    }

    fn ink(c: &Canvas) -> usize {
        c.pixels
            .chunks_exact(4)
            .filter(|p| p[0] < 200 && p[3] > 0)
            .count()
    }

    #[test]
    fn paints_text_and_background() {
        let c = render(
            "<body><h1>Hello Veyra</h1><p>layout and paint</p></body>",
            "body{margin:0;background:#ffffff}h1{color:#102a43;font-size:32px}p{color:#334e68}",
        );
        assert!(ink(&c) > 200, "expected glyph ink, got {}", ink(&c));
        assert_eq!(c.get(5, 5), Color::rgb(255, 255, 255));
    }

    #[test]
    fn renders_gradients_rounded_corners() {
        let c = render(
            "<body><div class='card'></div></body>",
            "body{margin:0;background:#fff}.card{width:200px;height:100px;margin:20px;background:linear-gradient(90deg,#ff0000,#00ff00);border-radius:20px}",
        );
        let left = c.get(40, 70);
        let right = c.get(180, 70);
        assert!(left.r > right.r, "left {left:?} right {right:?}");
        assert!(right.g > left.g);
        assert!(c.get(21, 21).r > 240, "corner should stay white");
    }

    #[test]
    fn renders_box_shadows_and_borders() {
        let c = render(
            "<body><div class='box'></div></body>",
            "body{margin:0;background:#fff}.box{width:160px;height:80px;margin:40px;background:#fff;border:2px solid #2563eb;box-shadow:0 8px 20px rgba(0,0,0,.5)}",
        );
        assert!(c.get(43, 43).b > 200, "border should be blue");
        let below = c.get(120, 126);
        assert!(below.r < 250, "shadow should darken below the box: {below:?}");
    }

    #[test]
    fn opacity_and_transform_layer() {
        let c = render(
            "<body><div class='box'></div></body>",
            "body{margin:0;background:#fff}.box{width:100px;height:100px;margin:100px;background:#0000ff;transform:rotate(45deg)}",
        );
        // The rotated diamond extends above the original box.
        let above_ink = (80..98).any(|y| (145..155).any(|x| c.get(x, y).b > 180));
        assert!(above_ink, "rotated content should appear above the box");
        assert!(c.get(100, 100).r > 240, "untransformed corner should be clear");
    }

    #[test]
    fn background_clip_text_paints_gradient_through_glyphs() {
        let base = Url::parse("https://example.test/").unwrap();
        let (doc, _) = parse_document(
            "<body><span class='g'>MAG</span></body>",
            base.clone(),
        );
        let mut styles = StyleEngine::new();
        styles.set_viewport(600.0, 400.0);
        styles.add_stylesheet(
            "body{margin:0;background:#fff}.g{display:inline-block;font-size:64px;background:linear-gradient(90deg,#ff0000,#0000ff);-webkit-background-clip:text;background-clip:text;color:transparent}",
            &base,
        );
        styles.compute(&doc);
        let span = doc.query_selector(doc.root, ".g").expect("span");
        assert_eq!(
            styles.style_rc(span).background_clip,
            veyra_css::style::BackgroundClip::Text,
            "background-clip: text should be parsed"
        );

        let fonts = FontSystem::new();
        let images = ImageCache::new();
        let tree = layout_document(&doc, &styles, &fonts, &NoImages, Size::new(600.0, 400.0));
        let cfg = PaintConfig {
            tree: &tree,
            fonts: &fonts,
            images: &images,
            viewport: Size::new(600.0, 400.0),
            scroll: Point::ZERO,
            scale: 1.0,
            background: Color::rgb(255, 255, 255),
        };
        let c = paint_page(&cfg);
        let mut red = 0;
        let mut blue = 0;
        for p in c.pixels.chunks_exact(4) {
            let (r, g, b) = (p[0], p[1], p[2]);
            if r > 140 && b < 120 && r > g + 40 {
                red += 1;
            }
            if b > 140 && r < 120 && b > g + 40 {
                blue += 1;
            }
        }
        assert!(
            red > 20 && blue > 20,
            "gradient should show through the text: red {red}, blue {blue}"
        );
    }
}
