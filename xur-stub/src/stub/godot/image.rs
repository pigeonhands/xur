//! `godot::image` — extract pixel data from Godot textures.
//!
//! Card portrait textures are `AtlasTexture` subregions of a large
//! atlas.  `Texture2D.GetImage()` returns zero-filled data because
//! Godot doesn't keep CPU copies after GPU upload.  Instead we:
//!
//! 1. Detect `AtlasTexture` via `get_Atlas` / `get_Region`
//! 2. GPU-readback the atlas once via `RenderingServer.Texture2DGet`
//! 3. Cache the decompressed RGBA8 atlas by RID
//! 4. Crop to the per-card region in Rust
//!
//! All managed Image manipulation (Texture2DGet → Decompress → Convert →
//! GetData) happens inside a single `[UnmanagedCallersOnly]` compound
//! method so the thread stays in cooperative GC mode — managed object
//! pointers cannot be relocated between calls.

use std::collections::HashMap;
use std::sync::Mutex;

use xur_runtime::registry::ModuleBuilder;

use crate::stub::coreclr::ManagedObject;

struct CachedAtlas {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

static ATLAS_CACHE: Mutex<Option<HashMap<u64, CachedAtlas>>> = Mutex::new(None);

fn readback_direct(
    rt: &hao_runtime::Runtime,
    tex: usize,
) -> Result<(i64, i64, Vec<u8>), String> {
    let rid = hao_godot::engine::texture2d::get_rid(rt, tex)
        .map_err(|e| e.to_string())?;
    if rid == 0 {
        return Err("texture RID is 0".into());
    }
    let (w, h, data) = hao_godot::engine::image::extract_rgba_compound(rt, rid)
        .map_err(|e| e.to_string())?;
    Ok((w as i64, h as i64, data))
}

fn readback_atlas(
    rt: &hao_runtime::Runtime,
    tex: usize,
) -> Result<(i64, i64, Vec<u8>), String> {
    let region = hao_godot::engine::atlas_texture::get_region(rt, tex)
        .map_err(|e| e.to_string())?;
    let [rx, ry, rw, rh] = region;
    if rw <= 0.0 || rh <= 0.0 {
        return Err("not an atlas texture".into());
    }
    let rid = hao_godot::engine::texture2d::get_rid(rt, tex)
        .map_err(|e| e.to_string())?;
    if rid == 0 {
        return Err("atlas RID is 0".into());
    }
    let (aw, _ah, atlas_data) = get_or_readback_atlas(rt, rid)?;
    let data = crop_rgba(&atlas_data, aw, rx as u32, ry as u32, rw as u32, rh as u32);
    Ok((rw as i64, rh as i64, data))
}

fn get_or_readback_atlas(
    rt: &hao_runtime::Runtime,
    rid: u64,
) -> Result<(u32, u32, Vec<u8>), String> {
    {
        let mut cache = ATLAS_CACHE.lock().unwrap();
        let map = cache.get_or_insert_with(HashMap::new);
        if let Some(a) = map.get(&rid) {
            return Ok((a.width, a.height, a.data.clone()));
        }
    }

    let (w, h, data) = hao_godot::engine::image::extract_rgba_compound(rt, rid)
        .map_err(|e| e.to_string())?;

    let mut cache = ATLAS_CACHE.lock().unwrap();
    let map = cache.get_or_insert_with(HashMap::new);
    map.insert(
        rid,
        CachedAtlas {
            width: w,
            height: h,
            data: data.clone(),
        },
    );

    Ok((w, h, data))
}

fn crop_rgba(
    atlas: &[u8],
    atlas_w: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> Vec<u8> {
    let mut out = vec![0u8; (w * h * 4) as usize];
    for row in 0..h {
        let src_off = ((y + row) * atlas_w + x) as usize * 4;
        let dst_off = (row * w) as usize * 4;
        let len = w as usize * 4;
        if src_off + len <= atlas.len() {
            out[dst_off..dst_off + len].copy_from_slice(&atlas[src_off..src_off + len]);
        }
    }
    out
}

pub(super) fn register(m: &mut ModuleBuilder<'_>) {
    m.named_function(
        "texture_to_rgba",
        &["tex"],
        |tex: ManagedObject| -> Result<(i64, i64, Vec<u8>), String> {
            if tex.0 == 0 {
                return Err("null texture".into());
            }
            let rt = hao_godot::runtime().map_err(|e| e.to_string())?;
            readback_atlas(rt, tex.0).or_else(|_| readback_direct(rt, tex.0))
        },
    );

    m.named_function(
        "texture_to_rgba_direct",
        &["tex"],
        |tex: ManagedObject| -> Result<(i64, i64, Vec<u8>), String> {
            if tex.0 == 0 {
                return Err("null texture".into());
            }
            let rt = hao_godot::runtime().map_err(|e| e.to_string())?;
            readback_direct(rt, tex.0)
        },
    );

    m.named_function(
        "resolve_image_bindings",
        &[],
        || -> Result<(), String> {
            let rt = hao_godot::runtime().map_err(|e| e.to_string())?;
            hao_godot::engine::texture2d::resolve_all(rt).map_err(|e| e.to_string())?;
            hao_godot::engine::image::resolve_all(rt).map_err(|e| e.to_string())?;
            hao_godot::engine::rendering_server::resolve_all(rt).map_err(|e| e.to_string())?;
            hao_godot::engine::atlas_texture::resolve_all(rt).map_err(|e| e.to_string())?;
            tracing::info!("godot::image: all bindings resolved");
            Ok(())
        },
    );
}
