use crate::environment::{ColorGrading, Environment, HdrBackground};
use crate::renderer::Renderer;
use crate::scene::{Scene, SceneAsset, SceneAssetBundle, SceneAssetResources};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const PROJECT_FILE_NAME: &str = "project.json";
pub const CONTENT_DIR: &str = "content";
const PROJECT_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("scene has no serializable content")]
    EmptyScene,
    #[error("failed to access the filesystem: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to (de)serialize project data: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("environment HDR image has no file name: {0}")]
    InvalidEnvironmentPath(PathBuf),
    #[error("environment HDR image not found: {0}")]
    MissingEnvironmentFile(PathBuf),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectMetadata {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

impl Default for ProjectMetadata {
    fn default() -> Self {
        Self {
            name: "Untitled Project".to_string(),
            description: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    version: u32,
    pub metadata: ProjectMetadata,
    pub scene: SceneAsset,
    pub environment: SerializedEnvironment,
}

impl ProjectManifest {
    /// Creates a manifest with default scene and environment data.
    ///
    /// # Examples
    ///
    /// ```
    /// use wgpu_cube::project::{ProjectManifest, ProjectMetadata};
    ///
    /// let manifest = ProjectManifest::new_empty(ProjectMetadata::default());
    /// assert_eq!(manifest.metadata.name, "Untitled Project");
    /// ```
    pub fn new_empty(metadata: ProjectMetadata) -> Self {
        let environment = SerializedEnvironment::from_environment(&Environment::default());
        Self {
            version: PROJECT_VERSION,
            scene: SceneAsset::builder(metadata.name.clone()).build(),
            metadata,
            environment,
        }
    }

    pub fn capture(scene: &Scene, metadata: ProjectMetadata) -> Result<Self, ProjectError> {
        let asset = scene
            .export_main_asset(metadata.name.clone())
            .ok_or(ProjectError::EmptyScene)?;
        let environment = SerializedEnvironment::from_environment(scene.environment());
        Ok(Self {
            version: PROJECT_VERSION,
            metadata,
            scene: asset,
            environment,
        })
    }

    pub fn save_to_dir(&self, dir: &Path) -> Result<(), ProjectError> {
        fs::create_dir_all(dir)?;

        let mut manifest = self.clone();
        manifest.environment.prepare_for_save(dir)?;

        let json = serde_json::to_string_pretty(&manifest)?;
        fs::write(dir.join(PROJECT_FILE_NAME), json)?;
        Ok(())
    }

    pub fn load_from_dir(dir: &Path) -> Result<Self, ProjectError> {
        let manifest_path = dir.join(PROJECT_FILE_NAME);
        let json = fs::read_to_string(&manifest_path)?;
        let mut manifest: ProjectManifest = serde_json::from_str(&json)?;
        manifest.environment.resolve_paths(dir)?;
        Ok(manifest)
    }

    pub fn from_json_str(json: &str) -> Result<Self, ProjectError> {
        let manifest: ProjectManifest = serde_json::from_str(json)?;
        Ok(manifest)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ProjectError> {
        let manifest: ProjectManifest = serde_json::from_slice(bytes)?;
        Ok(manifest)
    }

    pub fn instantiate_into(
        &self,
        scene: &mut Scene,
        renderer: &mut Renderer,
        project_root: &Path,
    ) -> Result<bool, ProjectError> {
        let mut new_scene = Scene::new();
        let environment = self.environment.clone().into_environment(project_root)?;
        new_scene.set_environment(environment);

        let mut bundle = SceneAssetBundle::new(self.scene.clone(), SceneAssetResources::default());
        let textures_changed = bundle.register_resources(renderer, &mut new_scene.assets);

        if !bundle.asset.entities.is_empty() {
            let main = new_scene.instantiate_asset(&bundle.asset, None);
            new_scene.set_main_scene(main);
        }

        *scene = new_scene;
        Ok(textures_changed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedEnvironment {
    clear_color: [f64; 4],
    ambient_intensity: f32,
    color_grading: SerializedColorGrading,
    #[serde(default)]
    hdr: Option<SerializedHdrBackground>,
}

impl SerializedEnvironment {
    fn from_environment(environment: &Environment) -> Self {
        let clear = environment.clear_color();
        let grading = environment.color_grading();
        let hdr = environment
            .hdr_background()
            .map(|background| SerializedHdrBackground {
                enabled: background.enabled(),
                image_path: Some(background.path().to_path_buf()),
                intensity: background.intensity(),
                source_path: Some(background.path().to_path_buf()),
            });

        Self {
            clear_color: [clear.r, clear.g, clear.b, clear.a],
            ambient_intensity: environment.ambient_intensity(),
            color_grading: SerializedColorGrading::from(grading),
            hdr,
        }
    }

    fn prepare_for_save(&mut self, project_dir: &Path) -> Result<(), ProjectError> {
        if let Some(hdr) = self.hdr.as_mut() {
            let Some(mut source) = hdr.source_path.clone().or_else(|| hdr.image_path.clone())
            else {
                return Ok(());
            };

            if !source.is_absolute() {
                source = std::env::current_dir()?.join(source);
            }

            if !source.exists() {
                return Err(ProjectError::MissingEnvironmentFile(source));
            }

            let Some(file_name) = source.file_name() else {
                return Err(ProjectError::InvalidEnvironmentPath(source));
            };

            let content_dir = project_dir.join(CONTENT_DIR);
            fs::create_dir_all(&content_dir)?;
            let target = content_dir.join(file_name);
            if source != target {
                fs::copy(&source, &target)?;
            }

            hdr.image_path = Some(PathBuf::from(CONTENT_DIR).join(file_name));
        }

        Ok(())
    }

    fn resolve_paths(&mut self, project_dir: &Path) -> Result<(), ProjectError> {
        if let Some(hdr) = self.hdr.as_mut() {
            if let Some(path) = hdr.image_path.clone() {
                let absolute = if path.is_absolute() {
                    path
                } else {
                    project_dir.join(path)
                };

                #[cfg(not(target_arch = "wasm32"))]
                if !absolute.exists() {
                    return Err(ProjectError::MissingEnvironmentFile(absolute));
                }

                hdr.source_path = Some(absolute);
            }
        }

        Ok(())
    }

    fn into_environment(self, project_root: &Path) -> Result<Environment, ProjectError> {
        let mut environment = Environment::new(wgpu::Color {
            r: self.clear_color[0],
            g: self.clear_color[1],
            b: self.clear_color[2],
            a: self.clear_color[3],
        });
        environment.set_ambient_intensity(self.ambient_intensity);
        environment.set_color_grading(self.color_grading.into());

        if let Some(hdr) = self.hdr {
            if let Some(path) = hdr
                .source_path
                .or_else(|| hdr.image_path.map(|p| project_root.join(p)))
            {
                if !path.exists() {
                    return Err(ProjectError::MissingEnvironmentFile(path));
                }

                let mut background = HdrBackground::new(path);
                background.set_enabled(hdr.enabled);
                background.set_intensity(hdr.intensity);
                environment.set_hdr_background(Some(background));
            }
        }

        Ok(environment)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializedColorGrading {
    exposure: f32,
    saturation: f32,
    contrast: f32,
}

impl From<ColorGrading> for SerializedColorGrading {
    fn from(grading: ColorGrading) -> Self {
        Self {
            exposure: grading.exposure(),
            saturation: grading.saturation(),
            contrast: grading.contrast(),
        }
    }
}

impl From<SerializedColorGrading> for ColorGrading {
    fn from(serialized: SerializedColorGrading) -> Self {
        let mut grading = ColorGrading::default();
        grading.set_exposure(serialized.exposure);
        grading.set_saturation(serialized.saturation);
        grading.set_contrast(serialized.contrast);
        grading
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializedHdrBackground {
    enabled: bool,
    image_path: Option<PathBuf>,
    intensity: f32,
    #[serde(skip)]
    source_path: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn environment_roundtrip_preserves_hdr_path() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let hdr_path = tmp_dir.path().join("env.hdr");
        let mut file = fs::File::create(&hdr_path).unwrap();
        writeln!(file, "dummy").unwrap();

        let mut environment = Environment::default();
        environment.enable_hdr_background(&hdr_path);
        environment.hdr_background_mut().unwrap().set_intensity(2.0);

        let mut serialized = SerializedEnvironment::from_environment(&environment);
        serialized.prepare_for_save(tmp_dir.path()).unwrap();

        let manifest_path = tmp_dir.path().join(PROJECT_FILE_NAME);
        let json = serde_json::to_string(&ProjectManifest {
            version: PROJECT_VERSION,
            metadata: ProjectMetadata::default(),
            scene: SceneAsset::builder("Test").build(),
            environment: serialized.clone(),
        })
        .unwrap();
        fs::write(&manifest_path, json).unwrap();

        let mut loaded_env = serialized;
        loaded_env.resolve_paths(tmp_dir.path()).unwrap();
        let environment = loaded_env.into_environment(tmp_dir.path()).unwrap();
        let background = environment.hdr_background().unwrap();
        assert!(background.path().ends_with("env.hdr"));
    }
}
