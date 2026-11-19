//! STEP import for the 3D project: reads the B-rep, tessellates it and keeps
//! the faces (for shading and surface picking) and edges (as paths).

use super::assembly;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use truck_meshalgo::prelude::*;
use truck_stepio::r#in::alias::{Curve3D, Surface};
use truck_stepio::r#in::*;
use truck_topology::compress::*;

pub type V3 = [f64; 3];

/// One B-rep face as triangles.
#[derive(Clone, Debug, Default)]
pub struct FaceMesh {
    pub tris: Vec<[V3; 3]>,
    /// Smoothed normal of every triangle corner, for shading.
    pub nors: Vec<[V3; 3]>,
}

fn unit(v: V3) -> V3 {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l < 1e-15 { [0.0, 0.0, 1.0] } else { [v[0] / l, v[1] / l, v[2] / l] }
}

impl FaceMesh {
    /// Corner normals: the average of the normals of the triangles that share
    /// the corner, ignoring those that bend away more than 50 degrees, so
    /// curved faces look smooth and sharp creases stay sharp.
    fn smooth(&mut self) {
        let key = |p: &V3| [(p[0] * 1e4).round() as i64, (p[1] * 1e4).round() as i64, (p[2] * 1e4).round() as i64];
        let mut map: HashMap<[i64; 3], Vec<V3>> = HashMap::new();
        for t in &self.tris {
            let n = tri_normal(t);
            for p in t {
                map.entry(key(p)).or_default().push(n);
            }
        }
        let cos_lim = 50.0_f64.to_radians().cos();
        self.nors = self
            .tris
            .iter()
            .map(|t| {
                let own = unit(tri_normal(t));
                t.map(|p| {
                    let mut sum = [0.0; 3];
                    for n in map.get(&key(&p)).into_iter().flatten() {
                        let u = unit(*n);
                        let d = u[0] * own[0] + u[1] * own[1] + u[2] * own[2];
                        if d >= cos_lim {
                            for k in 0..3 {
                                sum[k] += n[k];
                            }
                        }
                    }
                    unit(sum)
                })
            })
            .collect();
    }
}

/// One B-rep edge as a polyline.
#[derive(Clone, Debug, Default)]
pub struct EdgePoly {
    pub pts: Vec<V3>,
    /// Faces that use this edge.
    pub faces: Vec<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct Geometry {
    /// Things the user should know, e.g. faces that could not be meshed.
    pub notes: Vec<String>,
    pub faces: Vec<FaceMesh>,
    pub edges: Vec<EdgePoly>,
    pub min: V3,
    pub max: V3,
}

impl Geometry {
    pub fn tri_count(&self) -> usize {
        self.faces.iter().map(|f| f.tris.len()).sum()
    }

    fn compute_bounds(&mut self) {
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        let mut add = |p: &V3| {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        };
        for f in &self.faces {
            for t in &f.tris {
                t.iter().for_each(&mut add);
            }
        }
        for e in &self.edges {
            e.pts.iter().for_each(&mut add);
        }
        if lo[0] > hi[0] {
            lo = [0.0; 3];
            hi = [0.0; 3];
        }
        self.min = lo;
        self.max = hi;
    }
}

/// Cross product of two edges, so its length is twice the area.
pub fn tri_normal(t: &[V3; 3]) -> V3 {
    let a = [t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]];
    let b = [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]];
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn p3(p: &Point3) -> V3 {
    [p.x, p.y, p.z]
}

type CShell = CompressedShell<Point3, Curve3D, Surface>;
type Meshed = CompressedShell<Point3, PolylineCurve<Point3>, Option<PolygonMesh>>;

/// Longest wait for a whole shell, then it is meshed face by face.
const SHELL_SECS: f64 = 3.0;
/// Longest wait for one face. Some faces make the mesher loop forever.
const FACE_SECS: f64 = 0.6;

/// Tessellated shell with the numbering of the shell it came from.
struct ShellOut {
    /// Edge polylines, empty where the edge has none.
    edges: Vec<Vec<V3>>,
    /// Each face with the edges on its boundary.
    faces: Vec<(FaceMesh, Vec<usize>)>,
}

fn extract(m: &Meshed) -> ShellOut {
    let edges = m.edges.iter().map(|e| e.curve.0.iter().map(p3).collect()).collect();
    let faces = m
        .faces
        .iter()
        .map(|f| {
            let mut fm = FaceMesh::default();
            if let Some(mesh) = &f.surface {
                let pos = mesh.positions();
                for tri in mesh.faces().triangle_iter() {
                    fm.tris.push([p3(&pos[tri[0].pos]), p3(&pos[tri[1].pos]), p3(&pos[tri[2].pos])]);
                }
            }
            fm.smooth();
            let used = f.boundaries.iter().flatten().map(|e| e.index).collect();
            (fm, used)
        })
        .collect();
    ShellOut { edges, faces }
}

