#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]
//! Opens the Termy X panel for screenshots. It does not publish.
//!
//! ```sh
//! cargo run -p termy --bin x_panel_preview -- --mock --width 1600 --height 1000
//! cargo run -p termy --bin x_panel_preview -- --splash
//! ```

use gpui_kit::{AppContext, Bounds, WindowBounds, WindowKind, WindowOptions, px, size};

struct PreviewArgs {
    bench_tabs: bool,
    splash: bool,
    mock: bool,
    compose: bool,
    width: f32,
    height: f32,
}

fn preview_args() -> PreviewArgs {
    let mut args = PreviewArgs {
        splash: false,
        mock: false,
        compose: false,
        bench_tabs: false,
        width: std::env::var("TERMY_X_PREVIEW_WIDTH")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(1600.0),
        height: std::env::var("TERMY_X_PREVIEW_HEIGHT")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(1000.0),
    };
    let mut raw = std::env::args().skip(1);
    while let Some(arg) = raw.next() {
        match arg.as_str() {
            "--splash" | "splash" => args.splash = true,
            "--mock" | "mock" => args.mock = true,
            "--compose" | "compose" => args.compose = true,
            "--bench-tabs" | "bench-tabs" => args.bench_tabs = true,
            "--width" => {
                if let Some(value) = raw.next().and_then(|text| text.parse().ok()) {
                    args.width = value;
                }
            }
            "--height" => {
                if let Some(value) = raw.next().and_then(|text| text.parse().ok()) {
                    args.height = value;
                }
            }
            _ => {
                if let Some(value) = arg.strip_prefix("--width=") {
                    if let Ok(parsed) = value.parse() {
                        args.width = parsed;
                    }
                } else if let Some(value) = arg.strip_prefix("--height=") {
                    if let Ok(parsed) = value.parse() {
                        args.height = parsed;
                    }
                }
            }
        }
    }
    args.width = args.width.clamp(320.0, 3840.0);
    args.height = args.height.clamp(240.0, 2160.0);
    args
}

fn main() {
    let args = preview_args();
    // `--splash` is the empty full-strength bird. `--mock` and the default stay on fixtures.
    let scene = match (args.splash, args.compose, args.mock) {
        (true, _, _) => "splash",
        (_, true, _) => "compose",
        _ => "mock",
    };
    let width = args.width;
    let height = args.height;
    let bench_tabs = args.bench_tabs;

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        // Rasterize before the first frame so the timing file exists and paint reuses it.
        let _ = termy::x_panel::art::load();
        let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
        let scene = scene.to_string();
        let _ = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui_kit::TitlebarOptions {
                    title: Some("Termy X Preview".into()),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                #[cfg(target_os = "linux")]
                window_decorations: Some(gpui_kit::WindowDecorations::Server),
                kind: WindowKind::Normal,
                is_resizable: true,
                window_min_size: Some(size(px(800.0), px(600.0))),
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title("Termy X Preview");
                cx.new(move |cx| {
                    let mut panel = termy::x_panel::XPanel::preview(&scene, cx);
                    if bench_tabs {
                        panel.bench_tabs(cx);
                    }
                    panel
                })
            },
        );
    });
}
