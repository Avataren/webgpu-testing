use wgpu_cube::*;

fn main() {
    #[cfg(not(feature = "egui"))]
    {
        eprintln!("This example requires the 'egui' feature!");
        eprintln!("Run with: cargo run --example egui_demo --features egui");
        std::process::exit(1);
    }

    #[cfg(feature = "egui")]
    {
        let mut app = App::new();

        // Set up the egui UI
        app.set_egui_ui(|ctx| {
            egui::Window::new("Demo Window")
                .default_pos([10.0, 10.0])
                .show(ctx, |ui| {
                    ui.heading("Hello egui 0.32!");
                    ui.separator();

                    ui.label("This is rendered as an overlay on top of your 3D scene.");

                    if ui.button("Click me!").clicked() {
                        println!("Button clicked!");
                    }

                    ui.separator();

                    // Frame time display
                    let frame_time = ctx.input(|i| i.stable_dt * 1000.0);
                    ui.label(format!("Frame time: {:.2}ms", frame_time));
                    ui.label(format!("FPS: {:.0}", 1000.0 / frame_time));
                });

            // Example of a side panel
            egui::SidePanel::left("left_panel")
                .default_width(200.0)
                .show(ctx, |ui| {
                    ui.heading("Side Panel");
                    ui.label("You can add controls here");

                    ui.separator();

                    if ui.button("Reset").clicked() {
                        println!("Reset clicked!");
                    }
                });
        });

        wgpu_cube::run_with_app(app).unwrap();
    }
}