/// Run `f` on its own thread and give up after `secs`. A thread that does not
/// finish keeps running in the background, it cannot be stopped.
fn run_with_timeout<T: Send + 'static>(secs: f64, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = tx.send(r.ok());
    });
    spawned.ok()?;
    rx.recv_timeout(Duration::from_secs_f64(secs)).ok().flatten()
}

/// A shell holding only face `fi`, with edges and vertices renumbered.
/// Returns it with the original index of each of its edges.
fn single_face(shell: &CShell, fi: usize) -> (CShell, Vec<usize>) {
    let face = &shell.faces[fi];
    let mut vmap: HashMap<usize, usize> = HashMap::new();
    let mut emap: HashMap<usize, usize> = HashMap::new();
    let mut vertices = vec![];
    let mut edges = vec![];
    let mut orig = vec![];
    let mut boundaries = vec![];
    for wire in &face.boundaries {
        let mut w = vec![];
        for ei in wire {
            let idx = *emap.entry(ei.index).or_insert_with(|| {
                let e = &shell.edges[ei.index];
                let mut v = |i: usize| {
                    *vmap.entry(i).or_insert_with(|| {
                        vertices.push(shell.vertices[i]);
                        vertices.len() - 1
                    })
                };
                let (a, b) = (v(e.vertices.0), v(e.vertices.1));
                edges.push(CompressedEdge { vertices: (a, b), curve: e.curve.clone() });
                orig.push(ei.index);
                edges.len() - 1
            });
            w.push(CompressedEdgeIndex { index: idx, orientation: ei.orientation });
        }
        boundaries.push(w);
    }
    let sub = CompressedShell {
        vertices,
        edges,
        faces: vec![CompressedFace { boundaries, orientation: face.orientation, surface: face.surface.clone() }],
    };
    (sub, orig)
}

/// Mesh a shell. Falls back to face by face when the whole shell takes too
/// long, skipping the faces that never finish.
fn mesh_shell(shell: CShell, tol: f64, label: &str, cancel: &AtomicBool, notes: &mut Vec<String>) -> Option<ShellOut> {
    let whole = shell.clone();
    if let Some(out) = run_with_timeout(SHELL_SECS, move || extract(&whole.robust_triangulation(tol))) {
        return Some(out);
    }
    notes.push(format!("{label}: meshing was slow, meshed face by face."));
    let mut out = ShellOut { edges: vec![vec![]; shell.edges.len()], faces: vec![] };
    let mut skipped = 0;
    for fi in 0..shell.faces.len() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let (sub, orig) = single_face(&shell, fi);
        match run_with_timeout(FACE_SECS, move || extract(&sub.robust_triangulation(tol))) {
            Some(mut o) if o.faces.len() == 1 => {
                for (k, pts) in o.edges.drain(..).enumerate() {
                    if out.edges[orig[k]].is_empty() {
                        out.edges[orig[k]] = pts;
                    }
                }
                let (fm, used) = o.faces.remove(0);
                out.faces.push((fm, used.into_iter().map(|k| orig[k]).collect()));
            }
            _ => {
                skipped += 1;
                out.faces.push((FaceMesh::default(), vec![]));
            }
        }
    }
    if skipped > 0 {
        notes.push(format!("{label}: {skipped} face(s) could not be meshed and are missing."));
    }
    Some(out)
}

