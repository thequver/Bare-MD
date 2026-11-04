use std::time::Instant;
use bare_md::doc::Doc;
use bare_md::layout::Layout;
use bare_md::text::TextEngine;
use bare_md::theme::Theme;

fn big_doc(lines: usize) -> String {
    let mut s = String::new();
    for i in 0..lines {
        match i % 7 {
            0 => s.push_str(&format!("## Section {i}\n")),
            1 => s.push_str(&format!("A paragraph of text number {i} with **bold** and `code` bits that should be shaped once and cached.\n")),
            2 => s.push_str("- bullet item\n"),
            3 => s.push_str("> a quote line\n"),
            4 => s.push_str("\n"),
            5 => s.push_str("| a | b |\n"),
            _ => s.push_str(&format!("Plain line {i} with some ordinary words to wrap when narrow.\n")),
        }
    }
    s
}

#[test]
fn layout_performance_large_document() {
    let text = big_doc(4000);
    let doc = Doc::new(&text, None);
    let mut te = TextEngine::new();
    let theme = Theme::black();

    let t0 = Instant::now();
    let layout = Layout::build(&doc, &mut te, &theme, 900.0);
    let cold = t0.elapsed();
    println!("cold layout of {} lines: {:?}", layout.lines.len(), cold);

    // Warm rebuild (shape cache hot) == one keystroke.
    let mut times = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        let _ = Layout::build(&doc, &mut te, &theme, 900.0);
        times.push(t.elapsed());
    }
    println!("warm rebuilds: {:?}", times);
    let avg = times.iter().sum::<std::time::Duration>() / times.len() as u32;
    println!("average warm rebuild: {:?}", avg);
    // Budgets for an unoptimised `cargo test` build; the release binary is an
    // order of magnitude faster on the same machine.
    assert!(cold.as_secs_f64() < 5.0, "cold layout too slow: {cold:?}");
    assert!(avg.as_millis() < 220, "warm layout too slow: {avg:?}");
    println!("shape cache: {} hits / {} misses", te.cache_hits, te.cache_misses);
}

