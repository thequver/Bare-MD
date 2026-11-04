//! The window and the event loop: everything that talks to the operating system.
//!
//! This is the only module that knows winit exists (together with
//! `platform::input`, which translates a `WindowEvent` into an editor command).
//! The editor itself is an ordinary object with an `App::handle_event` entry
//! point, which is what lets the whole UI be driven from a test - and rendered
//! offscreen - without a window at all.

use std::path::PathBuf;
use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::app::{App, ConfirmAction, Dialog};
use crate::platform::single::{self, Claim, Claimed};
use crate::config::{Paths, Session, Settings};
use crate::gpu::Renderer;
use crate::input::InputState;
use crate::theme::ThemeKind;
use crate::ui;

/// What the event loop is woken up for from outside the window.
#[derive(Debug)]
pub enum UserEvent {
    /// Another launch handed this instance its file names.
    Handoff,
}

struct Root {
    /// Set once the About box's first page has been shaped (see `about::warm_shapes`).
    warmed_about: bool,
    last_persist: std::time::Instant,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    app: Option<App>,
    input: InputState,
    files: Vec<PathBuf>,
    screenshot: Option<PathBuf>,
    settings: Option<Settings>,
    paths: Option<Paths>,
    /// The running instance's end of the single-instance handoff.
    instance: Option<Arc<Claimed>>,
    /// Does the window hold the pointer for a drag right now?
    grabbed: bool,
}

