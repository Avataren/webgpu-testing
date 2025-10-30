// src/renderer/shader_builder.rs
// Modular shader composition system

use std::convert::TryFrom;

use crate::renderer::{
    lights::{MAX_DIRECTIONAL_LIGHTS, MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS},
    MAX_TEXTURES,
};

const MATERIAL_COMMON: &str = include_str!("../shader/material_common.wgsl");
const CONSTANTS: &str = include_str!("../shader/constants.wgsl");
const BINDINGS_BINDLESS_LINEAR: &str = include_str!("../shader/bindings_bindless_linear.wgsl");
const BINDINGS_BINDLESS_NEAREST: &str = include_str!("../shader/bindings_bindless_nearest.wgsl");
const BINDINGS_TRADITIONAL_LINEAR: &str =
    include_str!("../shader/bindings_traditional_linear.wgsl");
const BINDINGS_TRADITIONAL_NEAREST: &str =
    include_str!("../shader/bindings_traditional_nearest.wgsl");
const LIGHTING_COMMON: &str = include_str!("../shader/lighting_common.wgsl");
const SHADOWS: &str = include_str!("../shader/shadows.wgsl");
const ENVIRONMENT: &str = include_str!("../shader/environment.wgsl");
const LIGHTING_WITH_SHADOWS: &str = include_str!("../shader/lighting_with_shadows.wgsl");

const MARKER_INCLUDE_MATERIAL_SYSTEM: &str = "// @include_material_system";
const MARKER_INCLUDE_LIGHTING: &str = "// @include_lighting";
const MARKER_INCLUDE_SHADOWS: &str = "// @include_shadows";
const MARKER_INCLUDE_ENVIRONMENT: &str = "// @include_environment";

struct ShaderConstant {
    name: &'static str,
    value: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SamplerFilterMode {
    Linear,
    Nearest,
}

/// Builder for composing shaders from modular components
pub struct ShaderBuilder {
    modules: Vec<&'static str>,
    constant_overrides: Vec<ShaderConstant>,
}

impl ShaderBuilder {
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
            constant_overrides: Vec::new(),
        }
    }

    /// Share the material data layout and flag constants across shaders.
    pub fn with_material_system(mut self) -> Self {
        self.ensure_with_material_system();
        self
    }

    /// Add an arbitrary shader module to the builder.
    pub fn with_module(mut self, module: &'static str) -> Self {
        self.ensure_module(module);
        self
    }

    /// Add constants (PI, TWO_PI, etc.)
    pub fn with_constants(mut self) -> Self {
        self.ensure_module(CONSTANTS);
        self.set_constant("MAX_DIRECTIONAL_LIGHTS", MAX_DIRECTIONAL_LIGHTS as u32);
        self.set_constant("MAX_POINT_LIGHTS", MAX_POINT_LIGHTS as u32);
        self.set_constant("MAX_SPOT_LIGHTS", MAX_SPOT_LIGHTS as u32);
        self.set_constant(
            "MAX_TEXTURES",
            u32::try_from(MAX_TEXTURES).expect("MAX_TEXTURES must fit in u32"),
        );
        self
    }

    /// Add texture bindings (bindless or traditional)
    pub fn with_bindings_for_filter(
        mut self,
        bindless: bool,
        filtering: SamplerFilterMode,
    ) -> Self {
        if bindless {
            match filtering {
                SamplerFilterMode::Linear => self.ensure_module(BINDINGS_BINDLESS_LINEAR),
                SamplerFilterMode::Nearest => self.ensure_module(BINDINGS_BINDLESS_NEAREST),
            }
        } else {
            match filtering {
                SamplerFilterMode::Linear => self.ensure_module(BINDINGS_TRADITIONAL_LINEAR),
                SamplerFilterMode::Nearest => self.ensure_module(BINDINGS_TRADITIONAL_NEAREST),
            }
        }
        self
    }

    pub fn with_bindings(self, bindless: bool) -> Self {
        self.with_bindings_for_filter(bindless, SamplerFilterMode::Linear)
    }

    /// Add core PBR lighting functions and light structures
    pub fn with_lighting(mut self) -> Self {
        self.ensure_with_lighting();
        self
    }

    /// Add shadow sampling functions
    pub fn with_shadows(mut self) -> Self {
        self.ensure_with_shadows();
        self
    }

    /// Add environment lighting (IBL)
    pub fn with_environment(mut self) -> Self {
        self.ensure_with_environment();
        self
    }

    /// Add complete scene lighting with shadows
    pub fn with_lighting_and_shadows(mut self) -> Self {
        self.ensure_module(LIGHTING_WITH_SHADOWS);
        self
    }

    /// Add legacy PBR lighting (for backward compatibility)
    #[deprecated(note = "Use with_lighting() + with_shadows() instead")]
    pub fn with_pbr_lighting(mut self) -> Self {
        self.modules
            .push(include_str!("../shader/pbr_lighting.wgsl"));
        self
    }

    /// Build the final shader source by appending the main shader
    pub fn build(mut self, main_shader: &str) -> String {
        self = self.apply_markers(main_shader);
        let mut source = String::with_capacity(
            self.modules.iter().map(|m| m.len()).sum::<usize>()
                + main_shader.len()
                + self.modules.len() * 2, // newlines
        );

        // Add all modules in order
        for module in &self.modules {
            source.push_str(module);
            source.push_str("\n\n");
        }

        // Add main shader
        source.push_str(main_shader);

        self.finalize(source)
    }

    /// Build without a main shader (for testing or custom usage)
    pub fn build_modules_only(self) -> String {
        let mut source = String::new();
        for module in &self.modules {
            source.push_str(module);
            source.push_str("\n\n");
        }
        self.finalize(source)
    }
}

