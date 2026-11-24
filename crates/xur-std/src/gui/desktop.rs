use eframe::egui;
use xur_runtime::convert::into_value;
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{RuntimeError, Value};

use super::types::{EguiUi, with_ui};
use crate::error_text;

pub(crate) fn register_desktop(m: &mut ModuleBuilder<'_>) {
    register_run(m);
    register_desktop_ui_methods(m);
}

fn register_run(m: &mut ModuleBuilder<'_>) {
    m.raw_function("run", |cx, args| {
        let [Value::Str(title), draw] = args else {
            return Err(RuntimeError::new("`run` takes (title: str, draw: fn(ui))"));
        };
        let viewport = egui::ViewportBuilder::default().with_inner_size([600.0, 400.0]);
        run_native(cx, title, draw, viewport)
    });

    m.raw_function("dialog", |cx, args| {
        let [Value::Str(title), draw] = args else {
            return Err(RuntimeError::new(
                "`dialog` takes (title: str, draw: fn(ui))",
            ));
        };
        let viewport = egui::ViewportBuilder::default()
            .with_inner_size([600.0, 400.0])
            .with_window_type(egui::X11WindowType::Dialog);
        run_native(cx, title, draw, viewport)
    });
}

fn run_native(
    cx: &mut dyn xur_runtime::Context,
    title: &std::sync::Arc<str>,
    draw: &Value,
    viewport: egui::ViewportBuilder,
) -> Result<Value, RuntimeError> {
    if !matches!(draw, Value::Closure(_) | Value::Fn(_)) {
        return Err(RuntimeError::new("`draw` must be a function or closure"));
    }

    let title = title.to_string();
    let draw = draw.clone();
    let cx_raw: *mut dyn xur_runtime::Context = cx;
    let cx_erased: [usize; 2] = unsafe { std::mem::transmute(cx_raw) };

    let options = eframe::NativeOptions {
        viewport,
        event_loop_builder: Some(Box::new(|builder| {
            #[cfg(target_os = "linux")]
            {
                use winit::platform::x11::EventLoopBuilderExtX11 as _;
                builder.with_any_thread(true);
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = builder;
            }
        })),
        ..Default::default()
    };

    struct XurApp {
        draw: Value,
        cx_erased: [usize; 2],
    }

    unsafe impl Send for XurApp {}

    impl XurApp {
        #[allow(clippy::mut_from_ref)]
        fn cx(&self) -> &mut dyn xur_runtime::Context {
            unsafe {
                let fat: *mut dyn xur_runtime::Context = std::mem::transmute(self.cx_erased);
                &mut *fat
            }
        }
    }

    impl eframe::App for XurApp {
        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            ui.ctx().request_repaint();
            let draw = self.draw.clone();
            let cx = self.cx();
            match into_value(cx, EguiUi::from_ui(ui)) {
                Ok(ui_val) => {
                    if let Err(e) = cx.call(&draw, &[ui_val]) {
                        ui.colored_label(
                            egui::Color32::RED,
                            format!("error: {}", e.message),
                        );
                    }
                }
                Err(e) => {
                    ui.colored_label(egui::Color32::RED, format!("error: {}", e.message));
                }
            }
        }
    }

    let app = XurApp { draw, cx_erased };
    eframe::run_native(
        &title,
        options,
        Box::new(move |_cc| Ok(Box::new(app))),
    )
    .map_err(|e| RuntimeError::new(error_text(e)))?;

    Ok(Value::Unit)
}

fn register_desktop_ui_methods(m: &mut ModuleBuilder<'_>) {
    let mut ty = m.ty_existing::<EguiUi>();

    ty.raw_function("window", |cx, args| {
        let [this, Value::Str(title), body] = args else {
            return Err(RuntimeError::new(
                "`window` takes (title: str, body: fn(ui))",
            ));
        };
        show_viewport(cx, this, title, body, None)
    });

    ty.raw_function("dialog", |cx, args| {
        let [this, Value::Str(title), body] = args else {
            return Err(RuntimeError::new(
                "`dialog` takes (title: str, body: fn(ui))",
            ));
        };
        show_viewport(cx, this, title, body, Some(egui::X11WindowType::Dialog))
    });

    ty.raw_function("panel_left", |cx, args| {
        let [this, Value::Str(id), body] = args else {
            return Err(RuntimeError::new(
                "`panel_left` takes (id: str, body: fn(ui))",
            ));
        };
        if !matches!(body, Value::Closure(_) | Value::Fn(_)) {
            return Err(RuntimeError::new("body must be a function"));
        }
        let body = body.clone();
        let id = id.clone();
        let cx_ptr = cx as *mut dyn xur_runtime::Context;
        with_ui(this, |ui| {
            egui::Panel::left(id.to_string()).show(ui, |inner_ui| {
                let cx = unsafe { &mut *cx_ptr };
                let inner = into_value(cx, EguiUi::from_ui(inner_ui)).expect("EguiUi registered");
                let _ = cx.call(&body, &[inner]);
            });
            Ok(Value::Unit)
        })
    });
}

fn show_viewport(
    cx: &mut dyn xur_runtime::Context,
    this: &Value,
    title: &std::sync::Arc<str>,
    body: &Value,
    window_type: Option<egui::X11WindowType>,
) -> Result<Value, RuntimeError> {
    if !matches!(body, Value::Closure(_) | Value::Fn(_)) {
        return Err(RuntimeError::new("body must be a function"));
    }
    let body = body.clone();
    let title = title.clone();
    let cx_ptr = cx as *mut dyn xur_runtime::Context;
    with_ui(this, |ui| {
        let ctx = ui.ctx().clone();
        let id = egui::ViewportId::from_hash_of(&*title);
        let mut vp = egui::ViewportBuilder::default()
            .with_title(title.as_ref())
            .with_inner_size([400.0, 300.0]);
        if let Some(wt) = window_type {
            vp = vp.with_window_type(wt);
        }
        ctx.show_viewport_immediate(id, vp, |inner_ui, _class| {
            let cx = unsafe { &mut *cx_ptr };
            let inner = into_value(cx, EguiUi::from_ui(inner_ui)).expect("EguiUi registered");
            if let Err(e) = cx.call(&body, &[inner]) {
                inner_ui.colored_label(egui::Color32::RED, format!("error: {}", e.message));
            }
        });
        Ok(Value::Unit)
    })
}
