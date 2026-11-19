//! Assembly structure of a STEP file: where every part instance sits.
//!
//! CAD systems store each part once, in its own coordinates, and place copies
//! of it with transformations between shape representations (or with mapped
//! items). Reading only the solids gives every part at the origin. This reads
//! those placements and returns, for each solid, the matrix of every instance.

use super::step::V3;
use std::collections::BTreeMap;

/// 3x4 affine matrix. Rows give x', y', z'; the last column is the shift.
pub type M = [[f64; 4]; 3];

pub const IDENTITY: M = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];

/// `a` applied after `b`.
pub fn mul(a: &M, b: &M) -> M {
    let mut r = [[0.0; 4]; 3];
    for i in 0..3 {
        for j in 0..4 {
            r[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j] + if j == 3 { a[i][3] } else { 0.0 };
        }
    }
    r
}

pub fn inverse(m: &M) -> M {
    // Rotation part is orthonormal for placements, so its inverse is the transpose.
    let mut r = [[0.0; 4]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = m[j][i];
        }
    }
    for i in 0..3 {
        r[i][3] = -(r[i][0] * m[0][3] + r[i][1] * m[1][3] + r[i][2] * m[2][3]);
    }
    r
}

pub fn apply(m: &M, p: V3) -> V3 {
    [
        m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2] + m[0][3],
        m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2] + m[1][3],
        m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2] + m[2][3],
    ]
}

pub fn apply_dir(m: &M, v: V3) -> V3 {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// A solid (its shell ids) placed in the model.
pub struct Instance {
    pub shells: Vec<u64>,
    pub m: M,
}

#[derive(Clone, Debug)]
enum P {
    Ref(u32),
    Num(f64),
    /// Strings and enumerations, whose value is not needed.
    Str,
    List(Vec<P>),
    Null,
}

type Ent = Vec<(String, Vec<P>)>;

struct Cur<'a> {
    b: &'a [u8],
    i: usize,
}

impl Cur<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&self) -> u8 {
        self.b.get(self.i).copied().unwrap_or(0)
    }

    fn ident(&mut self) -> String {
        let s = self.i;
        while self.i < self.b.len() && (self.b[self.i].is_ascii_alphanumeric() || self.b[self.i] == b'_') {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.b[s..self.i]).to_uppercase()
    }

    fn args(&mut self) -> Vec<P> {
        // At '('.
        self.i += 1;
        let mut out = vec![];
        loop {
            self.ws();
            match self.peek() {
                b')' => {
                    self.i += 1;
                    break;
                }
                b',' => self.i += 1,
                0 => break,
                _ => out.push(self.param()),
            }
        }
        out
    }

    fn param(&mut self) -> P {
        self.ws();
        match self.peek() {
            b'#' => {
                self.i += 1;
                let s = self.i;
                while self.peek().is_ascii_digit() {
                    self.i += 1;
                }
                P::Ref(std::str::from_utf8(&self.b[s..self.i]).ok().and_then(|t| t.parse().ok()).unwrap_or(0))
            }
            b'\'' => {
                self.i += 1;
                let s = self.i;
                while self.i < self.b.len() {
                    if self.b[self.i] == b'\'' {
                        if self.b.get(self.i + 1) == Some(&b'\'') {
                            self.i += 2;
                            continue;
                        }
                        break;
                    }
                    self.i += 1;
                }
                let _ = s;
                self.i += 1;
                P::Str
            }
            b'(' => P::List(self.args()),
            b'$' | b'*' => {
                self.i += 1;
                P::Null
            }
            b'.' if self.b.get(self.i + 1).is_some_and(|c| c.is_ascii_alphabetic()) => {
                self.i += 1;
                self.ident();
                if self.peek() == b'.' {
                    self.i += 1;
                }
                P::Str
            }
            c if c.is_ascii_digit() || c == b'-' || c == b'+' || c == b'.' => {
                let s = self.i;
                while self.i < self.b.len() && matches!(self.b[self.i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                    self.i += 1;
                }
                P::Num(std::str::from_utf8(&self.b[s..self.i]).ok().and_then(|t| t.parse().ok()).unwrap_or(0.0))
            }
            c if c.is_ascii_alphabetic() => {
                // Typed value such as LENGTH_MEASURE(1.0).
                self.ident();
                self.ws();
                if self.peek() == b'(' { P::List(self.args()) } else { P::Null }
            }
            _ => {
                self.i += 1;
                P::Null
            }
        }
    }
}