/// Start-up, as a test rather than a feeling.
///
/// "It starts more slowly since the About section" is checkable: the pieces that run
/// during a launch are `App::new` (settings, workspace, the file named on the command
/// line) and the first frame, plus the window icon's rasterisation. The About box is
/// *not* among them - it is built when it is opened - and this test pins that down
/// three ways: a budget for the launch, a budget for the icon, and the fact that the
/// About text costs a fraction of a millisecond and no dialog exists after start-up.
///
/// The budgets are generous (a debug build, in CI, on a slow machine) because their job
/// is catching a *blunder* - a 4 MB allocation per shape, a full license text shaped
/// per frame - not measuring a specific computer.
#[test]
fn start_up_stays_within_budget_and_about_costs_nothing() {
    use bare_md::app::App;
    use bare_md::config::{Paths, Session, Settings};
    use bare_md::gpu::Renderer;

    let dir = std::env::temp_dir().join(format!("bare-md-startup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let paths = Paths {
        data_dir: dir.clone(),
        config_file: dir.join("settings.conf"),
        origin: "test".into(),
    };
    let mut settings = Settings::default();
    settings.workspace = Some(dir.clone());

    // 1. The window icon, which is the only one of these that runs before the window.
    let t = Instant::now();
    let icon = bare_md::appicon::window_icon();
    let icon_time = t.elapsed();
    drop(icon);
    println!("window icon: {icon_time:?}");
    assert!(
        icon_time < std::time::Duration::from_millis(400),
        "rasterising the window icon took {icon_time:?}: something in the icon path is \
         quadratic again (it should be a few milliseconds in release)"
    );

    // 2. The editor itself, and the first frame.
    let t = Instant::now();
    let mut app = App::new(paths, settings, Session::default(), vec![]);
    let new_time = t.elapsed();
    let mut r = Renderer::offscreen(1180, 780, 1.0);
    bare_md::ui::draw(&mut app, &mut r);
    r.end();
    let frame_time = t.elapsed() - new_time;
    println!("App::new: {new_time:?}, first frame: {frame_time:?}");
    assert!(
        new_time < std::time::Duration::from_millis(600),
        "App::new took {new_time:?}"
    );
    assert!(
        frame_time < std::time::Duration::from_millis(3000),
        "the first frame took {frame_time:?}"
    );

    // 3. Start-up builds no About box, and the box's own text is not start-up work.
    assert!(app.dialog.is_none(), "a dialog opened itself during start-up");
    let t = Instant::now();
    let about = bare_md::ui::about::About::new();
    let about_time = t.elapsed();
    println!("About::new: {about_time:?} ({} lines)", about.text.len());
    assert!(
        about_time < std::time::Duration::from_millis(50),
        "building the About text took {about_time:?} - it is 20 KB of text, not 20 KB of work"
    );
    // The crate list is a compile-time constant: touching it is free.
    let t = Instant::now();
    let n = bare_md::ui::about::CRATES.len();
    let crates_time = t.elapsed();
    assert!(n > 50);
    assert!(
        crates_time < std::time::Duration::from_millis(5),
        "reading the crate list took {crates_time:?}: it must never be built at run time"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A dialog must not cost a fifth of a second per frame.
///
/// The About box holds a page of text, and on a software rasteriser (a VM, or a machine
/// with no GPU driver) "lots of text in a box" was measured at **+21.8 ms per frame**:
/// the scrim, the box fill and the box border were three overlapping full-area quads,
/// each fetching the atlas per pixel, and the border was drawn twice over the whole box
/// (`round` then `stroke`). A flat fill now skips the atlas fetch, and every bordered
/// panel is one quad - so the same frame costs about 5.6 ms here. The budget below sits
/// between the two: it passes today and fails if either fix is undone.
#[test]
fn a_dialog_frame_costs_a_few_milliseconds() {
    use bare_md::app::App;
    use bare_md::config::{Paths, Session, Settings};
    use bare_md::gpu::Renderer;

    let dir = std::env::temp_dir().join(format!("bare-md-dialog-cost-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let paths = Paths {
        data_dir: dir.clone(),
        config_file: dir.join("settings.conf"),
        origin: "test".into(),
    };
    let mut settings = Settings::default();
    settings.workspace = Some(dir.clone());
    let mut app = App::new(paths, settings, Session::default(), vec![]);
    let mut text = String::new();
    for i in 0..60 {
        text.push_str(&format!("## Section {i}\n\nA paragraph with **bold** and `code`.\n\n- item\n\n"));
    }
    app.insert_text(&text);
    app.mouse = (-50.0, -50.0);
    let mut r = Renderer::offscreen(1180, 780, 1.0);

    let frame = |app: &mut App, r: &mut Renderer| {
        let t = Instant::now();
        bare_md::ui::draw(app, r);
        r.end();
        t.elapsed().as_secs_f64() * 1e3
    };
    let median = |mut v: Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };

    for _ in 0..3 {
        frame(&mut app, &mut r);
    }
    let base = median((0..15).map(|_| frame(&mut app, &mut r)).collect());

    app.open_about();
    frame(&mut app, &mut r);
    let still = median((0..15).map(|_| frame(&mut app, &mut r)).collect());
    let mut scrolled = Vec::new();
    for _ in 0..15 {
        if let Some(bare_md::app::Dialog::About(a)) = &mut app.dialog {
            a.scroll_by(1);
        }
        scrolled.push(frame(&mut app, &mut r));
    }
    let scrolling = median(scrolled);

    println!("frame: document {base:.2} ms, About {still:.2} ms, scrolling {scrolling:.2} ms");
    println!("  -> the About box costs {:.2} ms per frame", still - base);
    assert!(
        still - base < 12.0,
        "the About box costs {:.2} ms per frame (document {base:.2}, About {still:.2}): \
         a full-area quad is fetching the atlas again, or a panel is drawn as fill + \
         stroke instead of one bordered quad",
        still - base
    );
    assert!(
        scrolling - base < 14.0,
        "scrolling the About box costs {:.2} ms per frame",
        scrolling - base
    );
    drop(app);
    drop(r);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Opening the About box must not hitch: preparing its first page is done ahead.
///
/// Building the notice is cheap (about 1 ms), but the *first* frame that draws it
/// paid for everything at once - shaping 21 lines of licence text (about 30 ms in a
/// debug build, rustybuzz, once per distinct string) and rasterising 92 glyphs into the
/// atlas one `write_texture` at a time. Measured here: 41 ms for that frame against 9 ms
/// for the next one, which is exactly what "the delay is loading the text in About"
/// looks like. `about::warm` (called once, after the first frame is on screen) shapes
/// and rasterises the page up front, and this test pins the difference down.
#[test]
fn warming_the_about_page_makes_opening_it_cheap() {
    use bare_md::app::{App, Dialog};
    use bare_md::config::{Paths, Session, Settings};
    use bare_md::gpu::Renderer;

    fn setup(tag: &str) -> (App, Renderer, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("bare-md-warm-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = Paths {
            data_dir: dir.clone(),
            config_file: dir.join("settings.conf"),
            origin: "test".into(),
        };
        let mut settings = Settings::default();
        settings.workspace = Some(dir.clone());
        let mut app = App::new(paths, settings, Session::default(), vec![]);
        app.insert_text("# A document\n\nWith a paragraph of ordinary text in it.\n");
        app.mouse = (-50.0, -50.0);
        let mut r = Renderer::offscreen(1180, 780, 1.0);
        bare_md::ui::draw(&mut app, &mut r);
        r.end();
        (app, r, dir)
    }

    let first_frame_after_opening = |app: &mut App, r: &mut Renderer| {
        let t = Instant::now();
        bare_md::ui::draw(app, r);
        r.end();
        let first = t.elapsed().as_secs_f64() * 1e3;
        let mut later = Vec::new();
        for _ in 0..6 {
            if let Some(Dialog::About(a)) = &mut app.dialog {
                a.scroll_by(2);
            }
            let t = Instant::now();
            bare_md::ui::draw(app, r);
            r.end();
            later.push(t.elapsed().as_secs_f64() * 1e3);
        }
        later.sort_by(|a, b| a.partial_cmp(b).unwrap());
        (first, later[later.len() / 2])
    };

    // Cold: press F1 with nothing prepared.
    let (mut app, mut r, dir) = setup("cold");
    app.open_about();
    let (cold_first, _) = first_frame_after_opening(&mut app, &mut r);
    drop(app);
    drop(r);
    let _ = std::fs::remove_dir_all(&dir);

    // Warm: what the app does after its first frame.
    let (mut app, mut r, dir) = setup("warm");
    let t = Instant::now();
    bare_md::ui::about::warm(&mut r);
    let warm_time = t.elapsed().as_secs_f64() * 1e3;
    app.open_about();
    let (warm_first, steady) = first_frame_after_opening(&mut app, &mut r);
    println!(
        "first About frame: cold {cold_first:.2} ms, warmed {warm_first:.2} ms \
         (steady {steady:.2} ms); the warm-up itself {warm_time:.2} ms"
    );

    assert!(
        warm_time < 400.0,
        "the warm-up itself cost {warm_time:.2} ms - it must stay a background-sized task"
    );
    assert!(
        warm_first < cold_first,
        "warming did not help: cold {cold_first:.2} ms, warmed {warm_first:.2} ms"
    );
    assert!(
        warm_first - steady < 10.0,
        "even warmed, the first About frame costs {:.2} ms more than the ones after it \
         ({warm_first:.2} vs {steady:.2}) - the page is being shaped or uploaded on it again",
        warm_first - steady
    );
    drop(app);
    drop(r);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A launch must put the first frame on screen without parsing fonts it will not use.
///
/// The eight bundled faces used to be parsed together at start-up - about 110 ms in a
/// release build, half of the entire launch - even though a Latin screen only draws
/// with Lato and JetBrains Mono. They are parsed one at a time now (see `text::SLOTS`),
/// so a cold process reaches its first frame after the faces it needs and no more.
/// Only a *cold* process can show this, so the test re-runs itself as a child: font
/// slots are process-wide and every other test in this binary has already parsed them.
#[test]
fn a_launch_does_not_parse_fonts_it_cannot_use() {
    const CHILD: &str = "BARE_MD_LAUNCH_CHILD";

    if std::env::var_os(CHILD).is_some() {
        use bare_md::app::App;
        use bare_md::config::{Paths, Session, Settings};
        use bare_md::gpu::Renderer;

        let t_all = Instant::now();
        let mut r = Renderer::offscreen(1180, 780, 1.0);
        let renderer_ms = t_all.elapsed().as_secs_f64() * 1e3;
        let dir = std::env::temp_dir().join(format!("bare-md-launch-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let paths = Paths {
            data_dir: dir.clone(),
            config_file: dir.join("settings.conf"),
            origin: "test".into(),
        };
        let mut settings = Settings::default();
        settings.workspace = Some(dir.clone());
        let mut app = App::new(paths, settings, Session::default(), vec![]);
        app.insert_text("# A heading\n\nAn ordinary Latin paragraph, with a dash - and *emphasis*.\n");
        app.mouse = (-50.0, -50.0);
        let t = Instant::now();
        bare_md::ui::draw(&mut app, &mut r);
        r.end();
        let frame_ms = t.elapsed().as_secs_f64() * 1e3;
        println!(
            "LAUNCH renderer {renderer_ms:.1} ms, first frame {frame_ms:.1} ms, \
             fallbacks parsed: {}",
            bare_md::text::fallbacks_parsed()
        );
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }

    let exe = std::env::current_exe().expect("test binary");
    let out = std::process::Command::new(exe)
        .args(["--exact", "a_launch_does_not_parse_fonts_it_cannot_use", "--nocapture"])
        .env(CHILD, "1")
        .output()
        .expect("re-run the test binary");
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text
        .lines()
        .find(|l| l.starts_with("LAUNCH "))
        .unwrap_or_else(|| panic!("no LAUNCH line in the child output:\n{text}"));
    println!("{line}");
    assert!(
        line.ends_with("fallbacks parsed: false"),
        "a launch parsed the DejaVu fallback faces: {line} - they are only needed for \
         characters Lato does not have, and a launch should not pay for them"
    );
}

/// A long document must not make every *frame* slower - only *loading* it.
///
/// A 30 000-line file (753 KB) measured on this machine: opening costs 2 ms, the
/// first frame 320 ms (that is the layout of the whole document), and every frame
/// after it 6.6 ms whether it scrolls near the top or at line 20 000 - painting is
/// windowed, so the document's size does not reach the frame. This test pins that
/// down with a 5 000-line document against a 50-line one; the absolute numbers are
/// debug-build numbers, so the assertion is about the *ratio*, which is the part that
/// would break if painting or the text path ever started walking the document.
#[test]
fn a_long_document_costs_the_same_per_frame_as_a_short_one() {
    use bare_md::app::App;
    use bare_md::config::{Paths, Session, Settings};
    use bare_md::gpu::Renderer;

    fn doc_of(blocks: usize) -> String {
        let mut s = String::new();
        for i in 0..blocks {
            s.push_str(&format!(
                "# Section {i}\n\nA paragraph of ordinary prose with **bold**, *italic*, `code` and a \
                 [link](https://example.com/{i}) that is long enough to wrap in the window.\n\n\
                 - first item\n- second item\n\n```rust\nfn sample_{i}(x: u32) -> u32 {{ x + {i} }}\n```\n\n"
            ));
        }
        s
    }

    /// Median frame time while scrolling, for a document of `blocks` sections.
    fn median_scroll_frame(tag: &str, blocks: usize) -> (f64, usize) {
        let dir = std::env::temp_dir().join(format!("bare-md-long-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = Paths {
            data_dir: dir.clone(),
            config_file: dir.join("settings.conf"),
            origin: "test".into(),
        };
        let mut settings = Settings::default();
        settings.workspace = Some(dir.clone());
        let mut app = App::new(paths, settings, Session::default(), vec![]);
        app.insert_text(&doc_of(blocks));
        app.mouse = (-50.0, -50.0);
        let mut r = Renderer::offscreen(1180, 780, 1.0);
        bare_md::ui::draw(&mut app, &mut r);
        r.end();
        let lines = app.tab().layout.lines.len();
        // Somewhere in the middle: the far end of the document.
        let start = app.tab().scroll;
        let mut times = Vec::new();
        for i in 0..8 {
            app.tab_mut().scroll = start + i as f32 * 600.0;
            let t = Instant::now();
            bare_md::ui::draw(&mut app, &mut r);
            r.end();
            times.push(t.elapsed().as_secs_f64() * 1e3);
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        drop(app);
        drop(r);
        let _ = std::fs::remove_dir_all(&dir);
        (times[times.len() / 2], lines)
    }

    let (short, short_lines) = median_scroll_frame("short", 6);
    let (long, long_lines) = median_scroll_frame("long", 500);
    println!("scroll frame: {short:.2} ms over {short_lines} lines, {long:.2} ms over {long_lines} lines");
    assert!(
        long < short * 2.0 + 8.0,
        "a frame in a {long_lines}-line document costs {long:.2} ms against {short:.2} ms in a \
         {short_lines}-line one: something on the frame path is walking the document"
    );
}
