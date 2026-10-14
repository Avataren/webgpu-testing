//! Generate portable PNG material maps and PCM WAV effects in the active project.

use std::fs::{self, OpenOptions};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageFormat, RgbaImage};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

#[cfg(not(target_arch = "wasm32"))]
use crate::procgen::sound::SoundOptions;
use crate::procgen::texture::{HeightField, MapKind, MapOptions, TextureOptions, MAX_SIZE};
use crate::remote::json::{color_from_json, req_str};

use super::{RemoteContext, RemoteResult};

fn parse<T: DeserializeOwned>(params: &Value) -> Result<T, String> {
    serde_json::from_value(params.clone())
        .map_err(|err| format!("invalid generation options: {err}"))
}

fn flag(params: &Value, key: &str, default: bool) -> Result<bool, String> {
    match params.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| format!("{key} must be a boolean")),
    }
}

fn output_dir(params: &Value, kind: &str) -> Result<(String, PathBuf), String> {
    let name = req_str(params, "name")?;
    if name.is_empty()
        || name.len() > 80
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return Err("name must be 1..=80 ASCII letters, digits, underscores or hyphens".into());
    }
    let directory = match params.get("directory") {
        Some(Value::String(path)) if !path.is_empty() => crate::project::resolve_project_path(path),
        Some(_) => return Err("directory must be a nonempty path string".into()),
        None => crate::project::active_project_root()
            .ok_or("open a project first, or supply an output directory")?
            .join("content")
            .join(kind),
    };
    Ok((name.to_owned(), directory))
}

fn display_path(path: &Path) -> String {
    let absolute = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    crate::project::active_project_root()
        .and_then(|root| absolute.strip_prefix(root).ok().map(Path::to_path_buf))
        .unwrap_or(absolute)
        .to_string_lossy()
        .replace('\\', "/")
}

fn png(image: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image.clone())
        .write_to(&mut bytes, ImageFormat::Png)
        .map_err(|err| format!("cannot encode texture: {err}"))?;
    Ok(bytes.into_inner())
}

/// Check every destination before writing any file. Exclusive creation protects existing
/// art unless replacement was explicitly requested, including calls made without MCP.
fn write_bundle(files: &[(PathBuf, Vec<u8>)], overwrite: bool) -> Result<(), String> {
    for (path, _) in files {
        if path.exists() && !overwrite {
            return Err(format!(
                "{} already exists; choose another name or pass overwrite: true",
                path.display()
            ));
        }
        if path.exists() && !path.is_file() {
            return Err(format!("{} is not a file", path.display()));
        }
    }
    for (path, bytes) in files {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let mut options = OpenOptions::new();
        options.write(true);
        if overwrite {
            options.create(true).truncate(true);
        } else {
            options.create_new(true);
        }
        options
            .open(path)
            .and_then(|mut file| file.write_all(bytes))
            .map_err(|err| format!("cannot write {}: {err}", path.display()))?;
    }
    Ok(())
}

fn recipe(method: &str, params: &Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(&json!({ "generator": method, "version": 1, "parameters": params }))
        .map_err(|err| err.to_string())
}

fn map_kinds(params: &Value, from_image: bool) -> Result<Vec<MapKind>, String> {
    let maps: Vec<MapKind> = match params.get("maps") {
        Some(value) => parse(value)?,
        None if from_image => vec![
            MapKind::Normal,
            MapKind::MetallicRoughness,
            MapKind::Occlusion,
        ],
        None => vec![
            MapKind::BaseColor,
            MapKind::Height,
            MapKind::Normal,
            MapKind::MetallicRoughness,
            MapKind::Occlusion,
        ],
    };
    if maps.is_empty() || maps.len() > 7 {
        return Err("maps must contain 1..=7 map names".into());
    }
    for (index, kind) in maps.iter().enumerate() {
        if maps[..index].contains(kind) {
            return Err(format!("duplicate map {}", kind.name()));
        }
    }
    Ok(maps)
}

fn colors(params: &Value) -> Result<[[f32; 4]; 2], String> {
    match params.get("colors") {
        None => Ok([[0.12, 0.1, 0.08, 1.0], [0.7, 0.65, 0.55, 1.0]]),
        Some(Value::Array(items)) if items.len() == 2 => {
            Ok([color_from_json(&items[0])?, color_from_json(&items[1])?])
        }
        Some(_) => Err("colors must contain two colors, for low and high heights".into()),
    }
}

