use wgpu_cube::scene::MeshComponent;

/// Displays the mesh component (just shows the handle index)
pub fn show_mesh_section(ui: &mut egui::Ui, mesh: MeshComponent) {
    ui.collapsing("Mesh", |ui| {
        ui.label(format!("Handle index: {}", mesh.0.index()));
    });
}
