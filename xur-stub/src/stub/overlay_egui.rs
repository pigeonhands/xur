//! `stub::overlay::egui` — engine-agnostic egui overlay driven by Xur scripts.
//!
//! Scripts call `egui::init()` in `ctor()` to install GL/Vulkan hooks, then
//! `egui::run(draw)` in `stub_main()` which blocks forever, calling
//! `draw(ctx)` each frame.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use hao_egui_overlay::egui;
use hao_egui_overlay::egui::{Color32, Context, Frame, Grid, ScrollArea, SelectableLabel, Slider, Window};
use facet::Facet;
use xur_runtime::convert::into_value;
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{Registry, RuntimeError, Value};

// -----------------------------------------------------------------------
// Global bridge between render thread and script thread
// -----------------------------------------------------------------------

static BRIDGE: OnceLock<Arc<Bridge>> = OnceLock::new();

struct Bridge {
    mu: Mutex<BridgeState>,
    cv: Condvar,
    ready: AtomicBool,
    frames: AtomicU64,
}

struct BridgeState {
    ctx: Option<Context>,
    done: bool,
    shutdown: bool,
}

fn get_or_init_bridge() -> Arc<Bridge> {
    Arc::clone(BRIDGE.get_or_init(|| {
        Arc::new(Bridge {
            mu: Mutex::new(BridgeState {
                ctx: None,
                done: false,
                shutdown: false,
            }),
            cv: Condvar::new(),
            ready: AtomicBool::new(false),
            frames: AtomicU64::new(0),
        })
    }))
}

// -----------------------------------------------------------------------
// Opaque wrappers
// -----------------------------------------------------------------------

#[derive(Facet, Clone)]
#[facet(opaque)]
struct EguiCtx(usize);

unsafe impl Send for EguiCtx {}
unsafe impl Sync for EguiCtx {}

impl EguiCtx {
    fn from_ctx(ctx: &Context) -> Self {
        Self(ctx as *const Context as usize)
    }
    fn ctx(&self) -> &Context {
        unsafe { &*(self.0 as *const Context) }
    }
}

impl std::fmt::Display for EguiCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ctx")
    }
}

#[derive(Facet, Clone)]
#[facet(opaque)]
struct EguiUi(usize);

unsafe impl Send for EguiUi {}
unsafe impl Sync for EguiUi {}

impl EguiUi {
    fn from_ui(ui: &mut egui::Ui) -> Self {
        Self(ui as *mut egui::Ui as usize)
    }
    fn ui(&self) -> &mut egui::Ui {
        unsafe { &mut *(self.0 as *mut egui::Ui) }
    }
}

impl std::fmt::Display for EguiUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ui")
    }
}

// -----------------------------------------------------------------------
// Init — installs GL + Vulkan hooks and sets up the Bridge
// -----------------------------------------------------------------------

fn init_early() {
    let bridge = get_or_init_bridge();
    let render_bridge = Arc::clone(&bridge);

    hao_egui_overlay::on_frame(move |ctx| {
        render_bridge.frames.fetch_add(1, Ordering::Relaxed);

        if !render_bridge.ready.load(Ordering::Relaxed) {
            return;
        }

        // Post the Context to the script thread and wait for it to finish.
        {
            let mut state = render_bridge.mu.lock().unwrap();
            state.ctx = Some(ctx.clone());
            state.done = false;
            render_bridge.cv.notify_one();
        }
        {
            let mut state = render_bridge.mu.lock().unwrap();
            while !state.done {
                state = render_bridge.cv.wait(state).unwrap();
            }
        }
    });

    hao_egui_overlay::init();
}

