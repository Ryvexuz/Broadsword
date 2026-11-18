//! Mesh tests: the OBJ reader (accepted forms and every refusal with its line number) and the procedural shapes.

use render::mesh::{WHEEL_HALF_WIDTH, WHEEL_RADIUS};
use render::{car_body_mesh, car_mesh, car_wheel_centres, ground_plane_mesh, wheel_mesh, Mesh, Vec3};

fn err(text: &str) -> (usize, String) {
    let e = Mesh::from_obj(text).expect_err("this input must be refused");
    (e.line, e.message)
}

#[test]
fn the_fixture_cube_is_read_and_its_quads_are_split() {
    let m = Mesh::from_obj(include_str!("fixtures/cube.obj")).unwrap();
    m.validate().unwrap();
    assert_eq!(m.triangles.len(), 12, "6 quads give 12 triangles");
    // 8 positions, but each is used with 3 different normals: 24 distinct (position, normal) vertices.
    assert_eq!(m.positions.len(), 24);
    assert_eq!(m.normals.len(), 24);
    assert!(m.normals.iter().all(|n| (n.length() - 1.0).abs() < 1e-6));
    // Every triangle's geometric normal agrees with its vertex normals, so the winding is outward.
    for t in &m.triangles {
        let p = t.map(|i| m.positions[i as usize]);
        let g = (p[1] - p[0]).cross(p[2] - p[0]).normalized();
        for &i in t {
            assert!(g.dot(m.normals[i as usize]) > 0.999, "triangle {t:?} faces the wrong way");
        }
    }
    assert!((signed_volume(&m) - 8.0).abs() < 1e-9, "a 2 x 2 x 2 cube has volume 8");
}

#[test]
fn all_three_vertex_forms_and_triangles_or_quads_are_accepted() {
    let tri = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let m = Mesh::from_obj(tri).unwrap();
    assert_eq!((m.positions.len(), m.normals.len(), m.triangles.len()), (3, 0, 1), "no normals: flat shading");

    let with_vn = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvn 0 0 2\nf 1//1 2//1 3//1\n";
    let m = Mesh::from_obj(with_vn).unwrap();
    assert_eq!(m.normals, vec![Vec3::new(0.0, 0.0, 1.0); 3], "normals are normalised");

    let with_vt = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1 4/1/1 # a quad\n";
    let m = Mesh::from_obj(with_vt).unwrap();
    assert_eq!(m.triangles, vec![[0, 1, 2], [0, 2, 3]], "a quad is split on its first diagonal");

    // Comments, blank lines, CRLF, and the harmless directives.
    let noisy = "# a comment\r\n\r\no thing\r\ng group\r\ns off\r\nmtllib x.mtl\r\nusemtl red\r\nv 0 0 0\r\nv 1 0 0 # trailing\r\nv 0 1 0\r\nf 1 2 3\r\n";
    assert_eq!(Mesh::from_obj(noisy).unwrap().triangles.len(), 1);
}

#[test]
fn every_refusal_names_its_line() {
    // (input, expected line, a word the message must contain)
    let cases: [(&str, usize, &str); 16] = [
        ("v 0 0\nf 1 1 1\n", 1, "exactly 3"),
        ("v 0 0 0\nv 1 0 0\nv 0 1 x\nf 1 2 3\n", 3, "not a finite number"),
        ("v 0 0 0\nv 1 0 0\nv 0 1 NaN\nf 1 2 3\n", 3, "not a finite number"),
        ("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2\n", 4, "3 or 4"),
        ("v 0 0 0\nv 1 0 0\nv 0 1 0\nv 1 1 0\nv 2 2 0\nf 1 2 3 4 5\n", 6, "3 or 4"),
        ("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 4\n", 4, "not defined yet"),
        ("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 0\n", 4, "index 0"),
        ("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 -1\n", 4, "negative"),
        ("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 x\n", 4, "not a positive"),
        ("v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nf 1/1 2/1 3/1\n", 5, "only v, v//vn and v/vt/vn"),
        ("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1/1/1/1 2 3\n", 4, "only v, v//vn and v/vt/vn"),
        ("v 0 0 0\nv 1 0 0\nv 0 1 0\nvn 0 0 1\nf 1//1 2 3\n", 5, "mixed"),
        ("v 0 0 0\nv 1 0 0\nv 0 1 0\nvn 0 0 1\nf 1//2 2//1 3//1\n", 5, "normal index 2"),
        ("v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvn 0 0 1\nf 1/2/1 2/1/1 3/1/1\n", 6, "texture index 2"),
        ("v 0 0 0\nv 1 0 0\nl 1 2\n", 3, "unsupported directive `l`"),
        ("v 0 0 0\nvn 0 1\n", 2, "exactly 3"),
    ];
    for (text, line, word) in cases {
        let (got_line, msg) = err(text);
        assert_eq!(got_line, line, "{text:?}: {msg}");
        assert!(msg.contains(word), "{text:?}: message {msg:?} should mention {word:?}");
    }
    // A file with no faces is refused as a whole (line 0).
    assert_eq!(err("v 0 0 0\n"), (0, "the file has no faces".to_string()));
    assert_eq!(err("").0, 0);
    // And the line number appears in the text a person reads.
    let e = Mesh::from_obj("v 0 0 0\nf 1 2 3\n").unwrap_err();
    assert!(e.to_string().starts_with("OBJ line 2:"), "{e}");
}

