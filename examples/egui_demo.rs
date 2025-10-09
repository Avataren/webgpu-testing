#[cfg(feature = "egui")]
use wgpu_cube::App;

#[cfg(feature = "egui")]
use wgpu_cube::ui::{init_log_recorder, LogWindow, StatsWindow};

fn main() {
    #[cfg(not(feature = "egui"))]
    {
        eprintln!("This example requires the 'egui' feature!");
        eprintln!("Run with: cargo run --example egui_demo --features egui");
        std::process::exit(1);
    }

    #[cfg(feature = "egui")]
    {
        let log_handle = init_log_recorder();
        let mut app = App::new();
        let stats_handle = app.frame_stats_handle();
        let mut stats_window = StatsWindow::new(stats_handle);
        let mut log_window = LogWindow::new(log_handle);
        let mut stats_open = true;
        let mut log_open = true;

        app.set_egui_ui(move |ctx| {
            stats_window.show(ctx, Some(&mut stats_open));
            log_window.show(ctx, Some(&mut log_open));
        });

        wgpu_cube::run_with_app(app).unwrap();
    }
}
