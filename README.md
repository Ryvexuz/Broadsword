# render

Broadsword's renderer, Stage 4 of the development plan. It uses only the Rust standard library, with no crates.

All drawing happens in software, into an in-memory framebuffer. Because of that:
- **tests can see the output:** they compare rendered frames byte for byte against golden images;
- **a window is only a viewer:** a small Win32 window displays the framebuffer.

Build and test it only through Broadsword: `broadsword test render`.

## What it does

- `Framebuffer`: `0x00RRGGBB` pixels, `clear`, bounds-checked `set`, an exact integer Bresenham `line` (both
  endpoints included, clipped to the buffer), `point` (a filled square), `to_ppm` (binary P6) and `from_ppm`.
  A zero-sized framebuffer is an error from `Framebuffer::new`, not a panic.
- `Vec3`, `Mat4` (look-at, perspective), private to this crate.
- `Camera`: position, yaw, pitch, field of view, aspect, near, far, and fly speeds. `view_proj()`,
  `project(world, width, height)`, and `fly(&FlyInput, dt)`, a pure function with no clock or window.
  Yaw 0 looks down -Z; positive yaw turns toward +X.
- `draw_line_3d` clips each line against the near and far planes before projecting, so a line passing behind
  the camera draws correctly. A line whose screen ends are beyond about 1e9 pixels is then clipped in 2D
  (Liang-Barsky, along the line, so its slope is kept). `Framebuffer::line` skips straight to the on-screen part,
  so even `line(i32::MIN, 0, i32::MAX, 0, c)` is instant and draws the same pixels as the full walk. `draw_point_3d` draws a point if it is in the depth range.
- `wire_cube(center, size, rotation_y)`: the 12 edges of a cube.
- Screen coordinate `s` lights pixel `round(s)`, so the image centre of a 160 pixel wide image is x = 80.0.

## The viewer (Windows)

```
broadsword run render -- cargo run --offline --release --example spin
```

Controls: W/S forward/back, A/D left/right, E/Q up/down, arrow keys turn and look, Esc or the close box quits.
Keys only count while the window has focus.

For a headless timing run that opens no window:

```
broadsword run render -- cargo run --offline --release --example spin -- --frames 120
```

It renders 120 frames of a 640x480 scene, prints the average frame time and exits 0. `--width` and `--height`
change the size.

`tests/window_smoke.rs` opens the real window far off every monitor and without activation, presents 3 frames
and closes it. It never takes focus.

## Replaying a crash-sim trajectory

The renderer and crash-sim are separate projects that meet only through a file: crash-sim's
`examples/trajectory.rs` writes a CSV (`t,node,x,y,z`, 8 nodes per frame), and `src/replay.rs` reads it.

- `Trajectory::parse(text)` refuses, naming the line: an empty file, a wrong header, a non-number, a frame
  with other than 8 nodes, nodes out of order, time going backwards (or changing inside a frame).
- Node `i` of crash-sim is numbered by its bits (x = bit 0, y = bit 1, z = bit 2), so the 12 cube edges are
  the node pairs that differ in one bit (`cube_edges()`).
- `draw_frame(fb, camera, frame)` clears, then draws the ground grid at y = 0, the 12 edges and the 8 nodes.
  The viewer and the tests use this same function. `frame_index_at(t)` picks the frame for a time and holds
  the last frame after the end.

```
broadsword run render -- cargo run --offline --release --example replay -- tests/fixtures/trajectory.csv
```

Plays in real time from the file's own `t` column. Space pauses, R restarts, the fly keys move the camera,
Esc quits. `--frames N` renders N frames headless and prints the average frame time. `--present N` opens the
window off screen without activation, presents N frames through the real `pump`, `fly_input` and `present`
path, and exits 0. `tests/fixtures/trajectory.csv` is a real crash-sim run (51 frames, stride 10).

`tests/replay.rs` holds three golden images of it (first frame, first ground contact, settled). Each is also
checked against a second projection written in the test in f64, a plain Bresenham walk and pixel counts, so the
golden file is not the only evidence the picture is right.

## Golden images

`tests/golden.rs` renders each scene and compares it byte for byte with `tests/golden/<name>.ppm`.

- **Mismatch:** the test fails with the number of differing pixels and their bounding box, and writes
  `tests/golden/<name>.actual.ppm` (git-ignored). Open it in any image viewer or diff it against the golden file.
