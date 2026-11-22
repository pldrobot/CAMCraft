//! Preview images of projects for the welcome window. Drawn when a project is saved and kept in
//! the user config folder, so the recent list does not need to open the projects.

use crate::p2d::model::Project;
use crate::p3d::model::Project3;
use image::{Rgba, RgbaImage};
use std::path::{Path, PathBuf};

pub const W: u32 = 320;
pub const H: u32 = 180;
const BG: Rgba<u8> = Rgba([0x21, 0x28, 0x30, 255]);
const BED: Rgba<u8> = Rgba([0x3f, 0xc1, 0x6b, 255]);
const PATH: Rgba<u8> = Rgba([0xe6, 0xec, 0xf2, 255]);
const EDGE: Rgba<u8> = Rgba([0x6f, 0xc3, 0xff, 255]);

/// Where the preview of a project file is kept.
pub fn file_for(project: &Path) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    project.hash(&mut h);
    dirs::config_dir().map(|d| d.join("CAMCraft").join("thumbs").join(format!("{:016x}.png", h.finish())))
}

/// A failed write only loses the preview, so it is ignored.
pub fn save(project: &Path, img: &RgbaImage) {
    if let Some(f) = file_for(project) {
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = img.save(f);
    }
}

/// Maps drawing coordinates (y up) into the image, keeping the aspect ratio.
struct Fit {
    min: [f64; 2],
    scale: f64,
    off: [f64; 2],
}

impl Fit {
    fn new(min: [f64; 2], max: [f64; 2]) -> Self {
        let m = 14.0;
        let (w, h) = ((max[0] - min[0]).max(1e-6), (max[1] - min[1]).max(1e-6));
        let scale = ((W as f64 - 2.0 * m) / w).min((H as f64 - 2.0 * m) / h);
        Self { min, scale, off: [(W as f64 - w * scale) / 2.0, (H as f64 - h * scale) / 2.0] }
    }

    fn map(&self, p: [f64; 2], max_y: f64) -> [f64; 2] {
        // `max_y` flips the Y axis, the image grows downwards.
        [self.off[0] + (p[0] - self.min[0]) * self.scale, self.off[1] + (max_y - p[1]) * self.scale]
    }
}

fn line(img: &mut RgbaImage, a: [f64; 2], b: [f64; 2], c: Rgba<u8>) {
    let n = ((b[0] - a[0]).abs().max((b[1] - a[1]).abs()).ceil() as i32).max(1);
    for i in 0..=n {
        let t = i as f64 / n as f64;
        let (x, y) = ((a[0] + (b[0] - a[0]) * t).round() as i32, (a[1] + (b[1] - a[1]) * t).round() as i32);
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let (px, py) = (x + dx, y + dy);
            if px >= 0 && py >= 0 && (px as u32) < W && (py as u32) < H {
                img.put_pixel(px as u32, py as u32, c);
            }
        }
    }
}

pub fn render_2d(p: &Project) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(W, H, BG);
    let (bx, by) = (p.machine.bed_x, p.machine.bed_y);
    let fit = Fit::new([0.0, 0.0], [bx, by]);
    let m = |pt: [f64; 2]| fit.map(pt, by);
    let corners = [[0.0, 0.0], [bx, 0.0], [bx, by], [0.0, by], [0.0, 0.0]];
    for w in corners.windows(2) {
        line(&mut img, m(w[0]), m(w[1]), BED);
    }
    for part in &p.parts {
        for path in &part.paths {
            let mut pts: Vec<[f64; 2]> = path.pts.iter().map(|q| [q[0] + part.offset[0], q[1] + part.offset[1]]).collect();
            if path.closed && !pts.is_empty() {
                pts.push(pts[0]);
            }
            for w in pts.windows(2) {
                line(&mut img, m(w[0]), m(w[1]), PATH);
            }
        }
    }
    img
}

/// Wire frame of the parts on the bed, seen from an isometric direction.
pub fn render_3d(p: &Project3) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(W, H, BG);
    let iso = |q: [f64; 3]| [(q[0] - q[1]) * 0.866, q[2] + (q[0] + q[1]) * 0.5];
    let (bx, by) = (p.machine.bed_x, p.machine.bed_y);
    let bed = [[0.0, 0.0, 0.0], [bx, 0.0, 0.0], [bx, by, 0.0], [0.0, by, 0.0], [0.0, 0.0, 0.0]];
    let mut lo = [f64::MAX; 2];
    let mut hi = [f64::MIN; 2];
    let mut grow = |u: [f64; 2]| {
        for k in 0..2 {
            lo[k] = lo[k].min(u[k]);
            hi[k] = hi[k].max(u[k]);
        }
    };
    bed.iter().for_each(|q| grow(iso(*q)));
    for b in &p.bodies {
        if let Some(g) = &b.geo {
            for e in &g.edges {
                e.pts.iter().for_each(|q| grow(iso(b.world(*q))));
            }
        }
    }
    let fit = Fit::new(lo, hi);
    let m = |u: [f64; 2]| fit.map(u, hi[1]);
    for w in bed.windows(2) {
        line(&mut img, m(iso(w[0])), m(iso(w[1])), BED);
    }
    for b in &p.bodies {
        if let Some(g) = &b.geo {
            for e in &g.edges {
                for w in e.pts.windows(2) {
                    line(&mut img, m(iso(b.world(w[0]))), m(iso(b.world(w[1]))), EDGE);
                }
            }
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::machine::Machine;
    use crate::p2d::model::{PathData, Part};

    #[test]
    fn preview_2d_draws_bed_and_path() {
        let mut p = Project::new(Machine::new("Test"));
        p.parts.push(Part {
            name: "sq".into(),
            paths: vec![PathData { pts: vec![[10.0, 10.0], [60.0, 10.0], [60.0, 60.0], [10.0, 60.0]], closed: true, arcs: vec![] }],
            offset: [0.0, 0.0],
            source: String::new(),
            layers: vec![],
        });
        let img = render_2d(&p);
        assert!(img.pixels().any(|c| *c == BED));
        assert!(img.pixels().any(|c| *c == PATH));
        if let Ok(out) = std::env::var("THUMB_OUT") {
            img.save(out).unwrap();
        }
    }
}
