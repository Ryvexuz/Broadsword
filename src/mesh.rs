//! Triangle meshes: the `Mesh` type, a small OBJ reader, and procedural shapes (a car, a ground plane).
//!
//! Triangles are counter-clockwise seen from the outside (the OBJ convention); that is the front face.
//! Vertices are right-handed with +Y up.

use crate::math::Vec3;
use std::collections::HashMap;
use std::fmt;

/// Indexed triangles. `normals` is either empty (the renderer then shades each triangle flat, with its own
/// geometric normal) or has exactly one unit normal per position.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
}

/// An OBJ file was refused. `line` is 1-based; 0 means the whole file (for example, it has no faces).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ObjError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "OBJ: {}", self.message)
        } else {
            write!(f, "OBJ line {}: {}", self.line, self.message)
        }
    }
}

impl std::error::Error for ObjError {}

fn obj_err<T>(line: usize, message: impl Into<String>) -> Result<T, ObjError> {
    Err(ObjError { line, message: message.into() })
}

impl Mesh {
    /// Checks that the normals match the positions and every index is in range.
    pub fn validate(&self) -> Result<(), String> {
        if !self.normals.is_empty() && self.normals.len() != self.positions.len() {
            return Err(format!(
                "{} normals for {} positions (need none, or one per position)",
                self.normals.len(),
                self.positions.len()
            ));
        }
        let n = self.positions.len();
        for (i, t) in self.triangles.iter().enumerate() {
            if let Some(bad) = t.iter().find(|&&v| v as usize >= n) {
                return Err(format!("triangle {i} uses vertex {bad}, but there are only {n} positions"));
            }
        }
        Ok(())
    }