// -----------------------------------------------------------------------
// Module registration
// -----------------------------------------------------------------------

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::stub::overlay::egui", |m| {
        let registry = m.registry();
        m.raw_function("init", |_cx, args| {
            if !args.is_empty() {
                return Err(RuntimeError::new("`init` takes no arguments"));
            }
            init_early();
            Ok(Value::Unit)
        });

        m.raw_function("set_scale", |_cx, args| {
            let [Value::Float(factor)] = args else {
                return Err(RuntimeError::new("`set_scale` takes a float"));
            };
            let f = *factor as f32;
            if f <= 0.0 {
                return Err(RuntimeError::new("scale factor must be positive"));
            }
            hao_egui_overlay::set_scale(f);
            Ok(Value::Unit)
        });

        register_run(m);
        register_frame_count(m);
        register_ctx_methods(&registry, m);
        register_ui_methods(&registry, m);
    });
}

/// `egui::run(fn(ctx))` — blocks forever, calling draw each frame.
fn register_run(m: &mut ModuleBuilder<'_>) {
    m.raw_function("run", |cx, args| {
        let [draw] = args else {
            return Err(RuntimeError::new("`run` takes a draw function"));
        };
        if !matches!(draw, Value::Closure(_) | Value::Fn(_)) {
            return Err(RuntimeError::new("`draw` must be a function or closure"));
        }

        let draw = draw.clone();

        let bridge = BRIDGE.get().ok_or_else(|| {
            RuntimeError::new("egui not initialised (call egui::init() in ctor)")
        })?;

        bridge.ready.store(true, Ordering::Relaxed);

        loop {
            let ctx = {
                let mut state = bridge.mu.lock().unwrap();
                while state.ctx.is_none() && !state.shutdown {
                    state = bridge.cv.wait(state).unwrap();
                }
                if state.shutdown {
                    return Ok(Value::Unit);
                }
                state.ctx.take().unwrap()
            };

            let ctx_val = into_value(cx, EguiCtx::from_ctx(&ctx)).expect("EguiCtx registered");
            if let Err(e) = cx.call(&draw, &[ctx_val]) {
                tracing::error!("draw error: {}", e.message);
                Window::new("xur error").show(&ctx, |ui| {
                    ui.colored_label(Color32::RED, format!("draw error: {}", e.message));
                });
            }

            {
                let mut state = bridge.mu.lock().unwrap();
                state.done = true;
                bridge.cv.notify_one();
            }
        }
    });
}

fn register_frame_count(m: &mut ModuleBuilder<'_>) {
    m.raw_function("frame_count", |_cx, args| {
        if !args.is_empty() {
            return Err(RuntimeError::new("`frame_count` takes no arguments"));
        }
        let bridge = BRIDGE.get();
        let count = bridge.map_or(0, |b| b.frames.load(Ordering::Relaxed));
        Ok(Value::Int(count as i64))
    });
}

// -----------------------------------------------------------------------
// Ctx methods
// -----------------------------------------------------------------------