pub(super) fn texture_generate(
    params: &Value,
    ctx: &mut RemoteContext<'_, '_>,
    from_image: bool,
) -> RemoteResult {
    // Resolve before writing files, so a typo in the target doesn't leave unused assets.
    let entity = params
        .get("entity")
        .map(|value| crate::remote::json::entity_from_json(ctx.gpu.scene.main_world(), value))
        .transpose()?;
    let (mut report, files) = texture_files(params, from_image)?;
    let mut reloaded = Vec::new();
    let mut reload_errors = serde_json::Map::new();
    for path in &files {
        for (loaded, result) in ctx
            .gpu
            .scene
            .assets
            .reload_textures(&mut *ctx.gpu.renderer, Some(path))
        {
            match result {
                Ok(()) => reloaded.push(display_path(&loaded)),
                Err(err) => {
                    reload_errors.insert(display_path(&loaded), json!(err));
                }
            }
        }
    }
    let mut textures_loaded = !reloaded.is_empty();
    if let Some(entity) = entity {
        textures_loaded |= super::entity::patch_material(
            &mut ctx.gpu.scene,
            ctx.gpu.renderer,
            entity,
            &report["material"],
        )?;
        ctx.mark_modified();
        report["entity"] = json!(crate::remote::json::entity_id(entity));
    }
    if textures_loaded {
        ctx.gpu
            .renderer
            .update_texture_bind_group(&ctx.gpu.scene.assets);
    }
    report["textures_reloaded"] = json!(reloaded);
    report["reload_errors"] = json!(reload_errors);
    Ok(report)
}

fn texture_files(params: &Value, from_image: bool) -> Result<(Value, Vec<PathBuf>), String> {
    let (name, directory) = output_dir(params, "textures")?;
    let options: MapOptions = parse(params)?;
    options.validate()?;
    let kinds = map_kinds(params, from_image)?;
    let colors = colors(params)?;
    let overwrite = flag(params, "overwrite", false)?;
    let picture = flag(params, "image", true)?;
    let field = if from_image {
        let source = super::entity::resolve_existing_path(req_str(params, "source")?)?;
        let mut reader = image::ImageReader::open(&source)
            .map_err(|err| err.to_string())?
            .with_guessed_format()
            .map_err(|err| err.to_string())?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_SIZE);
        limits.max_image_height = Some(MAX_SIZE);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|err| format!("cannot read height image: {err}"))?
            .to_rgba8();
        HeightField::from_image(&image, &options)?
    } else {
        let texture: TextureOptions = parse(params)?;
        if !options.tileable {
            return Err("generated patterns are tileable; tileable: false is for texture_maps on an existing image".into());
        }
        texture.generate()?
    };
    let mut files = Vec::new();
    let mut paths = serde_json::Map::new();
    let mut previews = Vec::new();
    for kind in &kinds {
        let path = directory.join(format!("{name}_{}.png", kind.name()));
        let image = field.render(*kind, &options, colors);
        files.push((path, png(&image)?));
        if picture {
            previews.push(image::imageops::resize(
                &image,
                160,
                160,
                image::imageops::FilterType::Triangle,
            ));
        }
    }
    let method = if from_image {
        "texture.maps"
    } else {
        "texture.generate"
    };
    let recipe_path = directory.join(format!("{name}.texture.json"));
    files.push((recipe_path.clone(), recipe(method, params)?));
    write_bundle(&files, overwrite)?;
    for (kind, (path, _)) in kinds.iter().zip(&files) {
        paths.insert(kind.name().to_owned(), json!(display_path(path)));
    }
    let mut material = json!({});
    if let Some(path) = paths.get("base_color") {
        material["base_color_texture"] = path.clone();
        material["color"] = json!([1, 1, 1, 1]);
    }
    if let Some(path) = paths.get("normal") {
        material["normal_texture"] = path.clone();
    }
    if let Some(path) = paths.get("metallic_roughness").or_else(|| paths.get("orm")) {
        material["metallic_roughness_texture"] = path.clone();
        material["roughness"] = json!(1);
        material["metallic"] = json!(1);
    }
    if let Some(path) = paths.get("occlusion").or_else(|| paths.get("orm")) {
        material["occlusion_texture"] = path.clone();
    }
    let mut report = json!({
        "name": name, "size": [field.width, field.height], "paths": paths,
        "material": material, "recipe": display_path(&recipe_path),
        "normal_convention": if options.flip_y { "tangent space, Y flipped" } else { "tangent space, +Y" },
        "channels": { "metallic_roughness": "G = roughness, B = metallic", "orm": "R = occlusion, G = roughness, B = metallic" },
        "notes": "Height is linear data. Roughness and occlusion are height-based approximations. Data maps load as linear; base color loads as sRGB. Height and standalone roughness are export maps; use normal and packed metallic_roughness in engine materials."
    });
    if picture {
        let mut sheet = RgbaImage::new(previews.len() as u32 * 160, 160);
        for (index, preview) in previews.iter().enumerate() {
            image::imageops::replace(&mut sheet, preview, index as i64 * 160, 0);
        }
        report["png_base64"] = json!(base64::encode(png(&sheet)?));
        report["preview_order"] = json!(kinds.iter().map(|kind| kind.name()).collect::<Vec<_>>());
    }
    Ok((
        report,
        files
            .into_iter()
            .take(kinds.len())
            .map(|(path, _)| path)
            .collect(),
    ))
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn sound_generate(params: &Value) -> RemoteResult {
    let (name, directory) = output_dir(params, "audio")?;
    let options: SoundOptions = parse(params)?;
    let overwrite = flag(params, "overwrite", false)?;
    let picture = flag(params, "image", true)?;
    let sound = options.generate()?;
    let path = directory.join(format!("{name}.wav"));
    let recipe_path = directory.join(format!("{name}.sound.json"));
    write_bundle(
        &[
            (path.clone(), sound.wav()),
            (recipe_path.clone(), recipe("audio.generate", params)?),
        ],
        overwrite,
    )?;
    // Replacing a cached clip must affect the next play immediately.
    crate::audio::invalidate_file(&path);
    let (measurements, picture) = crate::audio_inspect::inspect(&path, picture)?;
    let mut report = json!({ "path": display_path(&path), "recipe": display_path(&recipe_path), "measurements": measurements });
    if let Some(picture) = picture {
        report["png_base64"] = json!(base64::encode(picture));
    }
    Ok(report)
}

