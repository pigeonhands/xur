use std::collections::HashMap;
use std::sync::Mutex;

use std::sync::Arc;

use egui::{Color32, ColorImage, Frame, Grid, Image, ImageData, ScrollArea, Slider, TextureHandle, TextureId, TextureOptions, Vec2, Window};
use xur_runtime::convert::into_value;
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{Registry, RuntimeError, Value};

use super::types::{EguiCtx, EguiUi, Texture, with_ctx, with_ui};

static TEXTURES: Mutex<Option<HashMap<u64, TextureHandle>>> = Mutex::new(None);

pub(crate) fn register_ctx_methods(_registry: &Registry, m: &mut ModuleBuilder<'_>) {
    let mut ty = m.ty::<EguiCtx>("Ctx");

    ty.raw_function("window", |cx, args| {
        let [this, Value::Str(title), body] = args else {
            return Err(RuntimeError::new(
                "`window` takes (title: str, body: fn(ui))",
            ));
        };
        if !matches!(body, Value::Closure(_) | Value::Fn(_)) {
            return Err(RuntimeError::new("body must be a function"));
        }
        let body = body.clone();
        let title = title.clone();
        let cx_ptr = cx as *mut dyn xur_runtime::registry::Context;
        with_ctx(this, |ctx| {
            let frame = Frame::window(&ctx.style_of(ctx.theme())).fill(Color32::from_rgb(24, 24, 28));
            let mut ret = Value::Unit;
            Window::new(title.as_ref())
                .frame(frame)
                .default_width(400.0)
                .show(ctx, |ui| {
                    let cx = unsafe { &mut *cx_ptr };
                    let ui_val = into_value(cx, EguiUi::from_ui(ui)).expect("EguiUi registered");
                    match cx.call(&body, &[ui_val]) {
                        Ok(val) => ret = val,
                        Err(e) => {
                            tracing::error!("{}", e.message);
                            ui.colored_label(Color32::RED, format!("error: {}", e.message));
                        }
                    }
                });
            Ok(ret)
        })
    });

    ty.raw_function("window_sized", |cx, args| {
        let [this, Value::Str(title), Value::Float(width), body] = args else {
            return Err(RuntimeError::new(
                "`window_sized` takes (title: str, width: float, body: fn(ui))",
            ));
        };
        if !matches!(body, Value::Closure(_) | Value::Fn(_)) {
            return Err(RuntimeError::new("body must be a function"));
        }
        let body = body.clone();
        let title = title.clone();
        let w = *width as f32;
        let cx_ptr = cx as *mut dyn xur_runtime::registry::Context;
        with_ctx(this, |ctx| {
            let frame = Frame::window(&ctx.style_of(ctx.theme())).fill(Color32::from_rgb(24, 24, 28));
            let mut ret = Value::Unit;
            Window::new(title.as_ref())
                .frame(frame)
                .default_width(w)
                .max_width(w)
                .show(ctx, |ui| {
                    let cx = unsafe { &mut *cx_ptr };
                    let ui_val = into_value(cx, EguiUi::from_ui(ui)).expect("EguiUi registered");
                    match cx.call(&body, &[ui_val]) {
                        Ok(val) => ret = val,
                        Err(e) => {
                            tracing::error!("{}", e.message);
                            ui.colored_label(Color32::RED, format!("error: {}", e.message));
                        }
                    }
                });
            Ok(ret)
        })
    });

    ty.method(
        "load_texture_rgba",
        |wrapper: &EguiCtx, name: String, width: i64, height: i64, rgba: Vec<u8>| -> Result<Texture, String> {
            let w = width as usize;
            let h = height as usize;
            if rgba.len() != w * h * 4 {
                return Err(format!(
                    "expected {} bytes ({}x{}x4), got {}",
                    w * h * 4, w, h, rgba.len()
                ));
            }
            let image = ColorImage::from_rgba_unmultiplied([w, h], &rgba);
            let ctx = wrapper.ctx();
            let tex_mngr = ctx.tex_manager();
            let tex_id = tex_mngr.write().alloc(
                name,
                ImageData::Color(Arc::new(image)),
                TextureOptions::LINEAR,
            );
            let TextureId::Managed(id) = tex_id else {
                return Err("unexpected texture id type".into());
            };
            let handle = TextureHandle::new(tex_mngr, tex_id);
            let mut map = TEXTURES.lock().unwrap();
            map.get_or_insert_with(HashMap::new).insert(id, handle);
            Ok(Texture::Managed(id as i64))
        },
    );

    ty.method("free_texture", |_wrapper: &EguiCtx, tex: Texture| {
        if let Texture::Managed(id) = tex {
            let mut map = TEXTURES.lock().unwrap();
            if let Some(m) = map.as_mut() {
                m.remove(&(id as u64));
            }
        }
    });
}