/// Words that mark the statements this module needs.
const WANTED: [&str; 9] = [
    "CARTESIAN_POINT",
    "DIRECTION",
    "AXIS2_PLACEMENT_3D",
    "ITEM_DEFINED_TRANSFORMATION",
    "REPRESENTATION",
    "MANIFOLD_SOLID_BREP",
    "BREP_WITH_VOIDS",
    "MAPPED_ITEM",
    "ORIENTED_CLOSED_SHELL",
];

fn parse(text: &str) -> BTreeMap<u32, Ent> {
    let mut ents: BTreeMap<u32, Ent> = BTreeMap::new();
    let Some(start) = text.find("DATA;") else { return ents };
    let b = text.as_bytes();
    let mut i = start + 5;
    while i < b.len() {
        // One statement, up to ';' outside quotes.
        let s = i;
        let mut quote = false;
        while i < b.len() {
            match b[i] {
                b'\'' => quote = !quote,
                b';' if !quote => break,
                _ => {}
            }
            i += 1;
        }
        let stmt = text[s..i.min(b.len())].trim();
        i += 1;
        if stmt.starts_with("ENDSEC") {
            break;
        }
        let Some(rest) = stmt.strip_prefix('#') else { continue };
        let Some(eq) = rest.find('=') else { continue };
        let Ok(id) = rest[..eq].trim().parse::<u32>() else { continue };
        let rhs = rest[eq + 1..].trim();
        if !WANTED.iter().any(|w| rhs.contains(w)) && !rhs.contains("SHELL_BASED_SURFACE_MODEL") {
            continue;
        }
        let mut c = Cur { b: rhs.as_bytes(), i: 0 };
        let mut ent: Ent = vec![];
        if c.peek() == b'(' {
            // Complex entity: ( NAME(args) NAME(args) ... )
            c.i += 1;
            loop {
                c.ws();
                if c.peek() == b')' || c.peek() == 0 {
                    break;
                }
                let name = c.ident();
                c.ws();
                let args = if c.peek() == b'(' { c.args() } else { vec![] };
                if name.is_empty() {
                    c.i += 1;
                    continue;
                }
                ent.push((name, args));
            }
        } else {
            let name = c.ident();
            c.ws();
            let args = if c.peek() == b'(' { c.args() } else { vec![] };
            ent.push((name, args));
        }
        ents.insert(id, ent);
    }
    ents
}

fn kind<'a>(e: &'a Ent, name: &str) -> Option<&'a Vec<P>> {
    e.iter().find(|k| k.0 == name).map(|k| &k.1)
}

fn num_list(p: &P) -> Option<V3> {
    let P::List(l) = p else { return None };
    let v: Vec<f64> = l.iter().filter_map(|x| if let P::Num(n) = x { Some(*n) } else { None }).collect();
    (v.len() >= 3).then(|| [v[0], v[1], v[2]])
}

fn point_or_dir(ents: &BTreeMap<u32, Ent>, p: Option<&P>, name: &str) -> Option<V3> {
    let P::Ref(r) = p? else { return None };
    num_list(kind(ents.get(r)?, name)?.get(1)?)
}