- **Missing golden file:** the test fails (it never passes silently), writes the `.actual.ppm`, and says how to accept it.
- **Accepting a change** is always by hand: check the `.actual.ppm` is right, then copy it over `<name>.ppm`.
  Nothing accepts automatically. Say in the commit why the image changed.

Scenes are chosen so the maths can be checked independently: `tests/behaviour.rs` holds hand-derived numbers
(for example where the cube's 8 corners must land) alongside the hard cases (lines outside or through the
camera, zero length, camera inside the cube, a 1x1 buffer).

The images assume `f32` sine and cosine give the same results on the machine; if a golden test fails
on a new platform with a pixel or two different, that is why.

## Shaded meshes, one light and shadows (milestone 2)

```
broadsword run render -- cargo run --offline --release --example car
broadsword run render -- cargo run --offline --release --example car -- --frames 120
broadsword run render -- cargo run --offline --release --example car -- --present 30
```

`examples/car.rs` turns a car on a turntable in a window (fly keys as above). `--frames N` renders headless and
prints the average frame time at 640x480 with the shadow pass, split into shadow pass and main pass, plus the
triangle counts. `--present N` presents N frames off screen without activation and also reports the blit.
Options: `--width`, `--height`, `--shadow-size` (default 1024), `--segments` (wheel sides, default 24).
Measured on the build machine while it was busy with other jobs: about 8 to 10 ms per frame (limit 16.6),
410 triangles, 199 surviving culling; shadow pass about 5.5 to 6.3 ms, main pass about 2.7 to 3.5 ms, blit about 0.09 ms.

### The pipeline (`src/raster.rs`, `src/mesh.rs`)