/// Load a STEP file. Units are taken as millimetres. Reports progress and
/// stops early when `cancel` is set. Parts of an assembly are placed where the
/// assembly puts them.
pub fn load_with(path: &Path, progress: &mut dyn FnMut(String), cancel: &AtomicBool) -> Result<Geometry, String> {
    progress("Reading file".into());
    let text = std::fs::read_to_string(path).map_err(|e| format!("{e}"))?;
    progress("Parsing STEP".into());
    let table = Table::from_step(&text).ok_or("The file is not a readable STEP file.")?;
    if table.shell.is_empty() {
        return Err("No solid or shell found in the STEP file.".into());
    }
    progress("Reading assembly".into());
    let placed = assembly::resolve(&text);

    // Mesh every shell once, in a stable order.
    let mut geo = Geometry::default();
    let mut failed = 0usize;
    let mut ids: Vec<u64> = table.shell.keys().copied().collect();
    ids.sort_unstable();
    let total = ids.len();
    let mut meshed: HashMap<u64, ShellOut> = HashMap::new();
    for (n, id) in ids.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("Import cancelled.".into());
        }
        progress(format!("Meshing {} of {}", n + 1, total));
        let Some(shell) = table.shell.get(id) else { continue };
        let Ok(cshell) = table.to_compressed_shell(shell) else {
            failed += 1;
            continue;
        };
        // Tolerance from the size of the shell.
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for v in &cshell.vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        let diag = (0..3).map(|k| (hi[k] - lo[k]).max(0.0).powi(2)).sum::<f64>().sqrt();
        let tol = (diag * 0.0015).max(1e-3);
        let label = format!("Shell {}", n + 1);
        let Some(out) = mesh_shell(cshell, tol, &label, cancel, &mut geo.notes) else {
            return Err("Import cancelled.".into());
        };
        meshed.insert(*id, out);
    }

    // Place the shells: once per instance, loose shells where they are.
    let mut instances: Vec<(u64, assembly::M)> = vec![];
    let mut used: std::collections::HashSet<u64> = Default::default();
    for inst in placed.iter().flatten() {
        for s in &inst.shells {
            if meshed.contains_key(s) {
                instances.push((*s, inst.m));
                used.insert(*s);
            }
        }
    }
    let n_inst = instances.len();
    for id in &ids {
        if meshed.contains_key(id) && !used.contains(id) {
            instances.push((*id, assembly::IDENTITY));
        }
    }
    if placed.is_some() {
        geo.notes.push(format!("Assembly: {} part instance(s) placed.", n_inst));
    }
    for (id, m) in instances {
        let Some(out) = meshed.get(&id) else { continue };
        let (face_base, edge_base) = (geo.faces.len(), geo.edges.len());
        for pts in &out.edges {
            geo.edges.push(EdgePoly { pts: pts.iter().map(|p| assembly::apply(&m, *p)).collect(), faces: vec![] });
        }
        for (fi, (fm, used_edges)) in out.faces.iter().enumerate() {
            let mut f = FaceMesh::default();
            f.tris = fm.tris.iter().map(|t| t.map(|p| assembly::apply(&m, p))).collect();
            f.nors = fm.nors.iter().map(|t| t.map(|n| unit(assembly::apply_dir(&m, n)))).collect();
            geo.faces.push(f);
            for ei in used_edges {
                if let Some(e) = geo.edges.get_mut(edge_base + ei) {
                    if !e.faces.contains(&(face_base + fi)) {
                        e.faces.push(face_base + fi);
                    }
                }
            }
        }
    }
    if failed > 0 {
        geo.notes.push(format!("{failed} shell(s) use geometry that could not be read."));
    }
    if geo.faces.iter().all(|f| f.tris.is_empty()) && geo.edges.iter().all(|e| e.pts.is_empty()) {
        return Err("No geometry could be read from the STEP file.".into());
    }
    geo.compute_bounds();
    Ok(geo)
}

/// Blocking load, for tests.
#[allow(dead_code)]
pub fn load(path: &Path) -> Result<Geometry, String> {
    load_with(path, &mut |_| {}, &AtomicBool::new(false))
}

enum Msg {
    Status(String),
    Done(Result<Geometry, String>),
}

/// A STEP import running on a background thread.
pub struct ImportJob {
    rx: mpsc::Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    pub path: PathBuf,
    pub status: String,
}

impl ImportJob {
    pub fn spawn(path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (c, p) = (cancel.clone(), path.clone());
        let tx2 = tx.clone();
        // If the thread cannot start, the receiver sees the sender drop and
        // reports the import as stopped.
        let _ = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
            let mut report = |s: String| {
                let _ = tx.send(Msg::Status(s));
            };
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load_with(&p, &mut report, &c)))
                .unwrap_or_else(|_| Err("The STEP reader crashed on this file.".into()));
            let _ = tx2.send(Msg::Done(r));
        });
        Self { rx, cancel, path, status: "Starting".into() }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Latest status; the result once the import has finished.
    pub fn poll(&mut self) -> Option<Result<Geometry, String>> {
        loop {
            match self.rx.try_recv() {
                Ok(Msg::Status(s)) => self.status = s,
                Ok(Msg::Done(r)) => return Some(r),
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => return Some(Err("The import stopped unexpectedly.".into())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// Loads the file named by CAMCRAFT_STEP. Run with --ignored.
    #[test]
    #[ignore]
    fn load_sample_file() {
        let Ok(p) = std::env::var("CAMCRAFT_STEP") else { return };
        let t = std::time::Instant::now();
        let g = super::load(std::path::Path::new(&p)).unwrap();
        println!(
            "notes {:?} faces {} edges {} tris {} min {:?} max {:?} in {:?}",
            g.notes,
            g.faces.len(),
            g.edges.len(),
            g.tri_count(),
            g.min,
            g.max,
            t.elapsed()
        );
        assert!(g.tri_count() > 0);
    }
}
