use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use facet::Facet;
use hao_egui::Context;
use hao_il2cpp_unity::egui as hao_egui;
use hao_il2cpp_unity::egui::TextureId;
use hao_il2cpp_unity::{input, KeyCode};
use xur_runtime::convert::into_value;
use xur_runtime::{Registry, RuntimeError, Value};

use xur_std::gui::{EguiCtx, Texture};

use crate::stub::il2cpp::XurUnityObject;

#[derive(Facet)]
#[repr(C)]
#[allow(dead_code)]
enum EguiError {
    InvalidScale,
    NotInitialized,
    UnknownKey(String),
}

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

impl Bridge {
    fn new() -> Self {
        Self {
            mu: Mutex::new(BridgeState {
                ctx: None,
                done: false,
                shutdown: false,
            }),
            cv: Condvar::new(),
            ready: AtomicBool::new(false),
            frames: AtomicU64::new(0),
        }
    }
}

static BRIDGE: OnceLock<Arc<Bridge>> = OnceLock::new();

fn get_or_init_bridge() -> Arc<Bridge> {
    Arc::clone(BRIDGE.get_or_init(|| Arc::new(Bridge::new())))
}

fn init_early() {
    let bridge = get_or_init_bridge();
    let render_bridge = Arc::clone(&bridge);

    let quit_bridge = Arc::clone(&bridge);
    hao_il2cpp_unity::on_init(move |il2cpp| {
        let quit_bridge = Arc::clone(&quit_bridge);
        let quit_result = il2cpp
            .class("UnityEngine.CoreModule", "UnityEngine", "Application")
            .and_then(|app| app.method("Quit", 1));
        if let Ok(quit) = quit_result {
            let _ = quit.hook(
                move |exit_code: i32, original: hao_il2cpp_runtime::Original<_, ()>| {
                    hao_il2cpp_unity::shutdown();
                    {
                        let mut state = quit_bridge.mu.lock().unwrap();
                        state.shutdown = true;
                        quit_bridge.cv.notify_all();
                    }
                    original.call((exit_code,));
                },
            );
        }

        let wake_bridge = Arc::clone(&render_bridge);
        hao_il2cpp_unity::dispatch::set_waker(move || {
            wake_bridge.cv.notify_all();
        });

        hao_egui::on_egui(move |ctx| {
            if hao_il2cpp_unity::is_shutting_down() {
                let mut state = render_bridge.mu.lock().unwrap();
                if !state.shutdown {
                    state.shutdown = true;
                    render_bridge.cv.notify_all();
                }
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
                    if !state.done {
                        drop(state);
                        hao_il2cpp_unity::dispatch::drain_pending();
                        state = render_bridge.mu.lock().unwrap();
                    }
                }
            }
        })?;
        Ok(())
    });
}

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::stub::unity::egui", |m| {
        m.shape_type("EguiError", EguiError::SHAPE);

        m.function("init", || {
            init_early();
        });

        m.named_function("set_scale", &["factor"], |factor: f64| -> Result<(), EguiError> {
            let f = factor as f32;
            if f <= 0.0 {
                return Err(EguiError::InvalidScale);
            }
            hao_egui::set_ui_scale(f);
            Ok(())
        });

        m.named_function(
            "register_texture",
            &["obj"],
            |obj: XurUnityObject| -> Result<Texture, String> {
                let tex_id = hao_egui::register_user_texture(obj.obj())
                    .map_err(|e| format!("register_texture: {e}"))?;
                match tex_id {
                    TextureId::User(id) => Ok(Texture::User(id as i64)),
                    _ => Err("register_texture: unexpected texture id".into()),
                }
            },
        );

        m.named_function("unregister_texture", &["tex"], |tex: Texture| {
            if let Texture::User(id) = tex {
                hao_egui::unregister_user_texture(TextureId::User(id as u64));
            }
        });

        register_run(m);

        m.function("frame_count", || -> i64 {
            BRIDGE.get().map_or(0, |b| b.frames.load(Ordering::Relaxed)) as i64
        });

        let mut ty = m.ty_existing::<EguiCtx>();
        ty.method("key", |_ctx: &EguiCtx, name: String| -> Result<bool, EguiError> {
            let code = parse_keycode(&name)
                .ok_or_else(|| EguiError::UnknownKey(name))?;
            Ok(input::key(code).unwrap_or(false))
        });
    });
}

fn register_run(m: &mut xur_runtime::registry::ModuleBuilder<'_>) {
    m.raw_function("run", move |cx, args| {
        let [draw] = args else {
            return Err(RuntimeError::new("`run` takes a draw function"));
        };
        if !matches!(draw, Value::Closure(_) | Value::Fn(_)) {
            return Err(RuntimeError::new("`draw` must be a function or closure"));
        }

        let draw = draw.clone();
        let bridge = BRIDGE
            .get()
            .ok_or_else(|| RuntimeError::new("egui not initialised (call egui::init() in ctor)"))?;

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
                hao_egui::Window::new("xur error").show(&ctx, |ui| {
                    ui.colored_label(hao_egui::Color32::RED, format!("draw error: {}", e.message));
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