#[cfg(target_arch = "wasm32")]
pub(super) fn sound_generate(_params: &Value) -> RemoteResult {
    Err("audio.generate writes files and needs a native build".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_map_selection_names_and_types() {
        assert!(map_kinds(&json!({"maps": ["normal", "normal"]}), false).is_err());
        assert!(map_kinds(&json!({"maps": []}), false).is_err());
        assert!(map_kinds(&json!({"maps": ["bogus"]}), false).is_err());
        assert!(output_dir(
            &json!({"name": "../outside", "directory": "/tmp"}),
            "textures"
        )
        .is_err());
        assert!(parse::<TextureOptions>(&json!({"size": [1.5, 32]})).is_err());
        assert!(parse::<MapOptions>(&json!({"flip_y": "yes"})).is_err());
    }

    #[test]
    fn collision_is_checked_before_writing_any_map() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.png");
        let second = dir.path().join("second.png");
        fs::write(&second, b"original").unwrap();
        let files = vec![(first.clone(), vec![1]), (second.clone(), vec![2])];
        assert!(write_bundle(&files, false).is_err());
        assert!(!first.exists());
        assert_eq!(fs::read(&second).unwrap(), b"original");
        write_bundle(&files, true).unwrap();
        assert_eq!(fs::read(&second).unwrap(), vec![2]);
    }

    #[test]
    fn png_maps_round_trip_and_height_images_can_be_reused() {
        let dir = tempfile::tempdir().unwrap();
        let args = json!({"name": "stone", "directory": dir.path(), "size": [32, 16],
            "pattern": "voronoi", "seed": 42, "roughness": 0.4, "metallic": 0.8,
            "maps": ["base_color", "height", "normal", "roughness", "metallic_roughness", "occlusion", "orm"]});
        let (report, files) = texture_files(&args, false).unwrap();
        assert_eq!(files.len(), 7);
        for path in &files {
            assert_eq!(image::open(path).unwrap().to_rgba8().dimensions(), (32, 16));
        }
        assert_eq!(report["material"]["roughness"], 1);
        assert_eq!(report["material"]["metallic"], 1);
        let packed = image::open(&files[4]).unwrap().to_rgba8();
        assert!(packed.pixels().all(|p| p[2] == 204));
        assert!(report["png_base64"].as_str().is_some());
        assert!(texture_files(&args, false).is_err());
        let derived = json!({"name": "derived", "directory": dir.path(),
            "source": report["paths"]["height"], "maps": ["normal", "orm"], "image": false,
            "channel": "red", "invert": true, "tileable": false, "flip_y": true});
        let (report, files) = texture_files(&derived, true).unwrap();
        assert_eq!(files.len(), 2);
        assert!(report.get("png_base64").is_none());
        assert_eq!(
            report["material"]["occlusion_texture"],
            report["material"]["metallic_roughness_texture"]
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn generated_sound_round_trips_through_engine_decoder_and_inspector() {
        let dir = tempfile::tempdir().unwrap();
        let args =
            json!({"name": "test", "directory": dir.path(), "preset": "laser", "image": false});
        let result = sound_generate(&args).unwrap();
        assert_eq!(result["measurements"]["sample_rate"], 44100);
        assert_eq!(result["measurements"]["channels"], 1);
        assert_eq!(result["measurements"]["clipped_samples"], 0);
        assert_eq!(result["measurements"]["duration_s"], 0.25);
        assert!(dir.path().join("test.wav").exists());
        assert!(dir.path().join("test.sound.json").exists());
        assert!(sound_generate(&args).is_err());
    }
}
