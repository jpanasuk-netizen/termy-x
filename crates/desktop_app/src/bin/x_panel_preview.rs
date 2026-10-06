//! Opens the Termy X panel on mock data so the bird backdrop can be reviewed
//! without a live X session.
//!
//! ```sh
//! cargo run -p termy --bin x_panel_preview -- timeline
//! TERMY_X_PREVIEW_WIDTH=3440 TERMY_X_PREVIEW_HEIGHT=900 cargo run -p termy --bin x_panel_preview -- timeline
//! ```

use gpui_kit::{AppContext, Bounds, WindowBounds, WindowKind, WindowOptions, px, size};

fn main() {
    if std::env::var("TERMY_X_BENCH_SVG").is_ok() {
        let started = std::time::Instant::now();
        let image = gpui_kit::Image::from_bytes(
            gpui_kit::ImageFormat::Svg,
            termy_x::splash::BIRD_SVG.as_bytes().to_vec(),
        );
        match image.to_image_data(gpui_kit::SvgRenderer::new(std::sync::Arc::new(()))) {
            Ok(rendered) => {
                eprintln!(
                    "svg raster ok in {:?} size {:?}",
                    started.elapsed(),
                    rendered.size(0)
                );
            }
            Err(error) => eprintln!("svg raster failed in {:?}: {error}", started.elapsed()),
        }
        return;
    }
    let scene = std::env::args().nth(1).unwrap_or_else(|| "timeline".into());
    let width = std::env::var("TERMY_X_PREVIEW_WIDTH")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(1440.0);
    let height = std::env::var("TERMY_X_PREVIEW_HEIGHT")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(900.0);

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
        let scene = scene.clone();
        let _ = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui_kit::TitlebarOptions {
                    title: Some("Termy X".into()),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                #[cfg(target_os = "linux")]
                window_decorations: Some(gpui_kit::WindowDecorations::Server),
                kind: WindowKind::Normal,
                is_resizable: true,
                window_min_size: Some(size(px(480.0), px(320.0))),
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title("Termy X");
                cx.new(move |cx| termy::x_panel::XPanel::preview(&scene, cx))
            },
        );
    });
}
