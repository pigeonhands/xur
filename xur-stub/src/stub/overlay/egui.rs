use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use hao_egui_overlay::egui;
use xur_runtime::convert::into_value;
use xur_runtime::{Registry, RuntimeError, Value};

use xur_std::gui::{with_ctx, EguiCtx};

struct Bridge {
    mu: Mutex<BridgeState>,
    cv: Condvar,
    ready: AtomicBool,
    frames: AtomicU64,
}

struct BridgeState {
    ctx: Option<egui::Context>,
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
    tracing::info!("overlay::egui: init_early called");
    let bridge = get_or_init_bridge();
    let render_bridge = Arc::clone(&bridge);

    hao_egui_overlay::on_frame(move |ctx| {
        let n = render_bridge.frames.fetch_add(1, Ordering::Relaxed);
        if n == 0 {
            tracing::info!("overlay: first frame from render thread");
        }
        if n < 3 {
            tracing::debug!(
                "overlay: frame {n}, ready={}",
                render_bridge.ready.load(Ordering::Relaxed)
            );
        }

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
    });

    tracing::info!("overlay::egui: calling hao_egui_overlay::init()");
    hao_egui_overlay::init();
    tracing::info!("overlay::egui: init complete");
}

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::stub::overlay::egui", |m| {
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
        register_key(m);
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

        tracing::info!("overlay::egui: run() called, marking ready");
        bridge.ready.store(true, Ordering::Relaxed);

        let mut frame_n = 0u64;
        loop {
            let ctx = {
                let mut state = bridge.mu.lock().unwrap();
                while state.ctx.is_none() && !state.shutdown {
                    state = bridge.cv.wait(state).unwrap();
                }
                if state.shutdown {
                    tracing::info!("overlay::egui: shutdown received");
                    return Ok(Value::Unit);
                }
                state.ctx.take().unwrap()
            };

            if frame_n == 0 {
                tracing::info!("overlay::egui: first draw dispatch to script");
            }
            frame_n += 1;

            let ctx_val = into_value(cx, EguiCtx::from_ctx(&ctx)).expect("EguiCtx registered");
            if let Err(e) = cx.call(&draw, &[ctx_val]) {
                tracing::error!("draw error: {}", e.message);
                egui::Window::new("xur error").show(&ctx, |ui| {
                    ui.colored_label(egui::Color32::RED, format!("draw error: {}", e.message));
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
        let count = BRIDGE.get().map_or(0, |b| b.frames.load(Ordering::Relaxed));
        Ok(Value::Int(count as i64))
    });
}

fn register_key(m: &mut xur_runtime::registry::ModuleBuilder<'_>) {
    let mut ty = m.ty_existing::<EguiCtx>();
    ty.raw_function("key", |_cx, args| {
        let [this, Value::Str(name)] = args else {
            return Err(RuntimeError::new("`key` takes a key name string"));
        };
        with_ctx(this, |ctx| {
            let pressed = ctx.input(|i| match name.to_ascii_lowercase().as_str() {
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
            });
            Ok(Value::Bool(pressed))
        })
    });
}