impl ApplicationHandler<UserEvent> for Root {
    fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Handoff => self.take_handoff(el),
        }
    }

    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // The eight fonts take ~130 ms to parse, and the graphics device below takes
        // longer than that. Starting the parse here means it happens *while* the
        // window is being created and the device made, instead of after - the user is
        // waiting through that time anyway.
        crate::text::preload_faces();
        let paths = self.paths.clone().unwrap_or_else(Paths::resolve);
        let settings = self.settings.clone().unwrap_or_else(|| Settings::load(&paths));
        // The screens are needed twice - to fit the remembered size and to check
        // that the remembered position is still somewhere visible - so they are
        // read once here.
        let screens = screen_rects(el);
        let size = crate::config::fit_window(settings.window.unwrap_or((1180.0, 780.0)), &screens);
        // The icon is built from `assets/app/icon-bg.svg` and `icon-fg.svg`, so it
        // is an asset like the toolbar icons rather than a blob in the binary.
        // No window icon in the attributes: it is set right after the window exists
        // (a few lines down). Rasterising it used to happen *before* the window was
        // created, so the user waited for our drawing code before anything appeared on
        // screen - small in release, very visible in a debug build. The executable's own
        // icon (Explorer, shortcut, taskbar pin) comes from the Windows resource either
        // way, so nothing looks wrong in the meantime.
        let mut attrs = Window::default_attributes()
            .with_title("Bare MD")
            .with_inner_size(LogicalSize::new(size.0 as f64, size.1 as f64))
            .with_min_inner_size(LogicalSize::new(
                crate::config::MIN_WINDOW.0 as f64,
                crate::config::MIN_WINDOW.1 as f64,
            ));
        // Put the window back where the user left it - size, position and whether
        // it was maximized. The position is only replayed if a screen that exists
        // *now* still shows a grabbable corner of it: a monitor that has since been
        // unplugged must not swallow the window.
        if let Some(pos) = settings.window_pos {
            if crate::config::rect_on_screens(pos, size, &screens) {
                attrs = attrs.with_position(LogicalPosition::new(pos.0 as f64, pos.1 as f64));
            } else {
                log::info!(
                    "window position {},{} is off every screen now; using the default",
                    pos.0,
                    pos.1
                );
            }
        }
        if settings.window_maximized {
            attrs = attrs.with_maximized(true);
        }
        // What the window calls itself to the desktop. A shell matches a running
        // window to its `.desktop` file by this name (Wayland `app_id`, X11
        // `WM_CLASS`): without it the taskbar shows a generic icon and a pinned entry
        // never lights up. `assets/app/bare-md.desktop` carries the same name as its
        // `StartupWMClass`.
        // Both traits provide a `with_name`, so the one that applies is chosen by
        // the session the window will actually be created in.
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            if matches!(
                std::env::var("XDG_SESSION_TYPE").as_deref(),
                Ok("x11") | Ok("X11") | Ok("tty")
            ) || std::env::var("DISPLAY").is_ok() && std::env::var("WAYLAND_DISPLAY").is_err()
            {
                attrs = winit::platform::x11::WindowAttributesExtX11::with_name(
                    attrs,
                    crate::APP_ID,
                    crate::APP_ID,
                );
            } else {
                attrs = winit::platform::wayland::WindowAttributesExtWayland::with_name(
                    attrs,
                    crate::APP_ID,
                    crate::APP_ID,
                );
            }
        }
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("cannot create window: {e}");
                eprintln!("cannot open a window: {e}");
                self.release_instance();
                el.exit();
                return;
            }
        };
        // The window exists from here, so the desktop has something to put up while the
        // graphics device is being made. The icon is rasterised in that gap.
        window.set_window_icon(Some(crate::appicon::window_icon()));
        let Some(renderer) =
            Renderer::new(window.clone(), self.screenshot.clone(), self.screenshot.is_some())
        else {
            // No Vulkan and no OpenGL: say what to install instead of crashing.
            //
            // A virtual machine is the common case for this message, and its driver
            // probing prints things like "failed to create dri2 screen", "DRI2: Failed
            // to load a driver" and "failed to detect any valid GPUs" before it gives
            // up - which reads like a broken build and is not one. So the message names
            // the *software* driver explicitly.
            eprintln!(
                "Bare MD needs a graphics driver that speaks Vulkan or OpenGL.\n\
                 \n\
                 On Debian/Ubuntu:  sudo apt install mesa-vulkan-drivers libgl1-mesa-dri\n\
                 On Fedora:         sudo dnf install mesa-vulkan-drivers mesa-dri-drivers\n\
                 On Arch/EndeavourOS: sudo pacman -S vulkan-icd-loader mesa vulkan-swrast\n\
                 \n\
                 A software driver is enough: it is only used when no GPU driver exists.\n\
                 In a virtual machine - where the driver says \"failed to create dri2 screen\"\n\
                 or \"failed to detect any valid GPUs\" - installing the software driver\n\
                 above (vulkan-swrast on Arch, mesa-vulkan-drivers elsewhere) is the fix.\n\
                 \n\
                 Check yours with: vulkaninfo --summary"
            );
            log::error!("no graphics device; exiting");
            self.release_instance();
            el.exit();
            return;
        };
        let session = Session::load(&paths);
        let app = App::new(paths.clone(), settings, session, self.files.clone());
        log::info!("settings: {} ({})", paths.config_file.display(), paths.origin);

        // Enable IME so Cyrillic/CJK input and composition work.
        window.set_ime_allowed(true);
        window.request_redraw();
        el.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_secs(10),
        ));

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.app = Some(app);
        self.paths = Some(paths);
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        // A launch may have handed files over while the loop was waiting; the
        // proxy event normally wakes us, but draining here costs nothing and means
        // a coalesced event cannot leave a file unopened.
        self.take_handoff(el);
        // A middle-button fling keeps the loop awake at ~120 Hz until it decays.
        if let Some(app) = &self.app {
            if app.kinetic_active() {
                let now = std::time::Instant::now();
                let wake = now + std::time::Duration::from_millis(8);
                let persist_due = self.last_persist + std::time::Duration::from_secs(10);
                el.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(wake.min(persist_due)));
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                return;
            }
        }
        // Wake up a few times a minute so settings and the session survive a
        // crash, a power cut or `kill -9`; the event loop stays idle otherwise.
        let due = self.last_persist + std::time::Duration::from_secs(10);
        let now = std::time::Instant::now();
        if now >= due {
            if let (Some(app), Some(window)) = (&self.app, &self.window) {
                self.persist(app, window);
            }
            self.last_persist = now;
            el.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                now + std::time::Duration::from_secs(10),
            ));
        } else {
            el.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(due));
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let (Some(window), Some(r), Some(app)) =
            (self.window.clone(), self.renderer.as_mut(), self.app.as_mut())
        else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => {
                let at_risk = app.tabs_at_risk();
                if at_risk.is_empty() {
                    self.shutdown(el);
                } else {
                    app.dialog = Some(Dialog::Confirm {
                        title: "Unsaved changes".to_string(),
                        message: format!(
                            "{} document(s) have unsaved changes or a deleted file.",
                            at_risk.len()
                        ),
                        danger: false,
                        on_yes: ConfirmAction::Quit,
                    });
                    window.request_redraw();
                }
            }
            WindowEvent::Resized(size) => {
                r.resize(size.width, size.height, window.scale_factor() as f32);
                window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let size = window.inner_size();
                r.resize(size.width, size.height, window.scale_factor() as f32);
                window.request_redraw();
            }
            WindowEvent::DroppedFile(path) => {
                // Drag and drop from the file manager: folders become the
                // workspace, files open in a tab.
                app.open_dropped(&path);
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                app.step_kinetic();
                ui::draw(app, r);
                // The window is up and the frame is on screen: this is the one moment
                // when a few milliseconds of shaping cost nothing, so the About box's
                // first page is prepared here instead of on the frame that opens it.
                if !self.warmed_about {
                    self.warmed_about = true;
                    crate::ui::about::warm(r);
                }
                let title = format!("{} — Bare MD", app.tab().title());
                window.set_title(&title);
                if r.end() {
                    el.exit();
                }
                if app.should_close {
                    self.shutdown(el);
                }
            }
            WindowEvent::Focused(false) => {
                // Losing the window ends the drag: a pointer kept confined to a window
                // nobody is looking at is a trap.
                app.handle_event(&event, r, &mut self.input);
                hold_pointer(&window, &mut self.grabbed, false);
                window.request_redraw();
            }
            other => {
                if app.handle_event(&other, r, &mut self.input) {
                    window.request_redraw();
                }
                hold_pointer(&window, &mut self.grabbed, app.dragging_something());
                if app.should_close {
                    self.shutdown(el);
                }
            }
        }
    }
}