pub(crate) fn register_ui_methods(_registry: &Registry, m: &mut ModuleBuilder<'_>) {
    let mut ty = m.ty::<EguiUi>("Ui");

    ty.raw_function("label", |_cx, args| {
        let [this, Value::Str(text)] = args else {
            return Err(RuntimeError::new("`label` takes a string"));
        };
        with_ui(this, |ui| {
            ui.label(text.as_ref());
            Ok(Value::Unit)
        })
    });

    ty.raw_function("heading", |_cx, args| {
        let [this, Value::Str(text)] = args else {
            return Err(RuntimeError::new("`heading` takes a string"));
        };
        with_ui(this, |ui| {
            ui.heading(text.as_ref());
            Ok(Value::Unit)
        })
    });

    ty.raw_function("small", |_cx, args| {
        let [this, Value::Str(text)] = args else {
            return Err(RuntimeError::new("`small` takes a string"));
        };
        with_ui(this, |ui| {
            ui.small(text.as_ref());
            Ok(Value::Unit)
        })
    });

    ty.raw_function("button", |_cx, args| {
        let [this, Value::Str(text)] = args else {
            return Err(RuntimeError::new("`button` takes a string"));
        };
        with_ui(this, |ui| Ok(Value::Bool(ui.button(text.as_ref()).clicked())))
    });

    ty.raw_function("separator", |_cx, args| {
        let [this] = args else {
            return Err(RuntimeError::new("`separator` takes no arguments"));
        };
        with_ui(this, |ui| {
            ui.separator();
            Ok(Value::Unit)
        })
    });

    ty.raw_function("checkbox", |_cx, args| {
        let [this, Value::Bool(checked), Value::Str(label)] = args else {
            return Err(RuntimeError::new(
                "`checkbox` takes (checked: bool, label: str)",
            ));
        };
        with_ui(this, |ui| {
            let mut val = *checked;
            ui.checkbox(&mut val, label.as_ref());
            Ok(Value::Bool(val))
        })
    });

    ty.raw_function("slider_float", |_cx, args| {
        let [this, Value::Str(label), Value::Float(value), Value::Float(min), Value::Float(max)] =
            args
        else {
            return Err(RuntimeError::new(
                "`slider_float` takes (label, value, min, max)",
            ));
        };
        with_ui(this, |ui| {
            let mut val = *value as f32;
            ui.add(Slider::new(&mut val, (*min as f32)..=(*max as f32)).text(label.as_ref()));
            Ok(Value::Float(val as f64))
        })
    });

    ty.raw_function("slider_int", |_cx, args| {
        let [this, Value::Str(label), Value::Int(value), Value::Int(min), Value::Int(max)] = args
        else {
            return Err(RuntimeError::new(
                "`slider_int` takes (label, value, min, max)",
            ));
        };
        with_ui(this, |ui| {
            let mut val = *value;
            ui.add(Slider::new(&mut val, *min..=*max).text(label.as_ref()));
            Ok(Value::Int(val))
        })
    });

    ty.raw_function("text_edit", |_cx, args| {
        let [this, Value::Str(text)] = args else {
            return Err(RuntimeError::new("`text_edit` takes a string"));
        };
        with_ui(this, |ui| {
            let mut buf = text.to_string();
            ui.text_edit_singleline(&mut buf);
            Ok(Value::Str(buf.into()))
        })
    });

    ty.raw_function("text_edit_multiline", |_cx, args| {
        let [this, Value::Str(text)] = args else {
            return Err(RuntimeError::new("`text_edit_multiline` takes a string"));
        };
        with_ui(this, |ui| {
            let mut buf = text.to_string();
            ui.text_edit_multiline(&mut buf);
            Ok(Value::Str(buf.into()))
        })
    });

    ty.raw_function("horizontal", |cx, args| {
        let [this, body] = args else {
            return Err(RuntimeError::new("`horizontal` takes a function"));
        };
        if !matches!(body, Value::Closure(_) | Value::Fn(_)) {
            return Err(RuntimeError::new("body must be a function"));
        }
        let body = body.clone();
        let cx_ptr = cx as *mut dyn xur_runtime::registry::Context;
        with_ui(this, |ui| {
            let mut ret = Value::Unit;
            ui.horizontal(|inner_ui| {
                let cx = unsafe { &mut *cx_ptr };
                let inner = into_value(cx, EguiUi::from_ui(inner_ui)).expect("EguiUi registered");
                match cx.call(&body, &[inner]) {
                    Ok(val) => ret = val,
                    Err(e) => { tracing::error!("gui callback error: {}", e.message); }
                }
            });
            Ok(ret)
        })
    });

    ty.raw_function("vertical", |cx, args| {
        let [this, body] = args else {
            return Err(RuntimeError::new("`vertical` takes a function"));
        };
        if !matches!(body, Value::Closure(_) | Value::Fn(_)) {
            return Err(RuntimeError::new("body must be a function"));
        }
        let body = body.clone();
        let cx_ptr = cx as *mut dyn xur_runtime::registry::Context;
        with_ui(this, |ui| {
            let mut ret = Value::Unit;
            ui.vertical(|inner_ui| {
                let cx = unsafe { &mut *cx_ptr };
                let inner = into_value(cx, EguiUi::from_ui(inner_ui)).expect("EguiUi registered");
                match cx.call(&body, &[inner]) {
                    Ok(val) => ret = val,
                    Err(e) => { tracing::error!("gui callback error: {}", e.message); }
                }
            });
            Ok(ret)
        })
    });

    ty.raw_function("collapsing", |cx, args| {
        let [this, Value::Str(heading), body] = args else {
            return Err(RuntimeError::new(
                "`collapsing` takes (heading: str, body: fn)",
            ));
        };
        if !matches!(body, Value::Closure(_) | Value::Fn(_)) {
            return Err(RuntimeError::new("body must be a function"));
        }
        let body = body.clone();
        let heading = heading.clone();
        let cx_ptr = cx as *mut dyn xur_runtime::registry::Context;
        with_ui(this, |ui| {
            let mut ret = Value::Unit;
            ui.collapsing(heading.as_ref(), |inner_ui| {
                let cx = unsafe { &mut *cx_ptr };
                let inner = into_value(cx, EguiUi::from_ui(inner_ui)).expect("EguiUi registered");
                match cx.call(&body, &[inner]) {
                    Ok(val) => ret = val,
                    Err(e) => { tracing::error!("gui callback error: {}", e.message); }
                }
            });
            Ok(ret)
        })
    });

    ty.raw_function("scroll", |cx, args| {
        let cx_ptr = cx as *mut dyn xur_runtime::registry::Context;
        let (this_val, id_salt, max_height, body_fn) = match args {
            [this_val, Value::Str(id), Value::Float(h), b] if matches!(b, Value::Closure(_) | Value::Fn(_)) => {
                (this_val, Some(id.clone()), Some(*h as f32), b.clone())
            }
            [this_val, Value::Str(id), b] if matches!(b, Value::Closure(_) | Value::Fn(_)) => {
                (this_val, Some(id.clone()), None, b.clone())
            }
            [this_val, Value::Float(h), b] if matches!(b, Value::Closure(_) | Value::Fn(_)) => {
                (this_val, None, Some(*h as f32), b.clone())
            }
            [this_val, b] if matches!(b, Value::Closure(_) | Value::Fn(_)) => {
                (this_val, None, None, b.clone())
            }
            _ => {
                return Err(RuntimeError::new(
                    "`scroll` takes fn(ui), (max_height, fn(ui)), (id, fn(ui)), or (id, max_height, fn(ui))"
                ));
            }
        };
        with_ui(this_val, |ui| {
            let mut scroll = ScrollArea::vertical().auto_shrink([false, true]);
            if let Some(id) = &id_salt {
                scroll = scroll.id_salt(&**id);
            }
            if let Some(h) = max_height {
                scroll = scroll.max_height(h);
            }
            let mut ret = Value::Unit;
            scroll.show(ui, |inner_ui| {
                let cx = unsafe { &mut *cx_ptr };
                let inner = into_value(cx, EguiUi::from_ui(inner_ui)).expect("EguiUi registered");
                match cx.call(&body_fn, &[inner]) {
                    Ok(val) => ret = val,
                    Err(e) => { tracing::error!("gui callback error: {}", e.message); }
                }
            });
            Ok(ret)
        })
    });

    ty.raw_function("grid", |cx, args| {
        let [this, Value::Str(id), Value::Int(cols), body] = args else {
            return Err(RuntimeError::new(
                "`grid` takes (id: str, columns: int, body: fn(ui))",
            ));
        };
        if !matches!(body, Value::Closure(_) | Value::Fn(_)) {
            return Err(RuntimeError::new("body must be a function"));
        }
        let id = id.clone();
        let cols = *cols as usize;
        let body = body.clone();
        let cx_ptr = cx as *mut dyn xur_runtime::registry::Context;
        with_ui(this, |ui| {
            let mut ret = Value::Unit;
            Grid::new(id.as_ref())
                .num_columns(cols)
                .striped(true)
                .show(ui, |grid_ui| {
                    let cx = unsafe { &mut *cx_ptr };
                    let inner =
                        into_value(cx, EguiUi::from_ui(grid_ui)).expect("EguiUi registered");
                    match cx.call(&body, &[inner]) {
                        Ok(val) => ret = val,
                        Err(e) => { tracing::error!("gui callback error: {}", e.message); }
                    }
                });
            Ok(ret)
        })
    });

    ty.raw_function("end_row", |_cx, args| {
        let [this] = args else {
            return Err(RuntimeError::new("`end_row` takes no arguments"));
        };
        with_ui(this, |ui| {
            ui.end_row();
            Ok(Value::Unit)
        })
    });

    ty.raw_function("selectable", |_cx, args| {
        let [this, Value::Bool(selected), Value::Str(text)] = args else {
            return Err(RuntimeError::new(
                "`selectable` takes (selected: bool, text: str)",
            ));
        };
        with_ui(this, |ui| {
            Ok(Value::Bool(
                ui.selectable_label(*selected, text.as_ref())
                    .clicked(),
            ))
        })
    });

    ty.raw_function("colored_label", |_cx, args| {
        let [this, Value::Int(r), Value::Int(g), Value::Int(b), Value::Str(text)] = args else {
            return Err(RuntimeError::new(
                "`colored_label` takes (r, g, b, text)",
            ));
        };
        with_ui(this, |ui| {
            ui.colored_label(
                Color32::from_rgb(*r as u8, *g as u8, *b as u8),
                text.as_ref(),
            );
            Ok(Value::Unit)
        })
    });

    ty.raw_function("add_space", |_cx, args| {
        let [this, Value::Float(amount)] = args else {
            return Err(RuntimeError::new("`add_space` takes a float"));
        };
        with_ui(this, |ui| {
            ui.add_space(*amount as f32);
            Ok(Value::Unit)
        })
    });

    ty.method("image", |wrapper: &EguiUi, tex: Texture, width: f64, height: f64| {
        let ui = wrapper.ui();
        let sized = egui::load::SizedTexture::new(
            tex.texture_id(),
            Vec2::new(width as f32, height as f32),
        );
        ui.add(Image::new(sized));
    });
}