    /// Appends `other`, moved by `offset`. If exactly one of the two has normals, the result has none
    /// (a mesh is either fully smooth-shaded or fully flat).
    pub fn append(&mut self, other: &Mesh, offset: Vec3) {
        let base = self.positions.len() as u32;
        let keep_normals = (self.positions.is_empty() || !self.normals.is_empty())
            && (other.positions.is_empty() || !other.normals.is_empty());
        self.positions.extend(other.positions.iter().map(|&p| p + offset));
        if keep_normals {
            self.normals.extend(other.normals.iter().copied());
        } else {
            self.normals.clear();
        }
        self.triangles.extend(other.triangles.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
    }

    /// Reads a Wavefront OBJ. Supported: `v x y z`, `vn x y z`, `vt u v [w]` (read and ignored), and
    /// `f` with three or four vertices, each written `v`, `v//vn` or `v/vt/vn` (a quad is split into two
    /// triangles). Comments, and the lines `o`, `g`, `s`, `usemtl` and `mtllib`, are ignored. Everything
    /// else is refused with its line number: other directives, `v/vt`, negative or zero or out-of-range
    /// indices (an index must refer to something already defined), polygons with five or more vertices,
    /// non-finite numbers, and a file that mixes faces with and without normals. Vertices that are used
    /// with different normals are duplicated, so the result has one normal per position.
    pub fn from_obj(text: &str) -> Result<Mesh, ObjError> {
        let mut positions: Vec<Vec3> = Vec::new();
        let mut normals: Vec<Vec3> = Vec::new();
        let mut uv_count = 0usize;
        let mut mesh = Mesh::default();
        let mut out_of: HashMap<(usize, usize), u32> = HashMap::new();
        let mut with_normals: Option<bool> = None;

        for (i, raw) in text.lines().enumerate() {
            let line_no = i + 1;
            let line = raw.split('#').next().unwrap_or("").trim();
            let mut words = line.split_whitespace();
            let Some(key) = words.next() else { continue };
            let rest: Vec<&str> = words.collect();
            match key {
                "v" | "vn" => {
                    if rest.len() != 3 {
                        return obj_err(line_no, format!("`{key}` needs exactly 3 numbers, found {}", rest.len()));
                    }
                    let mut c = [0.0f32; 3];
                    for (k, w) in rest.iter().enumerate() {
                        c[k] = match w.parse::<f32>() {
                            Ok(x) if x.is_finite() => x,
                            _ => return obj_err(line_no, format!("`{w}` is not a finite number")),
                        };
                    }
                    let v = Vec3::new(c[0], c[1], c[2]);
                    if key == "v" {
                        positions.push(v);
                    } else {
                        normals.push(v.normalized());
                    }
                }
                "vt" => {
                    if rest.len() < 2 || rest.len() > 3 || rest.iter().any(|w| w.parse::<f32>().map_or(true, |x| !x.is_finite())) {
                        return obj_err(line_no, "`vt` needs 2 or 3 finite numbers");
                    }
                    uv_count += 1;
                }
                "f" => {
                    if rest.len() != 3 && rest.len() != 4 {
                        return obj_err(
                            line_no,
                            format!("a face needs 3 or 4 vertices, found {} (only triangles and quads)", rest.len()),
                        );
                    }
                    let mut ids = [0u32; 4];
                    for (k, w) in rest.iter().enumerate() {
                        let parts: Vec<&str> = w.split('/').collect();
                        let index = |s: &str, what: &str, count: usize| -> Result<usize, ObjError> {
                            match s.parse::<usize>() {
                                Ok(n) if n >= 1 && n <= count => Ok(n - 1),
                                Ok(0) => obj_err(line_no, format!("{what} index 0 (indices start at 1)")),
                                Ok(n) => obj_err(
                                    line_no,
                                    format!("{what} index {n} is not defined yet ({count} defined before this line)"),
                                ),
                                Err(_) => obj_err(
                                    line_no,
                                    format!("`{s}` is not a positive {what} index (negative indices are not supported)"),
                                ),
                            }
                        };
                        let (v, vn) = match parts.as_slice() {
                            [v] => (index(v, "vertex", positions.len())?, None),
                            [v, "", vn] => (index(v, "vertex", positions.len())?, Some(index(vn, "normal", normals.len())?)),
                            [v, vt, vn] => {
                                let v = index(v, "vertex", positions.len())?;
                                index(vt, "texture", uv_count)?;
                                (v, Some(index(vn, "normal", normals.len())?))
                            }
                            _ => {
                                return obj_err(
                                    line_no,
                                    format!("`{w}`: only v, v//vn and v/vt/vn vertex forms are supported"),
                                )
                            }
                        };
                        match with_normals {
                            None => with_normals = Some(vn.is_some()),
                            Some(expected) if expected != vn.is_some() => {
                                return obj_err(line_no, "faces with and without normals are mixed in one file")
                            }
                            _ => {}
                        }
                        let key = (v, vn.unwrap_or(usize::MAX));
                        ids[k] = *out_of.entry(key).or_insert_with(|| {
                            mesh.positions.push(positions[v]);
                            if let Some(n) = vn {
                                mesh.normals.push(normals[n]);
                            }
                            (mesh.positions.len() - 1) as u32
                        });
                    }
                    mesh.triangles.push([ids[0], ids[1], ids[2]]);
                    if rest.len() == 4 {
                        mesh.triangles.push([ids[0], ids[2], ids[3]]);
                    }
                }
                "o" | "g" | "s" | "usemtl" | "mtllib" => {}
                other => return obj_err(line_no, format!("unsupported directive `{other}`")),
            }
        }
        if mesh.triangles.is_empty() {
            return obj_err(0, "the file has no faces");
        }
        Ok(mesh)
    }
}

/// Adds one flat quad: corners `centre +- u*half_u +- v*half_v`, counter-clockwise seen from the side `normal`
/// points to (so `u x v` must equal `normal`).
fn push_quad(mesh: &mut Mesh, centre: Vec3, normal: Vec3, u: Vec3, v: Vec3, half_u: f32, half_v: f32) {
    let base = mesh.positions.len() as u32;
    let (u, v) = (u * half_u, v * half_v);
    for p in [centre - u - v, centre + u - v, centre + u + v, centre - u + v] {
        mesh.positions.push(p);
        mesh.normals.push(normal);
    }
    mesh.triangles.push([base, base + 1, base + 2]);
    mesh.triangles.push([base, base + 2, base + 3]);
}

/// An axis-aligned box with flat faces (24 vertices, 12 triangles).
fn push_box(mesh: &mut Mesh, centre: Vec3, half: Vec3) {
    let (x, y, z) = (Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0));
    push_quad(mesh, centre + x * half.x, x, y, z, half.y, half.z);
    push_quad(mesh, centre - x * half.x, -x, z, y, half.z, half.y);
    push_quad(mesh, centre + y * half.y, y, z, x, half.z, half.x);
    push_quad(mesh, centre - y * half.y, -y, x, z, half.x, half.z);
    push_quad(mesh, centre + z * half.z, z, x, y, half.x, half.y);
    push_quad(mesh, centre - z * half.z, -z, y, x, half.y, half.x);
}