/// Keep the pointer inside the window for as long as a drag is running.
///
/// winit does not capture the mouse: the moment the pointer crosses the window's edge,
/// `CursorMoved` stops arriving - and the minimap is flush against that edge, so a drag
/// with a little sideways drift died halfway down the strip with the rest of the document
/// still below it, which reads as "the handle won't go any further". Holding the pointer
/// fixes it for every drag, not just that one.
///
/// Best effort by design: Wayland refuses `Confined` and does not need it - a button press
/// there is an implicit grab, so the events keep coming anyway.
fn hold_pointer(window: &Window, grabbed: &mut bool, hold: bool) {
    if *grabbed == hold {
        return;
    }
    *grabbed = hold;
    let mode = if hold { CursorGrabMode::Confined } else { CursorGrabMode::None };
    let _ = window.set_cursor_grab(mode);
}

impl Root {
    /// Open whatever a later launch handed over, and come to the front.
    ///
    /// Permission to open a second copy of the program is not permission to
    /// interrupt: the window is raised and the files are opened as ordinary tabs
    /// next to whatever was already there.
    fn take_handoff(&mut self, el: &ActiveEventLoop) {
        let Some(instance) = &self.instance else { return };
        let batches = instance.take();
        if batches.is_empty() {
            return;
        }
        let (Some(app), Some(window)) = (self.app.as_mut(), self.window.as_ref()) else {
            return;
        };
        for batch in batches {
            for path in batch {
                log::info!("opening {} from another launch", path.display());
                app.open_dropped(&path);
            }
        }
        window.set_minimized(false);
        window.focus_window();
        window.request_redraw();
        let _ = el;
    }

    /// Hand the single-instance lock back if we cannot run: a stale lock would
    /// make the next launch wait for a port that nobody is listening on.
    fn release_instance(&self) {
        if let Some(instance) = &self.instance {
            instance.release();
        }
    }

