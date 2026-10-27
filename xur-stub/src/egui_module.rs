//! `stub::re::unity::egui` — egui overlay driven by Xur scripts.
//!
//! The script's `ctor()` calls `egui::init()` to install the Il2Cpp
//! hooks. Then `main()` calls `egui::run(draw)` which blocks forever,
//! calling `draw(ctx)` each frame. The script uses `ctx.window(...)` to
//! create its own windows.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use facet::Facet;
use hao_il2cpp_unity::egui as hao_egui;
use hao_il2cpp_unity::egui::{Color32, Context, Frame, ScrollArea, Slider, Window};
use hao_il2cpp_unity::{input, KeyCode};
use xur_runtime::convert::into_value;
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{Registry, RuntimeError, Value};

// -----------------------------------------------------------------------
// Global bridge
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
}

fn get_or_init_bridge() -> Arc<Bridge> {
    Arc::clone(BRIDGE.get_or_init(|| {
        Arc::new(Bridge {
            mu: Mutex::new(BridgeState {
                ctx: None,
                done: false,
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
    fn from_ui(ui: &mut hao_egui::Ui) -> Self {
        Self(ui as *mut hao_egui::Ui as usize)
    }

    fn ui(&self) -> &mut hao_egui::Ui {
        unsafe { &mut *(self.0 as *mut hao_egui::Ui) }
    }
}

impl std::fmt::Display for EguiUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ui")
    }
}

// -----------------------------------------------------------------------
// Init
// -----------------------------------------------------------------------

fn init_early() {
    let bridge = get_or_init_bridge();
    let render_bridge = Arc::clone(&bridge);

    hao_il2cpp_unity::on_init(move |_il2cpp| {
        hao_egui::on_egui(move |ctx| {
            render_bridge.frames.fetch_add(1, Ordering::Relaxed);

            if !render_bridge.ready.load(Ordering::Relaxed) {
                return;
            }

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
        })?;
        Ok(())
    });
}

// -----------------------------------------------------------------------
// Module registration
// -----------------------------------------------------------------------

pub(crate) fn register(registry: &Registry, m: &mut ModuleBuilder<'_>) {
    m.raw_function("init", |_cx, args| {
        if !args.is_empty() {
            return Err(RuntimeError::new("`init` takes no arguments"));
        }
        init_early();
        Ok(Value::Unit)
    });

    register_run(m);
    register_frame_count(m);
    register_ctx_methods(registry, m);
    register_ui_methods(registry, m);
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
                while state.ctx.is_none() {
                    state = bridge.cv.wait(state).unwrap();
                }
                state.ctx.take().unwrap()
            };

            let ctx_val = into_value(cx, EguiCtx::from_ctx(&ctx)).expect("EguiCtx registered");
            if let Err(e) = cx.call(&draw, vec![ctx_val]) {
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
            Window::new(title.as_ref())
                .frame(frame)
                .default_width(400.0)
                .show(ctx, |ui| {
                    let cx = unsafe { &mut *cx_ptr };
                    let ui_val = into_value(cx, EguiUi::from_ui(ui)).expect("EguiUi registered");
                    if let Err(e) = cx.call(&body, vec![ui_val]) {
                        ui.colored_label(Color32::RED, format!("error: {}", e.message));
                    }
                });
            Ok(Value::Unit)
        })
    });

    ty.raw_function("key", |_cx, args| {
        let [this, Value::Str(name)] = args else {
            return Err(RuntimeError::new("`key` takes a key name string"));
        };
        let _ = with_ctx(this, |_ctx| Ok(Value::Unit))?;
        let code = parse_keycode(name)
            .ok_or_else(|| RuntimeError::new(format!("unknown key: {name}")))?;
        Ok(Value::Bool(input::key(code).unwrap_or(false)))
    });

    let _ = registry;
}

fn parse_keycode(name: &str) -> Option<KeyCode> {
    match name.to_ascii_lowercase().as_str() {
        "f1" => Some(KeyCode::F1),
        "f2" => Some(KeyCode::F2),
        "f3" => Some(KeyCode::F3),
        "f4" => Some(KeyCode::F4),
        "f5" => Some(KeyCode::F5),
        "f6" => Some(KeyCode::F6),
        "escape" | "esc" => Some(KeyCode::ESCAPE),
        "space" => Some(KeyCode::SPACE),
        "tab" => Some(KeyCode::TAB),
        "return" | "enter" => Some(KeyCode::RETURN),
        "backspace" => Some(KeyCode::BACKSPACE),
        "insert" | "ins" => Some(KeyCode::INSERT),
        "delete" | "del" => Some(KeyCode::DELETE),
        "home" => Some(KeyCode::HOME),
        "end" => Some(KeyCode::END),
        "left" => Some(KeyCode::LEFT),
        "right" => Some(KeyCode::RIGHT),
        "up" => Some(KeyCode::UP),
        "down" => Some(KeyCode::DOWN),
        "lshift" => Some(KeyCode::LEFT_SHIFT),
        "rshift" => Some(KeyCode::RIGHT_SHIFT),
        "lctrl" => Some(KeyCode::LEFT_CONTROL),
        "rctrl" => Some(KeyCode::RIGHT_CONTROL),
        "lalt" => Some(KeyCode::LEFT_ALT),
        "ralt" => Some(KeyCode::RIGHT_ALT),
        _ => None,
    }
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
            ui.horizontal(|inner_ui| {
                let cx = unsafe { &mut *cx_ptr };
                let inner = into_value(cx, EguiUi::from_ui(inner_ui)).expect("EguiUi registered");
                let _ = cx.call(&body, vec![inner]);
            });
            Ok(Value::Unit)
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
            ui.collapsing(heading.as_ref(), |inner_ui| {
                let cx = unsafe { &mut *cx_ptr };
                let inner = into_value(cx, EguiUi::from_ui(inner_ui)).expect("EguiUi registered");
                let _ = cx.call(&body, vec![inner]);
            });
            Ok(Value::Unit)
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
            ScrollArea::vertical().max_height(max_height).show(ui, |inner_ui| {
                let cx = unsafe { &mut *cx_ptr };
                let inner = into_value(cx, EguiUi::from_ui(inner_ui)).expect("EguiUi registered");
                let _ = cx.call(&body, vec![inner]);
            });
            Ok(Value::Unit)
        })
    });

    let _ = registry;
}

fn with_ui(
    val: &Value,
    f: impl FnOnce(&mut hao_egui::Ui) -> Result<Value, RuntimeError>,
) -> Result<Value, RuntimeError> {
    let Value::Native(obj) = val else {
        return Err(RuntimeError::new("expected a `Ui`"));
    };
    obj.with(|wrapper: &EguiUi| f(wrapper.ui()))
        .ok_or_else(|| RuntimeError::new("expected a `Ui`"))?
}