- `Mesh { positions, normals, triangles }`: counter-clockwise triangles seen from outside. `normals` is empty
  (flat shading with each triangle's own normal) or has one unit normal per position (smooth, Gouraud).
  `Mesh::from_obj` reads `v`, `vn`, `vt` (ignored), and `f` with 3 or 4 vertices in the forms `v`, `v//vn`,
  `v/vt/vn`; quads are split on their first diagonal. It refuses everything else with the line number (other
  directives, `v/vt`, negative or zero or not-yet-defined indices, polygons of five or more, non-finite numbers, zero-length normals
  (a non-unit `vn` is normalised, not refused; a length that overflows to infinity is refused too),
  faces with and without normals mixed). `car_mesh(segments)`, `car_body_mesh()`, `wheel_mesh(segments)` and
  `ground_plane_mesh(half)` are procedural; the car faces +X, is 4.0 long and rests on y = 0.
- `Renderer::render(fb, camera, scene)` clears the framebuffer and a depth buffer, then runs two passes:
  1. **Shadow pass.** Every shadow-casting object is drawn, both faces, into a square `f32` map
     (`ShadowSettings::map_size`) seen along the light direction with an orthographic projection fitted to a sphere
     (`focus_center`, `focus_radius`). Each texel keeps the smallest distance from the light. `caster_sphere`
     computes a sphere for you.
  2. **Main pass.** For each triangle: world positions and normals, then the diffuse term `max(N.L, 0)` per vertex,
     the clip position, and the shadow-map position and distance. The triangle is **clipped against the near plane in
     clip space** (z >= -w, before the divide; it can become a quad, which is split in two). Then it is projected,
     **back faces are culled** (`Sides::FrontOnly`), and it is filled with the fill rule below. Each pixel does an
     `f32` depth test first; the colour is `base * (ambient + diffuse * N.L * lit)`, where `lit` is 1 or 0 from the
     shadow test, per channel, rounded, no gamma. Attributes are interpolated perspective-correctly.
- It is deterministic: the same scene gives the same bytes, whatever the renderer drew before.
- A pixel (x, y) is sampled at the screen coordinate (x, y), the same convention as the line drawing, so lines drawn
  on top of a mesh line up with it.

### The fill rule

`fill_triangle` snaps each screen vertex to 1/16 pixel and evaluates the three edge functions in exact `i64`
integers, so there is no rounding to disagree about. A pixel exactly on an edge belongs to the triangle only if the
edge is a **top** edge (horizontal, the triangle below it) or a **left** edge (the triangle to its right). The two
triangles sharing an edge walk it in opposite directions, so exactly one owns each pixel on it: no gaps, no double
pixels. Zero-area triangles (including exactly edge-on ones) are not drawn.

Known limitation: a vertex more than 2^24 (16,777,216) pixels from the origin is **clamped** to that range, so the
integer products stay inside `i64`. This is not clipping. The clamp moves the vertex, which changes the triangle's
shape, so such a triangle can cover different pixels than the true one. Only the near plane is clipped; there is no
clipping against the sides of the screen. Triangles whose vertices all lie within 2^24 pixels, however far outside
the screen, are filled exactly (the tests use corners at 1e6 pixels).

### Shadow bias

A pixel is in shadow if `distance_from_light - bias > map value`, where
`bias = texel * (1.0 + 1.0 * min(tan(angle), 8))`, `texel = 2 * focus_radius / map_size`, and `angle` is between the
triangle's true (geometric) normal and the light. The three numbers are `BIAS_CONST_TEXELS`, `BIAS_SLOPE` and
`BIAS_MAX_SLOPE`, and are fields of `ShadowSettings`.

- The map stores the surface depth at texel centres, and a pixel looks up the nearest texel (no filtering). On a
  surface tilted `angle` from the light, the depth can differ across half a texel by up to about
  `0.71 * texel * tan(angle)`; the slope term (1.0 * texel * tan) covers that with margin, and the constant term
  (one texel) covers rounding. These round values pass the acne test below at all four sun heights; no search was made for the smallest ones.
- **Acne** (a surface shadowing itself in stripes): `tests/shadows.rs` renders a ground plane that casts and
  receives, at sun heights of 75, 45, 20 and 10 degrees, and requires every visible ground pixel to be lit (over
  10000 pixels each). With the bias switched off the same test finds hundreds of wrong pixels, so it can see acne.
- **Peter-panning** (a gap between an object and its shadow) is as large as the bias. For the 2 m cube test the bias on
  the floor is 0.5 cm (one texel is 0.34 cm), far less than a pixel, and the test checks that the shadow reaches the
  cube's foot and ends where the maths says (within 5 cm at its far end).
- `max_slope` stops a surface almost edge-on to the light from getting a huge bias; such a surface is nearly black
  from `N.L` anyway.

### What the tests check

`tests/raster.rs`, `tests/shadows.rs`, `tests/mesh.rs`, with an independent model in `tests/common/mod.rs` (f64
projection, the near-plane clip redone in view space, and a pixel-ownership rule defined by an infinitesimal shift of
the sample point rather than by the rasterizer's bias form). Golden images: `depth_order`, `near_clip`,
`shared_edge`, `sphere_shaded`, `car_shadow`. Each test checks its scene against the model first (exact pixel masks for depth order
and the near clip; sphere N.L within 3 levels (10 levels where |N.L| < 0.12, near the terminator) at every sphere pixel inside 0.93 of the outline radius, with the exact count printed by the test; every judged ground pixel of the car scene classed as
shadow or lit by projecting the car's boxes along the light, with a control that fails when the shadow is moved 0.7 m).
`two_triangles_sharing_an_edge_cover_every_pixel_exactly_once` prints exact counts: for the 4-point quad, both
diagonals give triangle counts that sum to the quad's count with no pixel doubled, including pixels exactly on the
shared edge. `golden_shared_edge_through_the_renderer_lights_every_pixel_exactly_once` does the same through
`Renderer::render` (culling, near clip, depth test) with two coloured triangles, in both draw orders, compares the
rendered picture pixel for pixel with the f64 ownership model, prints r1, r2, their sum, the union and the lit total,
and locks the picture in as `tests/golden/shared_edge.ppm`.

## Not done yet

- Shadows are hard, one map, nearest-sample (no PCF, no cascades): edges are stair-stepped.
- Shading is Gouraud with no specular, no gamma, no textures, no transparency, no clipping to the other frustum
  planes (pixels beyond the far plane are dropped one by one), no multi-threading.
- `model` matrices may rotate, translate and scale uniformly only; a non-uniform scale would skew the normals.
- OBJ: no negative indices, forward references, polygons of five or more, groups or materials.
- No text.
- Lines are one pixel wide and not anti-aliased; colours are not blended.
- The viewer is Win32 only and its window cannot be resized. `Window` is not `Send`: keep it on the thread that opened it.
- `present` to a minimised window does nothing (and is not an error).
- Projection is f32: a line whose screen ends are millions of pixels away can be a pixel or two off in the middle.