    fn persist(&self, app: &App, window: &Window) {
        let scale = window.scale_factor() as f32;
        let mut settings = app.settings.clone();
        let maximized = window.is_maximized();
        settings.window_maximized = maximized;
        // While maximized `inner_size` is the size of the screen, so it must not
        // overwrite the size to come back to; the same goes for the position.
        if !maximized {
            let size = window.inner_size();
            if size.width > 0 && size.height > 0 {
                settings.window = Some((size.width as f32 / scale, size.height as f32 / scale));
            }
            // Wayland refuses to say where a window is (and to be moved); that is
            // not an error, the size above is still worth keeping.
            if let Ok(pos) = window.outer_position() {
                settings.window_pos = Some((
                    (pos.x as f32 / scale).round() as i32,
                    (pos.y as f32 / scale).round() as i32,
                ));
            }
        }
        settings.save(&app.paths);
        app.save_session();
    }

    fn shutdown(&mut self, el: &ActiveEventLoop) {
        if let (Some(app), Some(window)) = (&self.app, &self.window) {
            self.persist(app, window);
        }
        // Hand the folder back before the window closes, so the next launch starts
        // as the instance instead of waiting for a connect timeout.
        if let Some(instance) = &self.instance {
            instance.release();
        }
        el.exit();
    }
}

/// Every attached screen as `(x, y, w, h)` in *logical* pixels.
///
/// Logical, because that is the space the saved window position lives in; each
/// monitor is converted with its own scale factor, so a 150 % display next to a
/// 100 % one lines up the way the user sees it.
fn screen_rects(el: &ActiveEventLoop) -> Vec<(f32, f32, f32, f32)> {
    el.available_monitors()
        .map(|m| {
            let s = m.scale_factor() as f32;
            let p = m.position();
            let size = m.size();
            (
                p.x as f32 / s,
                p.y as f32 / s,
                size.width as f32 / s,
                size.height as f32 / s,
            )
        })
        .collect()
}

/// Write one frame to `out` without a window or a display server.
///
/// This is the "does it work on this machine at all" path: a Linux server, a CI
/// runner, an SSH session, or a box whose only graphics driver is a software
/// rasteriser. It exercises the whole pipeline - settings, the Markdown layout,
/// the text engine, the wgpu renderer - and needs nothing but a Vulkan or OpenGL
/// device, so `bare-md --render shot.png notes.md` is a complete smoke test.
pub fn render_headless(out: &std::path::Path, files: &[PathBuf], theme: Option<ThemeKind>) -> i32 {
    let paths = Paths::resolve();
    let mut settings = Settings::load(&paths);
    if let Some(kind) = theme {
        settings.theme = kind;
    }
    let session = Session::load(&paths);
    let mut app = App::new(paths, settings, session, files.to_vec());
    app.mouse = (-10.0, -10.0);
    let mut r = match Renderer::try_offscreen(1180, 780, 1.0) {
        Some(r) => r,
        None => {
            // `offscreen` reports the same way the window path does.
            eprintln!("no graphics device (Vulkan or OpenGL) is available");
            return 1;
        }
    };
    ui::draw(&mut app, &mut r);
    r.end();
    r.screenshot = Some(out.to_path_buf());
    r.screenshot_frames = 1;
    ui::draw(&mut app, &mut r);
    r.end();
    if out.exists() {
        println!("wrote {}", out.display());
        0
    } else {
        eprintln!("could not write {}", out.display());
        1
    }
}

