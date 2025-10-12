// src/renderer/shader_builder.rs
// Modular shader composition system

/// Builder for composing shaders from modular components
pub struct ShaderBuilder {
    modules: Vec<&'static str>,
}

impl ShaderBuilder {
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
        }
    }
    
    /// Add constants (PI, TWO_PI, etc.)
    pub fn with_constants(mut self) -> Self {
        self.modules.push(include_str!("../shader/constants.wgsl"));
        self
    }
    
    /// Add texture bindings (bindless or traditional)
    pub fn with_bindings(mut self, bindless: bool) -> Self {
        if bindless {
            self.modules.push(include_str!("../shader/bindings_bindless.wgsl"));
        } else {
            self.modules.push(include_str!("../shader/bindings_traditional.wgsl"));
        }
        self
    }
    
    /// Add core PBR lighting functions and light structures
    pub fn with_lighting(mut self) -> Self {
        self.modules.push(include_str!("../shader/lighting_common.wgsl"));
        self
    }
    
    /// Add shadow sampling functions
    pub fn with_shadows(mut self) -> Self {
        self.modules.push(include_str!("../shader/shadows.wgsl"));
        self
    }
    
    /// Add environment lighting (IBL)
    pub fn with_environment(mut self) -> Self {
        self.modules.push(include_str!("../shader/environment.wgsl"));
        self
    }
    
    /// Add complete scene lighting with shadows
    pub fn with_lighting_and_shadows(mut self) -> Self {
        self.modules.push(include_str!("../shader/lighting_with_shadows.wgsl"));
        self
    }
    
    /// Add legacy PBR lighting (for backward compatibility)
    #[deprecated(note = "Use with_lighting() + with_shadows() instead")]
    pub fn with_pbr_lighting(mut self) -> Self {
        self.modules.push(include_str!("../shader/pbr_lighting.wgsl"));
        self
    }
    
    /// Build the final shader source by appending the main shader
    pub fn build(self, main_shader: &'static str) -> String {
        let mut source = String::with_capacity(
            self.modules.iter().map(|m| m.len()).sum::<usize>() 
            + main_shader.len() 
            + self.modules.len() * 2 // newlines
        );
        
        // Add all modules in order
        for module in self.modules {
            source.push_str(module);
            source.push_str("\n\n");
        }
        
        // Add main shader
        source.push_str(main_shader);
        
        source
    }
    
    /// Build without a main shader (for testing or custom usage)
    pub fn build_modules_only(self) -> String {
        let mut source = String::new();
        for module in self.modules {
            source.push_str(module);
            source.push_str("\n\n");
        }
        source
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
    pub fn full_pbr(bindless: bool) -> Self {
        Self::new()
            .with_constants()
            .with_bindings(bindless)
            .with_lighting()
            .with_shadows()
            .with_environment()
            .with_lighting_and_shadows()
    }
    
    /// Particle shader (lighting but no shadows)
    pub fn particles(bindless: bool) -> Self {
        Self::new()
            .with_constants()
            .with_bindings(bindless)
            .with_lighting()
            .with_environment()
    }
    
    /// Simple shader (no lighting, just textures)
    pub fn unlit(bindless: bool) -> Self {
        Self::new()
            .with_constants()
            .with_bindings(bindless)
    }
    
    /// Background/skybox shader
    pub fn background() -> Self {
        Self::new()
            .with_constants()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_builder_basic() {
        let shader = ShaderBuilder::new()
            .with_constants()
            .build("fn main() {}");
        
        assert!(shader.contains("const PI:"));
        assert!(shader.contains("fn main()"));
    }
    
    #[test]
    fn test_builder_full_pbr() {
        let shader = ShaderBuilder::full_pbr(true)
            .build(include_str!("../shader/common.wgsl"));
        
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
        let shader = ShaderBuilder::particles(true)
            .build(include_str!("../shader/particle_render.wgsl"));
        
        // Should have lighting but not shadows
        assert!(shader.contains("calculate_light_contribution"));
        assert!(shader.contains("calculate_environment_lighting"));
        assert!(!shader.contains("sample_directional_shadow"));
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
}