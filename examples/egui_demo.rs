#[cfg(feature = "egui")]
use wgpu_cube::app::StartupContext;
#[cfg(feature = "egui")]
use wgpu_cube::render_application::{run_application, DefaultUI, RenderApplication};

fn main() {
    #[cfg(not(feature = "egui"))]
    {
        eprintln!("This example requires the 'egui' feature!");
        eprintln!("Run with: cargo run --example egui_demo --features egui");
        std::process::exit(1);
    }

    #[cfg(feature = "egui")]
    {
        run_application(ExampleApp::default()).unwrap();
    }
}

#[cfg(feature = "egui")]
#[derive(Default)]
struct ExampleApp {
    stats_open: bool,
    log_open: bool,
}

#[cfg(feature = "egui")]
impl RenderApplication for ExampleApp {
    fn setup(&mut self, ctx: &mut StartupContext) {
        let _ = ctx;
    }

    fn show_default_ui(&self) -> bool {
        false
    }

    fn ui(&mut self, ctx: &egui::Context, default_ui: &mut DefaultUI) {
        default_ui
            .stats_window_mut()
            .show(ctx, Some(&mut self.stats_open));
        default_ui
            .log_window_mut()
            .show(ctx, Some(&mut self.log_open));

        egui::Window::new("Custom UI")
            .default_width(280.0)
            .show(ctx, |ui| {
                ui.label("This window is provided by ExampleApp::ui");
                ui.checkbox(&mut self.stats_open, "Show stats window");
                ui.checkbox(&mut self.log_open, "Show log window");
            });
    }
}

#[cfg(all(feature = "egui", target_arch = "wasm32"))]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn start_app() {
    run_application(ExampleApp::default()).unwrap();
}