/// Command line: files and folders to open, `--theme`, `--screenshot`, `--render`.
fn parse_args() -> (Vec<PathBuf>, Option<PathBuf>, Option<ThemeKind>, Option<PathBuf>) {
    let mut files = Vec::new();
    let mut screenshot = None;
    let mut theme: Option<ThemeKind> = None;
    let mut render = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--screenshot" => screenshot = args.next().map(PathBuf::from),
            "--render" => render = args.next().map(PathBuf::from),
            "--theme" | "-t" => {
                let value = args.next().unwrap_or_default();
                match ThemeKind::parse(&value) {
                    Some(kind) => theme = Some(kind),
                    None => {
                        eprintln!("unknown theme '{value}' (black, grey, white)");
                        std::process::exit(2);
                    }
                }
            }
            "--help" | "-h" => {
                println!("bare-md [FILE|DIR ...] [--theme black|grey|white] [--screenshot OUT.png]");
                println!("         bare-md --render OUT.png [FILE ...]   (no window: Linux/CI smoke test)");
                println!();
                println!("  Settings: one settings.conf next to the executable (or");
                println!("  $BAREMD_DATA_DIR/settings.conf). Nothing is written to");
                println!("  AppData, the registry or ~/.config.");
                std::process::exit(0);
            }
            other => files.push(PathBuf::from(other)),
        }
    }
    (files, screenshot, theme, render)
}

/// Start the editor: open the window and run until it closes.
pub fn run() {
    // Keep the console quiet by default. An editor that chatters about where its
    // settings file is and which port it took reads like a program in trouble, so
    // nothing below `warn` is printed - ours or the driver's. Modern AMD/Intel
    // drivers make the Vulkan loader and `wgpu_hal::vulkan::instance` emit harmless
    // WARN lines ("Unable to find extension", cooperative-matrix notices, missing
    // layer manifests) that read like errors to a user but mean nothing, and telling
    // them to do that has to be done down to `error`. `RUST_LOG` overrides all of it,
    // so the startup notes are one environment variable away when they are wanted.
    if std::env::var_os("VK_LOADER_DEBUG").is_none() {
        // Errors only from the loader itself; drop its INFO/DEBUG chatter.
        unsafe { std::env::set_var("VK_LOADER_DEBUG", "error") }
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(
        "warn,wgpu_hal=error,wgpu_core=warn,wgpu=warn,naga=warn,ash=warn",
    ))
    .init();

    let (files, screenshot, theme, render) = parse_args();
    if let Some(out) = render {
        // Headless: no event loop, no window, no display server.
        std::process::exit(render_headless(&out, &files, theme));
    }
    // ---- one instance per install ----
    // Look for a running copy *before* creating a window or reading settings: if
    // there is one, this launch only hands it the file names and exits, so the
    // second copy can never fight the first over the remembered window geometry.
    let resolved = Paths::resolve();
    let forwarding = screenshot.is_none();
    let mut instance = None;
    if forwarding {
        match single::claim(&resolved.data_dir, &files) {
            Claim::Sent => {
                // The running instance already has the files and the window.
                std::process::exit(0);
            }
            Claim::Primary(claimed) => instance = Some(Arc::new(claimed)),
            Claim::Solo => {}
        }
    }

    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(el) => el,
        Err(e) => {
            eprintln!("cannot start event loop: {e}");
            if let Some(claimed) = &instance {
                claimed.release();
            }
            std::process::exit(1);
        }
    };
    // Wake the loop when a launch hands files over, wherever it is in its wait: a
    // watcher thread blocks on the handoff queue and pokes the event loop.
    if let Some(claimed) = &instance {
        let proxy = event_loop.create_proxy();
        let watched = Arc::clone(claimed);
        let spawned = std::thread::Builder::new()
            .name("bare-md-wake".into())
            .spawn(move || loop {
                if !watched.wait_any() {
                    return; // the editor is shutting down
                }
                if proxy.send_event(UserEvent::Handoff).is_err() {
                    return;
                }
            });
        if spawned.is_err() {
            log::warn!("cannot start the handoff waker; a second launch may need a click");
        }
    }
    // A --theme flag only applies to this run and is not persisted.
    let mut settings = None;
    if let Some(kind) = theme {
        let mut s = Settings::load(&resolved);
        s.theme = kind;
        settings = Some(s);
    }
    let mut root = Root {
        warmed_about: false,
        last_persist: std::time::Instant::now(),
        window: None,
        renderer: None,
        app: None,
        input: InputState::default(),
        files,
        screenshot,
        settings,
        paths: Some(resolved),
        instance,
        grabbed: false,
    };
    if let Err(e) = event_loop.run_app(&mut root) {
        eprintln!("event loop error: {e}");
        std::process::exit(1);
    }
}