fn norm(v: V3) -> V3 {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l < 1e-12 { v } else { [v[0] / l, v[1] / l, v[2] / l] }
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn placement(ents: &BTreeMap<u32, Ent>, id: u32) -> Option<M> {
    let e = kind(ents.get(&id)?, "AXIS2_PLACEMENT_3D")?;
    let loc = point_or_dir(ents, e.get(1), "CARTESIAN_POINT")?;
    let z = norm(point_or_dir(ents, e.get(2), "DIRECTION").unwrap_or([0.0, 0.0, 1.0]));
    let rx = point_or_dir(ents, e.get(3), "DIRECTION").unwrap_or(if z[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] });
    let d = rx[0] * z[0] + rx[1] * z[1] + rx[2] * z[2];
    let x = norm([rx[0] - z[0] * d, rx[1] - z[1] * d, rx[2] - z[2] * d]);
    let y = cross(z, x);
    Some([[x[0], y[0], z[0], loc[0]], [x[1], y[1], z[1], loc[1]], [x[2], y[2], z[2], loc[2]]])
}

fn refs(p: Option<&P>) -> Vec<u32> {
    match p {
        Some(P::List(l)) => l.iter().filter_map(|x| if let P::Ref(r) = x { Some(*r) } else { None }).collect(),
        Some(P::Ref(r)) => vec![*r],
        _ => vec![],
    }
}

/// Shell ids of a solid or shell model.
fn solid_shells(ents: &BTreeMap<u32, Ent>, id: u32) -> Option<Vec<u64>> {
    let e = ents.get(&id)?;
    if let Some(a) = kind(e, "MANIFOLD_SOLID_BREP") {
        return Some(refs(a.get(1)).into_iter().map(u64::from).collect());
    }
    if let Some(a) = kind(e, "BREP_WITH_VOIDS") {
        let mut v: Vec<u64> = refs(a.get(1)).into_iter().map(u64::from).collect();
        for o in refs(a.get(2)) {
            // ORIENTED_CLOSED_SHELL(name, *, shell, orientation)
            if let Some(oe) = ents.get(&o).and_then(|e| kind(e, "ORIENTED_CLOSED_SHELL")) {
                v.extend(refs(oe.get(2)).into_iter().map(u64::from));
            }
        }
        return Some(v);
    }
    if let Some(a) = kind(e, "SHELL_BASED_SURFACE_MODEL") {
        // Items are open shells, possibly through ORIENTED_OPEN_SHELL, which
        // the shell table does not list separately.
        return Some(refs(a.get(1)).into_iter().map(u64::from).collect());
    }
    None
}

enum Item {
    Solid(Vec<u64>),
    Child(u32, M),
}

