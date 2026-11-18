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
  the camera draws correctly. `draw_point_3d` draws a point if it is in the depth range.
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

## Not done yet

- No filled triangles, no shading, no lights, no depth buffer, no textures, no text.
- Lines are one pixel wide and not anti-aliased; colours are not blended.
- The viewer is Win32 only and its window cannot be resized. `Window` is not `Send`: keep it on the thread that opened it.
- `present` to a minimised window does nothing (and is not an error).
- Projection is f32: a line whose screen ends are millions of pixels away can be a pixel or two off in the middle.
