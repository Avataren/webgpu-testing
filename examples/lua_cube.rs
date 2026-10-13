//! A cube driven by an inline Lua script. Runs natively and on the web
//! (`./build_web.sh lua_cube`), where it checks that the Lua VM works in wasm.

use glam::Vec3;
use std::path::PathBuf;
use wgpu_cube::app::{StartupContext, UpdateContext};
use wgpu_cube::asset::MaterialAsset;
use wgpu_cube::render_application::{run_application, RenderApplication};
use wgpu_cube::renderer::Material;
use wgpu_cube::scene::EntityBuilder;
use wgpu_cube::scripting::LuaScriptSource;

const SCRIPT: &str = r#"
local t = 0

function on_created(self_entity)
    -- Errors unwind through setjmp/longjmp, which wasm builds lower to wasm exceptions.
    local ok, err = pcall(error, "caught")
    log_info(string.format("lua_cube: %s, pcall -> %s %s, 0.1 + 0.2 = %.17g, %s",
        _VERSION, tostring(ok), err, 0.1 + 0.2, os.date("!%Y-%m-%d", 0)))
end

function update(self_entity, dt)
    t = t + dt
    set_rotation(self_entity, t, t * 0.5, 0)
    local pulse = 0.5 + 0.5 * math.sin(t * 2)
    set_material_color(self_entity, 0.9, 0.2 + 0.6 * pulse, 0.2)
end
"#;

struct ExampleApp;

impl RenderApplication for ExampleApp {
    fn setup(&mut self, ctx: &mut StartupContext) {
        let (verts, idx) = wgpu_cube::renderer::cube_mesh();
        let mesh = ctx.renderer.create_mesh(&verts, &idx);
        let mesh_handle = ctx.scene.assets.meshes.insert(mesh);
        let material_handle = ctx
            .scene
            .assets
            .materials
            .insert(MaterialAsset::from_material(
                Material::red(),
                PathBuf::from("examples/lua_cube/red"),
            ));
        EntityBuilder::new(&mut ctx.scene)
            .with_name("Lua Cube")
            .with_mesh(mesh_handle)
            .with_material(material_handle)
            .with_script(LuaScriptSource::inline("spin.lua", SCRIPT))
            .visible(true)
            .spawn();
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        let camera = ctx.scene.camera_mut();
        camera.eye = Vec3::new(0.0, 2.0, 5.0);
        camera.target = Vec3::ZERO;
        camera.up = Vec3::Y;
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    run_application(ExampleApp).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn start_app() {
    run_application(ExampleApp).unwrap();
}
