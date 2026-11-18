//! Small scene helpers that produce line lists.

use crate::math::Vec3;

/// The 12 edges of a cube with edge length `size`, centred at `center`, turned `rotation_y` radians about
/// the vertical axis through its centre.
pub fn wire_cube(center: Vec3, size: f32, rotation_y: f32) -> Vec<(Vec3, Vec3)> {
    let h = size * 0.5;
    let (s, c) = rotation_y.sin_cos();
    let corner = |i: usize| {
        let x = if i & 1 == 0 { -h } else { h };
        let y = if i & 2 == 0 { -h } else { h };
        let z = if i & 4 == 0 { -h } else { h };
        // Rotation about +Y.
        Vec3::new(center.x + c * x + s * z, center.y + y, center.z - s * x + c * z)
    };
    let mut edges = Vec::with_capacity(12);
    for i in 0..8usize {
        for bit in [1usize, 2, 4] {
            let j = i | bit;
            if j != i {
                edges.push((corner(i), corner(j)));
            }
        }
    }
    edges
}
