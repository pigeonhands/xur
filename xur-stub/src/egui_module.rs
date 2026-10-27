//! `std::re::unity::egui` — draw an egui overlay from a Xur script.
//!
//! The script's `ctor()` calls `egui::init()` to install the Il2Cpp
//! hooks before the game starts. Then `main()` calls `egui::run(title,
//! draw)` which blocks forever, drawing each frame.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use facet::Facet;
use hao_il2cpp_unity::egui as hao_egui;
use hao_il2cpp_unity::egui::{Color32, Context, Frame, Slider, Window};
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
    visible: AtomicBool,
    f1_down: AtomicBool,
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
            visible: AtomicBool::new(true),
            f1_down: AtomicBool::new(false),
        })
    }))
}

// -----------------------------------------------------------------------
// Opaque Ui wrapper
// -----------------------------------------------------------------------

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
// Init (called from stub ctor, on the main thread)
// -----------------------------------------------------------------------

/// Installs the Il2Cpp dlopen/il2cpp_init hooks and registers the egui
/// render callback. Called from the script's `ctor()` via `egui::init()`.
fn init_early() {
    let bridge = get_or_init_bridge();
    let render_bridge = Arc::clone(&bridge);

    hao_il2cpp_unity::on_init(move |_il2cpp| {
        hao_egui::on_egui(move |ctx| {
            let f1 = input::key(KeyCode::F1).unwrap_or(false);
            let was_down = render_bridge.f1_down.swap(f1, Ordering::Relaxed);
            if f1 && !was_down {
                render_bridge.visible.fetch_xor(true, Ordering::Relaxed);
            }
            if !render_bridge.visible.load(Ordering::Relaxed) {
                return;
            }

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
    register_ui_methods(registry, m);
}

/// `egui::run(title, fn(ui))` — blocks forever, calling draw each frame.
fn register_run(m: &mut ModuleBuilder<'_>) {
    m.raw_function("run", |cx, args| {
        let [Value::Str(title), draw] = args else {
            return Err(RuntimeError::new("`run` takes (title: str, draw: fn)"));
        };
        if !matches!(draw, Value::Closure(_) | Value::Fn(_)) {
            return Err(RuntimeError::new("`draw` must be a function or closure"));
        }

        let title = title.to_string();
        let draw = draw.clone();

        let bridge = BRIDGE.get().ok_or_else(|| {
            RuntimeError::new("egui not initialised (stub must be built with --features egui)")
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

            let frames = bridge.frames.load(Ordering::Relaxed);
            let frame = Frame::window(&ctx.style()).fill(Color32::from_rgb(24, 24, 28));
            Window::new(&title)
                .frame(frame)
                .default_pos([24.0, 24.0])
                .default_width(400.0)
                .show(&ctx, |ui| {
                    ui.label(format!("F1 to toggle — frame {frames}"));
                    ui.separator();
                    let ui_val = into_value(cx, EguiUi::from_ui(ui)).expect("EguiUi registered");
                    if let Err(e) = cx.call(&draw, vec![ui_val]) {
                        ui.colored_label(Color32::RED, format!("draw error: {}", e.message));
                    }
                });

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