/// Volume enclosed by a closed, outward-facing mesh: the sum of signed tetrahedra against the origin.
/// Positive only if the triangles wind counter-clockwise seen from outside.
fn signed_volume(m: &Mesh) -> f64 {
    m.triangles
        .iter()
        .map(|t| {
            let p = t.map(|i| {
                let v = m.positions[i as usize];
                [v.x as f64, v.y as f64, v.z as f64]
            });
            let c = [p[1][1] * p[2][2] - p[1][2] * p[2][1], p[1][2] * p[2][0] - p[1][0] * p[2][2], p[1][0] * p[2][1] - p[1][1] * p[2][0]];
            (p[0][0] * c[0] + p[0][1] * c[1] + p[0][2] * c[2]) / 6.0
        })
        .sum()
}

fn aabb(m: &Mesh) -> (Vec3, Vec3) {
    let first = m.positions[0];
    m.positions.iter().fold((first, first), |(lo, hi), p| {
        (Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z)), Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z)))
    })
}

#[test]
fn the_car_body_is_two_closed_outward_boxes_of_the_stated_size() {
    let m = car_body_mesh();
    m.validate().unwrap();
    assert_eq!(m.triangles.len(), 24);
    // Body 4.0 x 0.55 x 1.8 and cabin 1.7 x 0.5 x 1.5, as the doc comment says.
    let want = 4.0 * 0.55 * 1.8 + 1.7 * 0.5 * 1.5;
    assert!((signed_volume(&m) - want).abs() < 1e-5, "volume {} vs {want}", signed_volume(&m));
    let (lo, hi) = aabb(&m);
    for (got, want) in [(lo.x, -2.0), (lo.y, 0.3), (lo.z, -0.9), (hi.x, 2.0), (hi.y, 1.35), (hi.z, 0.9)] {
        assert!((got - want).abs() < 1e-6, "bounding box {lo:?} {hi:?}");
    }
    for t in &m.triangles {
        let p = t.map(|i| m.positions[i as usize]);
        let g = (p[1] - p[0]).cross(p[2] - p[0]).normalized();
        assert!(t.iter().all(|&i| g.dot(m.normals[i as usize]) > 0.999));
    }
}

#[test]
fn a_wheel_is_a_closed_outward_cylinder_for_any_segment_count() {
    assert!(wheel_mesh(2).is_err() && wheel_mesh(0).is_err());
    for n in [3usize, 4, 8, 24, 64] {
        let m = wheel_mesh(n).unwrap();
        m.validate().unwrap();
        assert_eq!(m.triangles.len(), 4 * n, "{n} segments: 2n on the side and 2n on the caps");
        // A regular n-gon of circumradius r has area n/2 r^2 sin(2 pi / n).
        let want = n as f64 / 2.0 * (WHEEL_RADIUS as f64).powi(2) * (std::f64::consts::TAU / n as f64).sin() * 2.0 * WHEEL_HALF_WIDTH as f64;
        assert!((signed_volume(&m) - want).abs() < 1e-5, "{n} segments: volume {} vs {want}", signed_volume(&m));
        assert!(m.normals.iter().all(|v| (v.length() - 1.0).abs() < 1e-5));
        // Each triangle's true normal points the way its vertex normals do. On the side the vertex normals are the
        // radial directions at the two ends of the facet, up to half a segment (pi / n) from the facet's own normal.
        let worst = (std::f32::consts::PI / n as f32).cos() - 1e-3;
        for t in &m.triangles {
            let p = t.map(|i| m.positions[i as usize]);
            let g = (p[1] - p[0]).cross(p[2] - p[0]).normalized();
            for &i in t {
                assert!(g.dot(m.normals[i as usize]) >= worst, "{n}: triangle {t:?}");
            }
        }
    }
}

#[test]
fn the_whole_car_has_four_wheels_on_the_ground_and_valid_indices() {
    let m = car_mesh(24).unwrap();
    m.validate().unwrap();
    assert_eq!(m.triangles.len(), 24 + 4 * 96);
    assert_eq!(m.normals.len(), m.positions.len());
    let (lo, hi) = aabb(&m);
    assert!((lo.y - 0.0).abs() < 1e-6, "the wheels touch y = 0, found {}", lo.y);
    assert!((hi.x - 2.0).abs() < 1e-6 && (hi.y - 1.35).abs() < 1e-6);
    assert!((hi.z - 1.0).abs() < 1e-6, "the wheels stick out 0.1 either side of the body");
    for c in car_wheel_centres() {
        assert!((c.y - WHEEL_RADIUS).abs() < 1e-7);
    }
    let one = wheel_mesh(24).unwrap();
    let want = signed_volume(&car_body_mesh()) + 4.0 * signed_volume(&one);
    assert!((signed_volume(&m) - want).abs() < 1e-4);
}

#[test]
fn the_ground_plane_faces_up() {
    let g = ground_plane_mesh(5.0);
    g.validate().unwrap();
    assert_eq!(g.triangles.len(), 2);
    for t in &g.triangles {
        let p = t.map(|i| g.positions[i as usize]);
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalized();
        assert!((n.y - 1.0).abs() < 1e-6, "{n:?}");
    }
    let (lo, hi) = aabb(&g);
    assert_eq!((lo, hi), (Vec3::new(-5.0, 0.0, -5.0), Vec3::new(5.0, 0.0, 5.0)));
}

#[test]
fn validate_catches_bad_meshes() {
    let mut m = ground_plane_mesh(1.0);
    assert!(m.validate().is_ok());
    m.triangles.push([0, 1, 4]);
    assert!(m.validate().unwrap_err().contains("vertex 4"));
    let mut m = ground_plane_mesh(1.0);
    m.normals.pop();
    assert!(m.validate().unwrap_err().contains("normals"));
    m.normals.clear(); // no normals at all is fine: flat shading
    assert!(m.validate().is_ok());
}