fn register_ctx_methods(registry: &Registry, m: &mut ModuleBuilder<'_>) {
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
            let frame = Frame::window(&ctx.style()).fill(Color32::from_rgb(24, 24, 28));
            let mut ret = Value::Unit;
            Window::new(title.as_ref())
                .frame(frame)
                .default_width(400.0)
                .show(ctx, |ui| {
                    let cx = unsafe { &mut *cx_ptr };
                    let ui_val = into_value(cx, EguiUi::from_ui(ui)).expect("EguiUi registered");
                    match cx.call(&body, &[ui_val]) {
                        Ok(val) => { ret = val; }
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
            let frame = Frame::window(&ctx.style()).fill(Color32::from_rgb(24, 24, 28));
            let mut ret = Value::Unit;
            Window::new(title.as_ref())
                .frame(frame)
                .default_width(w)
                .max_width(w)
                .show(ctx, |ui| {
                    let cx = unsafe { &mut *cx_ptr };
                    let ui_val = into_value(cx, EguiUi::from_ui(ui)).expect("EguiUi registered");
                    match cx.call(&body, &[ui_val]) {
                        Ok(val) => { ret = val; }
                        Err(e) => {
                            tracing::error!("{}", e.message);
                            ui.colored_label(Color32::RED, format!("error: {}", e.message));
                        }
                    }
                });
            Ok(ret)
        })
    });

    ty.raw_function("key", |_cx, args| {
        let [this, Value::Str(name)] = args else {
            return Err(RuntimeError::new("`key` takes a key name string"));
        };
        let _ = with_ctx(this, |_ctx| Ok(Value::Unit))?;
        with_ctx(this, |ctx| {
            let pressed = ctx.input(|i| {
                match name.to_ascii_lowercase().as_str() {
                    "f1" => i.key_down(egui::Key::F1),
                    "f2" => i.key_down(egui::Key::F2),
                    "f3" => i.key_down(egui::Key::F3),
                    "f4" => i.key_down(egui::Key::F4),
                    "f5" => i.key_down(egui::Key::F5),
                    "escape" | "esc" => i.key_down(egui::Key::Escape),
                    "space" => i.key_down(egui::Key::Space),
                    "tab" => i.key_down(egui::Key::Tab),
                    "return" | "enter" => i.key_down(egui::Key::Enter),
                    _ => false,
                }
            });
            Ok(Value::Bool(pressed))
        })
    });

    let _ = registry;
}

fn with_ctx(
    val: &Value,
    f: impl FnOnce(&Context) -> Result<Value, RuntimeError>,
) -> Result<Value, RuntimeError> {
    let Value::Native(obj) = val else {
        return Err(RuntimeError::new("expected a `Ctx`"));
    };
    obj.with(|wrapper: &EguiCtx| f(wrapper.ctx()))
        .ok_or_else(|| RuntimeError::new("expected a `Ctx`"))?
}

// -----------------------------------------------------------------------
// Ui methods
// -----------------------------------------------------------------------

fn register_ui_methods(registry: &Registry, m: &mut ModuleBuilder<'_>) {
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

    ty.raw_function("small", |_cx, args| {
        let [this, Value::Str(text)] = args else {
            return Err(RuntimeError::new("`small` takes a string"));
        };
        with_ui(this, |ui| {
            ui.small(text.as_ref());
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
                    Err(e) => { let _ = e; }
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
                    Err(e) => { let _ = e; }
                }
            });
            Ok(ret)
        })
    });

    ty.raw_function("scroll", |cx, args| {
        let [this, Value::Float(max_height), body] = args else {
            return Err(RuntimeError::new(
                "`scroll` takes (max_height: float, body: fn)",
            ));
        };
        if !matches!(body, Value::Closure(_) | Value::Fn(_)) {
            return Err(RuntimeError::new("body must be a function"));
        }
        let max_height = *max_height as f32;
        let body = body.clone();
        let cx_ptr = cx as *mut dyn xur_runtime::registry::Context;
        with_ui(this, |ui| {
            let mut ret = Value::Unit;
            ScrollArea::vertical().max_height(max_height).show(ui, |inner_ui| {
                let cx = unsafe { &mut *cx_ptr };
                let inner = into_value(cx, EguiUi::from_ui(inner_ui)).expect("EguiUi registered");
                match cx.call(&body, &[inner]) {
                    Ok(val) => ret = val,
                    Err(e) => { let _ = e; }
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
                        Err(e) => { let _ = e; }
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
                ui.add(SelectableLabel::new(*selected, text.as_ref()))
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

    let _ = registry;
}

fn with_ui(
    val: &Value,
    f: impl FnOnce(&mut egui::Ui) -> Result<Value, RuntimeError>,
) -> Result<Value, RuntimeError> {
    let Value::Native(obj) = val else {
        return Err(RuntimeError::new("expected a `Ui`"));
    };
    obj.with(|wrapper: &EguiUi| f(wrapper.ui()))
        .ok_or_else(|| RuntimeError::new("expected a `Ui`"))?
}
