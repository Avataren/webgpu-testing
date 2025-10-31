#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
#[cfg(target_arch = "wasm32")]
use std::path::{Path, PathBuf};

use super::{SceneImportDevice, SceneLoadContext};
use crate::renderer::Texture;

#[derive(Debug, Clone)]
pub(super) struct ImportedTexture {
    pub index: u32,
    pub canonical_path: Option<PathBuf>,
    pub display_name: Option<String>,
}

pub(super) fn load_textures<D: SceneImportDevice>(
    ctx: &mut SceneLoadContext<'_, D>,
    document: &gltf::Document,
    images: &[gltf::image::Data],
) -> Result<Vec<ImportedTexture>, String> {
    log::info!("Loading textures...");
    let mut textures = Vec::new();

    for gltf_texture in document.textures() {
        let source = gltf_texture.source();
        let base_name = gltf_texture.name().map(|name| name.to_string());

        match source.source() {
            gltf::image::Source::Uri { uri, .. } => {
                let decoded = crate::io::percent_decode_uri(uri).ok();
                let mut candidates = Vec::new();

                if let Some(ref decoded_path) = decoded {
                    candidates.push(ctx.base_dir().join(decoded_path));
                }

                if decoded.as_deref() != Some(uri) {
                    candidates.push(ctx.base_dir().join(uri));
                }

                if candidates.is_empty() {
                    candidates.push(ctx.base_dir().join(uri));
                }

                let mut last_error: Option<String> = None;
                let mut loaded: Option<ImportedTexture> = None;

                for candidate in candidates {
                    let result = {
                        let scene = &mut *ctx.scene;
                        let renderer = &mut *ctx.renderer;
                        scene
                            .assets
                            .get_or_load_texture(renderer, &candidate, false)
                    };

                    match result {
                        Ok(handle) => {
                            let canonical = candidate.canonicalize().unwrap_or(candidate.clone());
                            let display_name = base_name.clone().or_else(|| {
                                canonical
                                    .file_name()
                                    .map(|name| name.to_string_lossy().to_string())
                            });

                            loaded = Some(ImportedTexture {
                                index: handle.index() as u32,
                                canonical_path: Some(canonical),
                                display_name,
                            });
                            break;
                        }
                        Err(err) => {
                            last_error = Some(err);
                        }
                    }
                }

                let texture = loaded.ok_or_else(|| {
                    last_error.unwrap_or_else(|| {
                        format!(
                            "Failed to load image {:?}",
                            decoded
                                .as_ref()
                                .map(|path| ctx.base_dir().join(path))
                                .unwrap_or_else(|| ctx.base_dir().join(uri))
                        )
                    })
                })?;

                textures.push(texture);
            }
            gltf::image::Source::View { .. } => {
                let img_data = &images[source.index()];
                log::debug!(
                    "  Loading embedded texture: {}x{}",
                    img_data.width,
                    img_data.height
                );

                let texture = {
                    let renderer_ref = &*ctx.renderer;
                    Texture::from_bytes(
                        renderer_ref.device(),
                        renderer_ref.queue(),
                        &img_data.pixels,
                        img_data.width,
                        img_data.height,
                        Some(&format!("EmbeddedTexture_{}", source.index())),
                    )
                };

                let handle = {
                    let scene = &mut *ctx.scene;
                    scene.assets.textures.insert(texture)
                };
                let display_name = base_name
                    .clone()
                    .or_else(|| Some(format!("EmbeddedTexture_{}", source.index())));

                textures.push(ImportedTexture {
                    index: handle.index() as u32,
                    canonical_path: None,
                    display_name,
                });
            }
        }
    }

    log::info!("Loaded {} textures", textures.len());
    Ok(textures)
}

#[cfg(target_arch = "wasm32")]
pub(super) fn import_images_web(
    document: &gltf::Document,
    base: Option<&Path>,
    buffers: &[gltf::buffer::Data],
) -> Result<Vec<gltf::image::Data>, String> {
    let mut images = Vec::new();

    for image in document.images() {
        let data = match image.source() {
            gltf::image::Source::Uri { uri, .. } => {
                let bytes = load_external_resource(base, uri, None)?;
                decode_image(&bytes)?
            }
            gltf::image::Source::View { view, .. } => {
                let parent = &buffers[view.buffer().index()].0;
                let begin = view.offset();
                let end = begin + view.length();
                if end > parent.len() {
                    return Err(format!(
                        "Image view for image {} is out of bounds",
                        image.index()
                    ));
                }
                decode_image(&parent[begin..end])?
            }
        };

        images.push(data);
    }

    Ok(images)
}

#[cfg(target_arch = "wasm32")]
pub(super) fn load_external_resource(
    base: Option<&Path>,
    uri: &str,
    original_path: Option<&Path>,
) -> Result<Vec<u8>, String> {
    if let Some(rest) = uri.strip_prefix("data:") {
        let (_, encoded) = rest
            .split_once(",")
            .ok_or_else(|| format!("Malformed data URI: {}", uri))?;
        return base64::decode(encoded)
            .map_err(|err| format!("Failed to decode data URI: {}", err));
    }

    if uri.starts_with("http://") || uri.starts_with("https://") {
        return crate::io::load_binary_from_str(uri);
    }

    let path = if uri.starts_with('/') {
        PathBuf::from(uri.trim_start_matches('/'))
    } else if let Some(base_path) = base {
        base_path.join(uri)
    } else if let Some(orig) = original_path {
        orig.parent()
            .map(|p| p.join(uri))
            .ok_or_else(|| format!("Cannot resolve URI {}", uri))?
    } else {
        return Err(format!("Cannot resolve URI {}", uri));
    };

    crate::io::load_binary(&path)
}

#[cfg(target_arch = "wasm32")]
fn decode_image(bytes: &[u8]) -> Result<gltf::image::Data, String> {
    use image::GenericImageView;

    let image = image::load_from_memory(bytes)
        .map_err(|err| format!("Failed to decode image data: {}", err))?;

    let format = match &image {
        image::DynamicImage::ImageLuma8(_) => gltf::image::Format::R8,
        image::DynamicImage::ImageLumaA8(_) => gltf::image::Format::R8G8,
        image::DynamicImage::ImageRgb8(_) => gltf::image::Format::R8G8B8,
        image::DynamicImage::ImageRgba8(_) => gltf::image::Format::R8G8B8A8,
        image::DynamicImage::ImageLuma16(_) => gltf::image::Format::R16,
        image::DynamicImage::ImageLumaA16(_) => gltf::image::Format::R16G16,
        image::DynamicImage::ImageRgb16(_) => gltf::image::Format::R16G16B16,
        image::DynamicImage::ImageRgba16(_) => gltf::image::Format::R16G16B16A16,
        image::DynamicImage::ImageRgb32F(_) => gltf::image::Format::R32G32B32FLOAT,
        image::DynamicImage::ImageRgba32F(_) => gltf::image::Format::R32G32B32A32FLOAT,
        other => return Err(format!("Unsupported image format: {:?}", other.color())),
    };

    let (width, height) = image.dimensions();
    let pixels = image.into_bytes();

    Ok(gltf::image::Data {
        pixels,
        format,
        width,
        height,
    })
}
