//! A small software rasteriser: z-buffer, Gouraud shading, depth-tested
//! lines and an id buffer for picking. Draws into a pixel buffer that the
//! viewer shows as a texture.

use eframe::egui::Color32;

pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub color: Vec<Color32>,
    /// 1 / depth. Larger is nearer, 0 is empty.
    pub inv_z: Vec<f32>,
    /// What was drawn at each pixel, 0 for nothing. See `pack_id`.
    pub id: Vec<u32>,
}

pub fn pack_id(body: usize, face: usize) -> u32 {
    (((body + 1) as u32) << 20) | (face as u32 & 0xF_FFFF)
}

pub fn unpack_id(id: u32) -> Option<(usize, usize)> {
    if id == 0 {
        None
    } else {
        Some((((id >> 20) - 1) as usize, (id & 0xF_FFFF) as usize))
    }
}

impl Frame {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            color: vec![Color32::TRANSPARENT; w * h],
            inv_z: vec![0.0; w * h],
            id: vec![0; w * h],
        }
    }

    /// Fill one triangle. `p` are pixel positions, `iz` the 1/depth of the
    /// corners, `c` the corner colours (0..255).
    pub fn triangle(&mut self, p: [[f32; 2]; 3], iz: [f32; 3], c: [[f32; 3]; 3], id: u32) {
        let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
        if area.abs() < 1e-6 {
            return;
        }
        let inv_area = 1.0 / area;
        let minx = p.iter().map(|q| q[0]).fold(f32::MAX, f32::min).floor().max(0.0) as i64;
        let maxx = p.iter().map(|q| q[0]).fold(f32::MIN, f32::max).ceil().min(self.w as f32 - 1.0) as i64;
        let miny = p.iter().map(|q| q[1]).fold(f32::MAX, f32::min).floor().max(0.0) as i64;
        let maxy = p.iter().map(|q| q[1]).fold(f32::MIN, f32::max).ceil().min(self.h as f32 - 1.0) as i64;
        if minx > maxx || miny > maxy {
            return;
        }
        const EPS: f32 = -1e-4;
        for y in miny..=maxy {
            let py = y as f32 + 0.5;
            for x in minx..=maxx {
                let px = x as f32 + 0.5;
                let w0 = ((p[1][0] - px) * (p[2][1] - py) - (p[1][1] - py) * (p[2][0] - px)) * inv_area;
                let w1 = ((p[2][0] - px) * (p[0][1] - py) - (p[2][1] - py) * (p[0][0] - px)) * inv_area;
                let w2 = 1.0 - w0 - w1;
                if w0 < EPS || w1 < EPS || w2 < EPS {
                    continue;
                }
                let z = w0 * iz[0] + w1 * iz[1] + w2 * iz[2];
                let i = y as usize * self.w + x as usize;
                if z <= self.inv_z[i] {
                    continue;
                }
                self.inv_z[i] = z;
                self.id[i] = id;
                let ch = |k: usize| (w0 * c[0][k] + w1 * c[1][k] + w2 * c[2][k]).clamp(0.0, 255.0) as u8;
                self.color[i] = Color32::from_rgb(ch(0), ch(1), ch(2));
            }
        }
    }

    /// A 2 pixel wide line, hidden where a nearer surface covers it. It does
    /// not write depth, so lines never hide each other.
    pub fn line(&mut self, a: [f32; 2], b: [f32; 2], za: f32, zb: f32, color: Color32) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let n = dx.abs().max(dy.abs()).ceil().max(1.0).min(20000.0) as usize;
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let (x, y) = (a[0] + dx * t, a[1] + dy * t);
            let z = za + (zb - za) * t;
            for (ox, oy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)] {
                let (xi, yi) = ((x - 0.5 + ox).round(), (y - 0.5 + oy).round());
                if xi < 0.0 || yi < 0.0 || xi >= self.w as f32 || yi >= self.h as f32 {
                    continue;
                }
                let idx = yi as usize * self.w + xi as usize;
                if z >= self.inv_z[idx] * 0.998 {
                    self.color[idx] = color;
                }
            }
        }
    }

    /// Largest 1/depth around a pixel, to test if something is visible there.
    pub fn nearest_z(&self, x: i64, y: i64) -> f32 {
        let mut best = 0.0_f32;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (xx, yy) = (x + dx, y + dy);
                if xx >= 0 && yy >= 0 && (xx as usize) < self.w && (yy as usize) < self.h {
                    best = best.max(self.inv_z[yy as usize * self.w + xx as usize]);
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearer_triangle_wins_and_lines_hide() {
        let mut f = Frame::new(20, 20);
        let tri = [[0.0, 0.0], [20.0, 0.0], [0.0, 20.0]];
        f.triangle(tri, [0.1; 3], [[255.0, 0.0, 0.0]; 3], pack_id(0, 1));
        f.triangle(tri, [0.2; 3], [[0.0, 255.0, 0.0]; 3], pack_id(0, 2));
        let i = 5 * 20 + 5;
        assert_eq!(f.color[i], Color32::from_rgb(0, 255, 0));
        assert_eq!(unpack_id(f.id[i]), Some((0, 2)));
        // A line farther than the face is hidden, a nearer one is drawn.
        f.line([2.0, 2.0], [8.0, 2.0], 0.1, 0.1, Color32::BLACK);
        assert_ne!(f.color[2 * 20 + 5], Color32::BLACK);
        f.line([2.0, 4.0], [8.0, 4.0], 0.3, 0.3, Color32::BLACK);
        assert_eq!(f.color[4 * 20 + 5], Color32::BLACK);
        // Outside the triangle stays empty.
        assert_eq!(f.id[19 * 20 + 19], 0);
    }
}
