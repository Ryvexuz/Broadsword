# render

Broadsword's renderer, Stage 4 of the development plan. It uses only the Rust standard library, with no crates.

All drawing happens in software, into an in-memory framebuffer. Because of that:
- **tests can see the output:** they compare rendered frames byte for byte against golden images;
- **a window is only a viewer:** a small Win32 window displays the framebuffer.

Build and test it only through Broadsword: `broadsword test render`.