impl Default for ShaderBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Preset configurations for common shader types
impl ShaderBuilder {
    /// Complete PBR shader with all features (main geometry)
    pub fn full_pbr_filtered(bindless: bool, filtering: SamplerFilterMode) -> Self {
        Self::new()
            .with_material_system()
            .with_constants()
            .with_bindings_for_filter(bindless, filtering)
            .with_lighting()
            .with_shadows()
            .with_environment()
            .with_lighting_and_shadows()
    }

    pub fn full_pbr(bindless: bool) -> Self {
        Self::full_pbr_filtered(bindless, SamplerFilterMode::Linear)
    }

    /// Particle shader with full lighting support (including shadows)
    pub fn particles_filtered(bindless: bool, filtering: SamplerFilterMode) -> Self {
        Self::new()
            .with_material_system()
            .with_constants()
            .with_bindings_for_filter(bindless, filtering)
            .with_lighting()
            .with_shadows()
            .with_environment()
            .with_lighting_and_shadows()
    }

    pub fn particles(bindless: bool) -> Self {
        Self::particles_filtered(bindless, SamplerFilterMode::Linear)
    }

    /// Simple shader (no lighting, just textures)
    pub fn unlit_filtered(bindless: bool, filtering: SamplerFilterMode) -> Self {
        Self::new()
            .with_material_system()
            .with_constants()
            .with_bindings_for_filter(bindless, filtering)
    }

    pub fn unlit(bindless: bool) -> Self {
        Self::unlit_filtered(bindless, SamplerFilterMode::Linear)
    }

    /// Background/skybox shader
    pub fn background() -> Self {
        Self::new().with_constants()
    }
}

impl ShaderBuilder {
    fn ensure_module(&mut self, module: &'static str) {
        if !self.modules.contains(&module) {
            self.modules.push(module);
        }
    }

    fn ensure_with_material_system(&mut self) {
        self.ensure_module(MATERIAL_COMMON);
    }

    fn ensure_with_lighting(&mut self) {
        self.ensure_module(LIGHTING_COMMON);
    }

    fn ensure_with_shadows(&mut self) {
        self.ensure_module(LIGHTING_COMMON);
        self.ensure_module(SHADOWS);
        self.ensure_module(LIGHTING_WITH_SHADOWS);
    }

    fn ensure_with_environment(&mut self) {
        self.ensure_module(ENVIRONMENT);
    }

    fn apply_markers(mut self, shader_source: &str) -> Self {
        if shader_source.contains(MARKER_INCLUDE_MATERIAL_SYSTEM) {
            self.ensure_with_material_system();
        }

        if shader_source.contains(MARKER_INCLUDE_LIGHTING) {
            self.ensure_with_lighting();
        }

        if shader_source.contains(MARKER_INCLUDE_SHADOWS) {
            self.ensure_with_shadows();
        }

        if shader_source.contains(MARKER_INCLUDE_ENVIRONMENT) {
            self.ensure_with_environment();
        }

        self
    }