/// A flat square ground plane at y = 0, `half_extent` out in every direction, facing +Y.
pub fn ground_plane_mesh(half_extent: f32) -> Mesh {
    let mut m = Mesh::default();
    push_quad(
        &mut m,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(1.0, 0.0, 0.0),
        half_extent,
        half_extent,
    );
    m
}

/// The car's body: a long low box and a cabin box on top. The car faces +X, is 4.0 long, 1.8 wide, and its
/// wheels (see `wheel_mesh`) touch y = 0.
pub fn car_body_mesh() -> Mesh {
    let mut m = Mesh::default();
    push_box(&mut m, Vec3::new(0.0, 0.575, 0.0), Vec3::new(2.0, 0.275, 0.9));
    push_box(&mut m, Vec3::new(0.05, 1.1, 0.0), Vec3::new(0.85, 0.25, 0.75));
    m
}

pub const WHEEL_RADIUS: f32 = 0.35;
pub const WHEEL_HALF_WIDTH: f32 = 0.15;

/// Centres of the four wheels (front left, front right, rear left, rear right), for a car at the origin.
pub fn car_wheel_centres() -> [Vec3; 4] {
    let y = WHEEL_RADIUS;
    [
        Vec3::new(1.25, y, 0.85),
        Vec3::new(1.25, y, -0.85),
        Vec3::new(-1.25, y, 0.85),
        Vec3::new(-1.25, y, -0.85),
    ]
}

/// A wheel: a cylinder around the Z axis, centred on the origin, with `segments` sides (at least 3).
/// The side has radial normals; the two caps have axial normals.
pub fn wheel_mesh(segments: usize) -> Result<Mesh, String> {
    if segments < 3 {
        return Err(format!("a wheel needs at least 3 segments, got {segments}"));
    }
    let mut m = Mesh::default();
    let ring = |i: usize| {
        let a = std::f32::consts::TAU * (i % segments) as f32 / segments as f32;
        let (s, c) = a.sin_cos();
        (c, s)
    };
    // Side: one quad per segment, counter-clockwise from outside.
    for i in 0..segments {
        let ((c0, s0), (c1, s1)) = (ring(i), ring(i + 1));
        let base = m.positions.len() as u32;
        let (r, w) = (WHEEL_RADIUS, WHEEL_HALF_WIDTH);
        for (c, s, z) in [(c0, s0, -w), (c1, s1, -w), (c1, s1, w), (c0, s0, w)] {
            m.positions.push(Vec3::new(c * r, s * r, z));
            m.normals.push(Vec3::new(c, s, 0.0));
        }
        m.triangles.push([base, base + 1, base + 2]);
        m.triangles.push([base, base + 2, base + 3]);
    }
    // Caps: a fan from the centre.
    for (z, nz) in [(WHEEL_HALF_WIDTH, 1.0f32), (-WHEEL_HALF_WIDTH, -1.0)] {
        let centre = m.positions.len() as u32;
        m.positions.push(Vec3::new(0.0, 0.0, z));
        m.normals.push(Vec3::new(0.0, 0.0, nz));
        for i in 0..segments {
            let (c, s) = ring(i);
            m.positions.push(Vec3::new(c * WHEEL_RADIUS, s * WHEEL_RADIUS, z));
            m.normals.push(Vec3::new(0.0, 0.0, nz));
        }
        for i in 0..segments as u32 {
            let (a, b) = (centre + 1 + i, centre + 1 + (i + 1) % segments as u32);
            // Increasing angle is counter-clockwise seen from +Z.
            m.triangles.push(if nz > 0.0 { [centre, a, b] } else { [centre, b, a] });
        }
    }
    Ok(m)
}

/// The whole car as one mesh: body, cabin and four wheels with `wheel_segments` sides each.
pub fn car_mesh(wheel_segments: usize) -> Result<Mesh, String> {
    let mut m = car_body_mesh();
    let wheel = wheel_mesh(wheel_segments)?;
    for c in car_wheel_centres() {
        m.append(&wheel, c);
    }
    Ok(m)
}
