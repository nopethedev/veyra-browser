//! A software RGBA rasteriser with anti-aliased rectangles, rounded corners,
//! gradients, images, text glyph blitting and damage diffing.

use veyra_common::{Color, Rect};
use veyra_css::values::GradientStop;

#[derive(Debug, Clone)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Canvas {
        let width = width.max(1);
        let height = height.max(1);
        Canvas {
            width,
            height,
            pixels: vec![255; width * height * 4],
        }
    }

    pub fn transparent(width: usize, height: usize) -> Canvas {
        let width = width.max(1);
        let height = height.max(1);
        Canvas {
            width,
            height,
            pixels: vec![0; width * height * 4],
        }
    }

    pub fn fill(&mut self, color: Color) {
        for px in self.pixels.chunks_exact_mut(4) {
            px[0] = color.r;
            px[1] = color.g;
            px[2] = color.b;
            px[3] = color.a;
        }
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0.0, 0.0, self.width as f32, self.height as f32)
    }

    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, color: Color, coverage: f32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let a = (color.a as f32 / 255.0) * coverage.clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let idx = (y as usize * self.width + x as usize) * 4;
        let dst_a = self.pixels[idx + 3] as f32 / 255.0;
        let out_a = a + dst_a * (1.0 - a);
        if out_a <= 0.0 {
            self.pixels[idx] = 0;
            self.pixels[idx + 1] = 0;
            self.pixels[idx + 2] = 0;
            self.pixels[idx + 3] = 0;
            return;
        }
        for (k, src) in [color.r, color.g, color.b].iter().enumerate() {
            let s = *src as f32 / 255.0;
            let d = self.pixels[idx + k] as f32 / 255.0;
            let out = (s * a + d * dst_a * (1.0 - a)) / out_a;
            self.pixels[idx + k] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        self.pixels[idx + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> Color {
        if x >= self.width || y >= self.height {
            return Color::TRANSPARENT;
        }
        let i = (y * self.width + x) * 4;
        Color::rgba(self.pixels[i], self.pixels[i + 1], self.pixels[i + 2], self.pixels[i + 3])
    }

    #[inline]
    fn coverage_rect(&self, r: Rect) -> (i32, i32, i32, i32) {
        (
            r.x.floor() as i32,
            r.y.floor() as i32,
            r.right().ceil() as i32,
            r.bottom().ceil() as i32,
        )
    }

    pub fn fill_rect(&mut self, r: Rect, color: Color, clip: Rect) {
        let r = r.intersection(&clip);
        if r.is_empty() || color.a == 0 {
            return;
        }
        let (x0, y0, x1, y1) = self.coverage_rect(r);
        for y in y0..y1 {
            let py = Rect::new(r.x, y as f32, r.w, 1.0);
            let cov_y = overlap(py.y, py.bottom(), y as f32, y as f32 + 1.0);
            if cov_y <= 0.0 {
                continue;
            }
            for x in x0..x1 {
                let cov_x = overlap(py.x, py.right(), x as f32, x as f32 + 1.0);
                if cov_x <= 0.0 {
                    continue;
                }
                self.blend(x, y, color, cov_x * cov_y);
            }
        }
    }

    pub fn fill_round_rect(&mut self, r: Rect, radius: f32, color: Color, clip: Rect) {
        if radius <= 0.5 {
            self.fill_rect(r, color, clip);
            return;
        }
        let r = r.intersection(&clip);
        if r.is_empty() || color.a == 0 {
            return;
        }
        let radius = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
        let rect = r;
        let (x0, y0, x1, y1) = self.coverage_rect(r);
        for y in y0..y1 {
            for x in x0..x1 {
                let d = rounded_sdf(x as f32 + 0.5, y as f32 + 0.5, rect, radius);
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(x, y, color, cov);
                }
            }
        }
    }

    pub fn stroke_round_rect(&mut self, r: Rect, radius: f32, width: f32, color: Color, clip: Rect) {
        if width <= 0.0 || color.a == 0 {
            return;
        }
        let outer = r.intersection(&clip);
        if outer.is_empty() {
            return;
        }
        let radius = radius.min(r.w / 2.0).min(r.h / 2.0);
        let (x0, y0, x1, y1) = self.coverage_rect(r);
        for y in y0..y1 {
            for x in x0..x1 {
                let d = rounded_sdf(x as f32 + 0.5, y as f32 + 0.5, r, radius);
                // Inside the outer edge and within `width` of it.
                if d <= 0.0 && d > -width {
                    let cov = (0.5 - d).clamp(0.0, 1.0);
                    let cov2 = (d + width + 0.5).clamp(0.0, 1.0);
                    self.blend(x, y, color, cov.min(cov2));
                }
            }
        }
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: Color, clip: Rect) {
        if radius <= 0.0 || color.a == 0 {
            return;
        }
        let b = Rect::new(cx - radius, cy - radius, radius * 2.0, radius * 2.0).intersection(&clip);
        if b.is_empty() {
            return;
        }
        for y in (b.y.floor() as i32)..(b.bottom().ceil() as i32) {
            for x in (b.x.floor() as i32)..(b.right().ceil() as i32) {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let d = (dx * dx + dy * dy).sqrt() - radius;
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(x, y, color, cov);
                }
            }
        }
    }

    pub fn draw_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: Color, clip: Rect) {
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len2 = dx * dx + dy * dy;
        if len2 < 0.0001 || color.a == 0 {
            return;
        }
        let pad = width / 2.0 + 1.0;
        let b = Rect::new(x0.min(x1) - pad, y0.min(y1) - pad, (x1 - x0).abs() + pad * 2.0, (y1 - y0).abs() + pad * 2.0)
            .intersection(&clip);
        for y in (b.y.floor() as i32)..(b.bottom().ceil() as i32) {
            for x in (b.x.floor() as i32)..(b.right().ceil() as i32) {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let t = (((px - x0) * dx + (py - y0) * dy) / len2).clamp(0.0, 1.0);
                let qx = x0 + t * dx;
                let qy = y0 + t * dy;
                let d = ((px - qx).powi(2) + (py - qy).powi(2)).sqrt() - width / 2.0;
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(x, y, color, cov);
                }
            }
        }
    }

    pub fn fill_gradient(
        &mut self,
        r: Rect,
        angle_deg: f32,
        stops: &[GradientStop],
        clip: Rect,
        radius: f32,
    ) {
        if stops.is_empty() {
            return;
        }
        let r = r.intersection(&clip);
        if r.is_empty() {
            return;
        }
        // CSS angles: 0deg points up, 90deg points right.
        let rad = angle_deg.to_radians();
        let dir = (rad.sin(), -rad.cos());
        let center = r.center();
        let (cx, cy) = (center.x, center.y);
        let half = (r.w * dir.0.abs() + r.h * dir.1.abs()) / 2.0;
        if half <= 0.0 {
            return;
        }
        let mut sorted: Vec<(f32, Color)> = stops.iter().map(|s| (s.position.unwrap_or(0.0), s.color)).collect();
        let mut prev = 0.0f32;
        for (i, s) in sorted.iter_mut().enumerate() {
            if stops[i].position.is_none() {
                s.0 = prev;
            }
            prev = s.0;
        }
        if sorted[0].0 > 0.0 {
            sorted[0].0 = 0.0;
        }
        let last = sorted.len() - 1;
        if sorted[last].0 < 1.0 {
            sorted[last].0 = 1.0;
        }
        let radius = radius.min(r.w / 2.0).min(r.h / 2.0);
        for y in (r.y.floor() as i32)..(r.bottom().ceil() as i32) {
            for x in (r.x.floor() as i32)..(r.right().ceil() as i32) {
                let cov = if radius > 0.5 {
                    let d = rounded_sdf(x as f32 + 0.5, y as f32 + 0.5, r, radius);
                    (0.5 - d).clamp(0.0, 1.0)
                } else {
                    let cx0 = x as f32;
                    let cy0 = y as f32;
                    overlap(r.x, r.right(), cx0, cx0 + 1.0) * overlap(r.y, r.bottom(), cy0, cy0 + 1.0)
                };
                if cov <= 0.0 {
                    continue;
                }
                let t = (((x as f32 + 0.5 - cx) * dir.0 + (y as f32 + 0.5 - cy) * dir.1) / (half * 2.0) + 0.5)
                    .clamp(0.0, 1.0);
                let color = sample_stops(&sorted, t);
                self.blend(x, y, color, cov);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_image(
        &mut self,
        img: &crate::images::Image,
        dst: Rect,
        clip: Rect,
        radius: f32,
        opacity: f32,
        smooth: bool,
    ) {
        let area = dst.intersection(&clip);
        if area.is_empty() || img.width == 0 || img.height == 0 {
            return;
        }
        let radius = radius.min(dst.w / 2.0).min(dst.h / 2.0);
        for y in (area.y.floor() as i32)..(area.bottom().ceil() as i32) {
            for x in (area.x.floor() as i32)..(area.right().ceil() as i32) {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                if px < area.x || px > area.right() || py < area.y || py > area.bottom() {
                    continue;
                }
                let cov = if radius > 0.5 {
                    let d = rounded_sdf(px, py, dst, radius);
                    (0.5 - d).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                if cov <= 0.0 {
                    continue;
                }
                let u = ((px - dst.x) / dst.w).clamp(0.0, 1.0) * (img.width - 1) as f32;
                let v = ((py - dst.y) / dst.h).clamp(0.0, 1.0) * (img.height - 1) as f32;
                let color = if smooth {
                    img.sample_bilinear(u, v)
                } else {
                    img.sample(u.round() as usize, v.round() as usize)
                };
                if color.a > 0 {
                    self.blend(x, y, color, cov * opacity);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn blit_glyph(
        &mut self,
        bitmap: &[u8],
        gw: usize,
        gh: usize,
        x: i32,
        y: i32,
        color: Color,
        clip: Rect,
        opacity: f32,
    ) {
        if gw == 0 || gh == 0 {
            return;
        }
        for gy in 0..gh {
            let py = y.saturating_add(gy as i32);
            if py < clip.y as i32 || py >= clip.bottom() as i32 {
                continue;
            }
            for gx in 0..gw {
                let px = x.saturating_add(gx as i32);
                if px < clip.x as i32 || px >= clip.right() as i32 {
                    continue;
                }
                let cov = bitmap[gy * gw + gx] as f32 / 255.0;
                if cov > 0.0 {
                    self.blend(px, py, color, cov * opacity);
                }
            }
        }
    }

    /// Composites `other` at `dst` (nearest-neighbour scaling, used for layers).
    pub fn composite(&mut self, other: &Canvas, dst: Rect, opacity: f32, clip: Rect) {
        let area = dst.intersection(&clip);
        if area.is_empty() {
            return;
        }
        for y in (area.y.floor() as i32)..(area.bottom().ceil() as i32) {
            for x in (area.x.floor() as i32)..(area.right().ceil() as i32) {
                let u = ((x as f32 + 0.5 - dst.x) / dst.w * other.width as f32).floor() as i32;
                let v = ((y as f32 + 0.5 - dst.y) / dst.h * other.height as f32).floor() as i32;
                if u < 0 || v < 0 || u >= other.width as i32 || v >= other.height as i32 {
                    continue;
                }
                let c = other.get(u as usize, v as usize);
                if c.a > 0 {
                    self.blend(x, y, c, opacity);
                }
            }
        }
    }

    pub fn to_rgba_bytes(&self) -> &[u8] {
        &self.pixels
    }

    pub fn to_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, self.width as u32, self.height as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("png header");
            writer.write_image_data(&self.pixels).expect("png data");
        }
        out
    }

    pub fn save_png(&self, path: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_png())
    }

    /// Returns rectangles covering pixels that differ between two canvases.
    pub fn damage(&self, other: &Canvas) -> Vec<Rect> {
        if self.width != other.width || self.height != other.height {
            return vec![self.bounds()];
        }
        let mut rects = Vec::new();
        let mut y = 0usize;
        while y < self.height {
            let row_start = y * self.width * 4;
            let row_end = row_start + self.width * 4;
            if self.pixels[row_start..row_end] != other.pixels[row_start..row_end] {
                let start_y = y;
                let mut end_y = y + 1;
                let mut ny = y + 1;
                while ny < self.height {
                    let s = ny * self.width * 4;
                    let e = s + self.width * 4;
                    if self.pixels[s..e] != other.pixels[s..e] {
                        end_y = ny + 1;
                        ny += 1;
                    } else {
                        break;
                    }
                }
                rects.push(Rect::new(0.0, start_y as f32, self.width as f32, (end_y - start_y) as f32));
                y = end_y;
                let _ = start_y;
            } else {
                y += 1;
            }
        }
        merge_vertical(rects)
    }
}

fn merge_vertical(rects: Vec<Rect>) -> Vec<Rect> {
    // Merge adjacent full-width row bands.
    let mut out: Vec<Rect> = Vec::new();
    for r in rects {
        if let Some(last) = out.last_mut() {
            if (last.bottom() - r.y).abs() < 0.01 {
                last.h += r.h;
                continue;
            }
        }
        out.push(r);
    }
    out
}

fn overlap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

fn rounded_sdf(px: f32, py: f32, r: Rect, radius: f32) -> f32 {
    let hx = r.w / 2.0;
    let hy = r.h / 2.0;
    let cx = r.x + hx;
    let cy = r.y + hy;
    let qx = (px - cx).abs() - (hx - radius);
    let qy = (py - cy).abs() - (hy - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    outside + inside - radius
}

fn sample_stops(stops: &[(f32, Color)], t: f32) -> Color {
    if stops.is_empty() {
        return Color::TRANSPARENT;
    }
    if t <= stops[0].0 {
        return stops[0].1;
    }
    for w in stops.windows(2) {
        let (p0, c0) = w[0];
        let (p1, c1) = w[1];
        if t >= p0 && t <= p1 {
            let span = (p1 - p0).max(0.0001);
            let f = (t - p0) / span;
            return c0.mix(c1, f);
        }
    }
    stops[stops.len() - 1].1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blending_and_fill() {
        let mut c = Canvas::new(10, 10);
        c.fill(Color::WHITE);
        c.fill_rect(Rect::new(2.0, 2.0, 4.0, 4.0), Color::rgb(255, 0, 0), c.bounds());
        assert_eq!(c.get(3, 3), Color::rgb(255, 0, 0));
        assert_eq!(c.get(0, 0), Color::WHITE);
        let mut d = Canvas::new(10, 10);
        d.fill(Color::WHITE);
        d.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::rgba(0, 0, 0, 128), d.bounds());
        let mid = d.get(5, 5);
        assert!(mid.r > 120 && mid.r < 135);
    }

    #[test]
    fn rounded_corners_are_transparent() {
        let mut c = Canvas::transparent(20, 20);
        c.fill_round_rect(Rect::new(0.0, 0.0, 20.0, 20.0), 8.0, Color::rgb(0, 0, 255), c.bounds());
        assert!(c.get(0, 0).a < 40);
        assert_eq!(c.get(10, 10), Color::rgb(0, 0, 255));
    }

    #[test]
    fn gradients_interpolate() {
        let mut c = Canvas::new(20, 4);
        let stops = vec![
            GradientStop { color: Color::BLACK, position: Some(0.0) },
            GradientStop { color: Color::WHITE, position: Some(1.0) },
        ];
        c.fill_gradient(Rect::new(0.0, 0.0, 20.0, 4.0), 90.0, &stops, c.bounds(), 0.0);
        let left = c.get(0, 2);
        let right = c.get(19, 2);
        assert!(left.r < 60, "{left:?}");
        assert!(right.r > 200, "{right:?}");
    }

    #[test]
    fn damage_reports_changed_rows() {
        let a = Canvas::new(10, 10);
        let mut b = Canvas::new(10, 10);
        b.fill_rect(Rect::new(0.0, 3.0, 10.0, 2.0), Color::BLACK, b.bounds());
        let d = a.damage(&b);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].y, 3.0);
        assert_eq!(d[0].h, 2.0);
    }

    #[test]
    fn png_encoding() {
        let c = Canvas::new(4, 4);
        let png = c.to_png();
        assert_eq!(&png[1..4], b"PNG");
    }
}