    fn set_constant(&mut self, name: &'static str, value: u32) {
        if let Some(existing) = self
            .constant_overrides
            .iter_mut()
            .find(|constant| constant.name == name)
        {
            existing.value = value;
        } else {
            self.constant_overrides.push(ShaderConstant { name, value });
        }
    }

    fn finalize(self, mut source: String) -> String {
        for constant in self.constant_overrides {
            Self::replace_constant(&mut source, constant.name, constant.value);
        }
        source
    }

    fn replace_constant(source: &mut String, name: &str, value: u32) {
        let declaration = format!("const {name}:");
        if let Some(declaration_index) = source.find(&declaration) {
            let after_declaration = declaration_index + declaration.len();
            if let Some(equals_offset) = source[after_declaration..].find('=') {
                let mut value_start = after_declaration + equals_offset + 1;
                while source[value_start..].starts_with(' ') {
                    value_start += 1;
                }

                if let Some(value_end_offset) = source[value_start..].find(';') {
                    let value_end = value_start + value_end_offset;
                    let replacement = format!("{value}u");
                    source.replace_range(value_start..value_end, &replacement);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::material::ShaderMaterialMetadata;
    use crate::renderer::{
        lights::{MAX_DIRECTIONAL_LIGHTS, MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS},
        MAX_TEXTURES,
    };
    use naga::front::wgsl;

    #[test]
    fn test_builder_basic() {
        let shader = ShaderBuilder::new().with_constants().build("fn main() {}");

        assert!(shader.contains("const PI:"));
        assert!(shader.contains("fn main()"));
        assert!(shader.contains(&format!(
            "const MAX_DIRECTIONAL_LIGHTS: u32 = {}u;",
            MAX_DIRECTIONAL_LIGHTS as u32
        )));
        assert!(shader.contains(&format!(
            "const MAX_POINT_LIGHTS: u32 = {}u;",
            MAX_POINT_LIGHTS as u32
        )));
        assert!(shader.contains(&format!(
            "const MAX_SPOT_LIGHTS: u32 = {}u;",
            MAX_SPOT_LIGHTS as u32
        )));
        assert!(shader.contains(&format!(
            "const MAX_TEXTURES: u32 = {}u;",
            u32::try_from(MAX_TEXTURES).expect("MAX_TEXTURES must fit in u32")
        )));
    }

    #[test]
    fn test_builder_full_pbr() {
        let shader = ShaderBuilder::full_pbr(true).build(include_str!("../shader/common.wgsl"));

        // Verify all components are present
        assert!(shader.contains("const PI:"));
        assert!(shader.contains("binding_array<texture_2d"));
        assert!(shader.contains("calculate_light_contribution"));
        assert!(shader.contains("sample_directional_shadow"));
        assert!(shader.contains("calculate_environment_lighting"));
        assert!(shader.contains("calculate_scene_lighting"));
        assert!(shader.contains("fn fs_main"));
    }

    #[test]
    fn test_builder_particles() {
        let shader =
            ShaderBuilder::particles(true).build(include_str!("../shader/particle_render.wgsl"));

        // Should have lighting and shadow sampling support
        assert!(shader.contains("calculate_light_contribution"));
        assert!(shader.contains("calculate_environment_lighting"));
        assert!(shader.contains("sample_directional_shadow"));
    }

    #[test]
    fn test_bindless_vs_traditional() {
        let bindless = ShaderBuilder::new()
            .with_bindings(true)
            .build_modules_only();

        let traditional = ShaderBuilder::new()
            .with_bindings(false)
            .build_modules_only();

        assert!(bindless.contains("binding_array"));
        assert!(traditional.contains("base_color_texture_binding"));
        assert_ne!(bindless, traditional);
    }

    #[test]
    fn include_markers_add_expected_modules() {
        let shader = ShaderBuilder::new()
            .with_constants()
            .build(
                "// @include_material_system\n// @include_lighting\n@fragment fn fs_main() -> @location(0) vec4<f32> {\n    return vec4<f32>(0.0, 0.0, 0.0, 1.0);\n}\n",
            );

        assert!(shader.contains("struct MaterialData"));
        assert!(shader.contains("struct Lights"));
    }

    #[test]
    fn default_template_compiles_with_naga() {
        let metadata = ShaderMaterialMetadata::default_template();
        let source = ShaderBuilder::new()
            .with_constants()
            .with_bindings(true)
            .build(metadata.wgsl_source());

        wgsl::parse_str(&source).expect("default template should produce valid WGSL");
    }
}
