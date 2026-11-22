use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use hao_il2cpp_unity::egui as hao_egui;
use hao_egui::Context;
use hao_il2cpp_unity::egui::{Image, TextureId, Vec2};
use hao_il2cpp_unity::{input, KeyCode};
use xur_runtime::convert::into_value;
use xur_runtime::{Registry, RuntimeError, Value};

use xur_std::gui::{EguiCtx, EguiUi, with_ctx, with_ui};

use crate::stub::il2cpp::XurObject;

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
            let _ = quit.hook(move |exit_code: i32, original: hao_il2cpp_runtime::Original<_, ()>| {
                hao_il2cpp_unity::shutdown();
                {
                    let mut state = quit_bridge.mu.lock().unwrap();
                    state.shutdown = true;
                    quit_bridge.cv.notify_all();
                }
                original.call((exit_code,));
            });
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
            hao_egui::set_ui_scale(f);
            Ok(Value::Unit)
        });

        m.raw_function("register_texture", |_cx, args| {
            let [obj_val] = args else {
                return Err(RuntimeError::new("`register_texture` takes a Unity Object"));
            };
            let Value::Native(n) = obj_val else {
                return Err(RuntimeError::new("`register_texture` takes a Unity Object"));
            };
            let obj = n.with(|o: &XurObject| o.obj())
                .ok_or_else(|| RuntimeError::new("expected an il2cpp Object"))?;
            let tex_id = hao_egui::register_user_texture(obj)
                .map_err(|e| RuntimeError::new(format!("register_texture: {e}")))?;
            match tex_id {
                TextureId::User(id) => Ok(Value::Int(id as i64)),
                _ => unreachable!(),
            }
        });

        m.raw_function("unregister_texture", |_cx, args| {
            let [Value::Int(id)] = args else {
                return Err(RuntimeError::new("`unregister_texture` takes a texture id (int)"));
            };
            hao_egui::unregister_user_texture(TextureId::User(*id as u64));
            Ok(Value::Unit)
        });

        register_run(m);
        register_frame_count(m);
        register_key(m);
        register_image(m);
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

fn register_frame_count(m: &mut xur_runtime::registry::ModuleBuilder<'_>) {
    m.raw_function("frame_count", move |_cx, args| {
        if !args.is_empty() {
            return Err(RuntimeError::new("`frame_count` takes no arguments"));
        }
        let count = BRIDGE
            .get()
            .map_or(0, |b| b.frames.load(Ordering::Relaxed));
        Ok(Value::Int(count as i64))
    });
}

fn register_key(m: &mut xur_runtime::registry::ModuleBuilder<'_>) {
    let mut ty = m.ty_existing::<EguiCtx>();
    ty.raw_function("key", |_cx, args| {
        let [this, Value::Str(name)] = args else {
            return Err(RuntimeError::new("`key` takes a key name string"));
        };
        let _ = with_ctx(this, |_ctx| Ok(Value::Unit))?;
        let code = parse_keycode(name)
            .ok_or_else(|| RuntimeError::new(format!("unknown key: {name}")))?;
        Ok(Value::Bool(input::key(code).unwrap_or(false)))
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

fn register_image(m: &mut xur_runtime::registry::ModuleBuilder<'_>) {
    let mut ty = m.ty_existing::<EguiUi>();
    ty.raw_function("image", |_cx, args| {
        let [this, Value::Int(tex_id), Value::Float(width), Value::Float(height)] = args else {
            return Err(RuntimeError::new(
                "`image` takes (texture_id: int, width: float, height: float)",
            ));
        };
        with_ui(this, |ui| {
            let sized = hao_egui::load::SizedTexture::new(
                TextureId::User(*tex_id as u64),
                Vec2::new(*width as f32, *height as f32),
            );
            ui.add(Image::new(sized));
            Ok(Value::Unit)
        })
    });
}