/// All instances of solids, with the matrix that places each in the
/// coordinates of the whole model. None when the file has no assembly
/// structure, in which case the solids are used where they are.
pub fn resolve(text: &str) -> Option<Vec<Instance>> {
    let ents = parse(text);
    if ents.is_empty() {
        return None;
    }

    // Items of every shape representation.
    let mut items: BTreeMap<u32, Vec<Item>> = BTreeMap::new();
    for (&id, e) in &ents {
        let Some(args) = e.iter().find(|k| k.0.ends_with("REPRESENTATION") && matches!(k.1.get(1), Some(P::List(_)))).map(|k| &k.1) else {
            continue;
        };
        let mut v = vec![];
        for r in refs(args.get(1)) {
            if let Some(s) = solid_shells(&ents, r) {
                v.push(Item::Solid(s));
                continue;
            }
            // Mapped item: (name, mapping_source, mapping_target).
            if let Some(mi) = ents.get(&r).and_then(|x| kind(x, "MAPPED_ITEM")) {
                let map_id = refs(mi.get(1)).first().copied();
                let target = refs(mi.get(2)).first().and_then(|t| placement(&ents, *t));
                if let (Some(mid), Some(target)) = (map_id, target) {
                    // REPRESENTATION_MAP(mapping_origin, mapped_representation)
                    if let Some(rm) = ents.get(&mid).and_then(|x| kind(x, "REPRESENTATION_MAP")) {
                        let origin = refs(rm.get(0)).first().and_then(|o| placement(&ents, *o)).unwrap_or(IDENTITY);
                        if let Some(child) = refs(rm.get(1)).first() {
                            v.push(Item::Child(*child, mul(&target, &inverse(&origin))));
                        }
                    }
                }
            }
        }
        items.insert(id, v);
    }

    // Relationships between representations: parent (rep_1) to child (rep_2).
    let mut has_parent: BTreeMap<u32, bool> = BTreeMap::new();
    for (_, e) in &ents {
        let Some(base) = kind(e, "REPRESENTATION_RELATIONSHIP").or_else(|| kind(e, "SHAPE_REPRESENTATION_RELATIONSHIP")) else {
            continue;
        };
        let (Some(P::Ref(a)), Some(P::Ref(b))) = (base.get(2), base.get(3)) else { continue };
        let m = match kind(e, "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION").and_then(|t| refs(t.first()).first().copied()) {
            Some(t) => {
                let idt = ents.get(&t).and_then(|x| kind(x, "ITEM_DEFINED_TRANSFORMATION"));
                match idt {
                    Some(a) => {
                        let p1 = refs(a.get(2)).first().and_then(|x| placement(&ents, *x));
                        let p2 = refs(a.get(3)).first().and_then(|x| placement(&ents, *x));
                        match (p1, p2) {
                            (Some(p1), Some(p2)) => mul(&p1, &inverse(&p2)),
                            _ => IDENTITY,
                        }
                    }
                    None => IDENTITY,
                }
            }
            None => IDENTITY,
        };
        items.entry(*a).or_default().push(Item::Child(*b, m));
        has_parent.insert(*b, true);
    }

    let mut roots: Vec<u32> = items.keys().copied().filter(|k| !has_parent.contains_key(k)).collect();
    roots.sort_unstable();
    let mut out: Vec<Instance> = vec![];
    fn walk(items: &BTreeMap<u32, Vec<Item>>, rep: u32, m: &M, depth: usize, out: &mut Vec<Instance>) {
        if depth > 64 {
            return;
        }
        for it in items.get(&rep).into_iter().flatten() {
            match it {
                Item::Solid(s) => out.push(Instance { shells: s.clone(), m: *m }),
                Item::Child(c, cm) => walk(items, *c, &mul(m, cm), depth + 1, out),
            }
        }
    }
    for r in roots {
        walk(&items, r, &IDENTITY, 0, &mut out);
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: &str = "DATA;
#1 = CARTESIAN_POINT('', (0., 0., 0.));
#2 = DIRECTION('', (0., 0., 1.));
#3 = DIRECTION('', (1., 0., 0.));
#4 = AXIS2_PLACEMENT_3D('', #1, #2, #3);
#5 = CARTESIAN_POINT('', (10., 20., 30.));
#6 = AXIS2_PLACEMENT_3D('', #5, #2, #3);
#10 = MANIFOLD_SOLID_BREP('part', #11);
#20 = ADVANCED_BREP_SHAPE_REPRESENTATION('', (#10, #4), #99);
#21 = SHAPE_REPRESENTATION('part', (#4), #99);
#22 = SHAPE_REPRESENTATION('asm', (#6), #99);
#30 = ITEM_DEFINED_TRANSFORMATION('', '', #6, #4);
#31 =( REPRESENTATION_RELATIONSHIP('', '', #22, #21) REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#30) SHAPE_REPRESENTATION_RELATIONSHIP() );
#32 = SHAPE_REPRESENTATION_RELATIONSHIP('', '', #21, #20);
ENDSEC;";

    #[test]
    fn places_a_part_through_the_assembly() {
        let inst = resolve(STEP).unwrap();
        assert_eq!(inst.len(), 1);
        assert_eq!(inst[0].shells, vec![11]);
        assert_eq!(apply(&inst[0].m, [1.0, 2.0, 3.0]), [11.0, 22.0, 33.0]);
    }

    #[test]
    fn matrix_inverse_and_product() {
        let m: M = [[0.0, -1.0, 0.0, 5.0], [1.0, 0.0, 0.0, 6.0], [0.0, 0.0, 1.0, 7.0]];
        let p = apply(&mul(&m, &inverse(&m)), [3.0, 4.0, 5.0]);
        assert!((p[0] - 3.0).abs() < 1e-12 && (p[1] - 4.0).abs() < 1e-12 && (p[2] - 5.0).abs() < 1e-12);
    }
}
