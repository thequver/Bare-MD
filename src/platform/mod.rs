//! The operating-system edge: a window, an event loop, a GPU and the events in
//! between.
//!
//! ```text
//! window.rs   winit: create the window, run the loop, persist on the way out
//! input.rs    WindowEvent -> App command (keys, pointer, wheel, IME)
//! single.rs   one instance per install: a second launch forwards its files
//! gpu/        wgpu: one instanced-quad pipeline, the text atlas, the targets
//! ```
//!
//! Nothing above this layer mentions winit or wgpu: `App::handle_event` takes a
//! plain `&WindowEvent` and `ui::draw` takes a `&mut Renderer`, which is what lets
//! the whole editor run headless in the test suite.

pub mod gpu;
pub mod input;
pub mod single;
pub mod window;

pub use window::run;
