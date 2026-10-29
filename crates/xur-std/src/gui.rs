//! `std::gui`: desktop GUI with [eframe](https://docs.rs/eframe)/[egui](https://docs.rs/egui).
//!
//! ```text
//! use std::gui;
//!
//! fn main() {
//!     gui::run("My App", |ui| {
//!         ui.heading("Welcome");
//!         if ui.button("Click me") {
//!             io::println("clicked!");
//!         }
//!     });
//! }
//! ```

use eframe::egui;
use facet::Facet;
use xur_runtime::convert::into_value;
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{Registry, RuntimeError, Value};

use crate::error_text;

// ---------------------------------------------------------------------------
// Opaque wrapper
// ---------------------------------------------------------------------------

#[derive(Facet, Clone)]
#[facet(opaque)]
struct GuiUi(usize);

unsafe impl Send for GuiUi {}
unsafe impl Sync for GuiUi {}

impl GuiUi {
    fn from_ui(ui: &mut egui::Ui) -> Self {
        Self(ui as *mut egui::Ui as usize)
    }

    fn ui(&self) -> &mut egui::Ui {
        unsafe { &mut *(self.0 as *mut egui::Ui) }
    }
}

impl std::fmt::Display for GuiUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ui")
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::gui", |m| {
        let registry = m.registry();
        register_run(m);
        register_ui_methods(&registry, m);
    });
}

// ---------------------------------------------------------------------------
// gui::run(title, draw)  /  gui::dialog(title, draw)
// ---------------------------------------------------------------------------

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
    // Safety: `run_native` blocks until the window closes, so the `cx`
    // borrow from the enclosing `raw_function` closure lives for the
    // entire lifetime of `XurApp`. We erase the non-`'static` lifetime
    // on the fat pointer so it can move into the `'static` app creator.
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
        fn cx(&self) -> &mut dyn xur_runtime::Context {
            unsafe {
                let fat: *mut dyn xur_runtime::Context = std::mem::transmute(self.cx_erased);
                &mut *fat
            }
        }
    }

    impl eframe::App for XurApp {
        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            let draw = self.draw.clone();
            let cx = self.cx();
            match into_value(cx, GuiUi::from_ui(ui)) {
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

// ---------------------------------------------------------------------------
// Ui methods
// ---------------------------------------------------------------------------

fn register_ui_methods(_registry: &Registry, m: &mut ModuleBuilder<'_>) {
    let mut ty = m.ty::<GuiUi>("Ui");

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
            ui.add(egui::Slider::new(&mut val, (*min as f32)..=(*max as f32)).text(label.as_ref()));
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
            ui.add(egui::Slider::new(&mut val, *min..=*max).text(label.as_ref()));
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
        let cx_ptr = cx as *mut dyn xur_runtime::Context;
        with_ui(this, |ui| {
            ui.horizontal(|inner_ui| {
                let cx = unsafe { &mut *cx_ptr };
                let inner = into_value(cx, GuiUi::from_ui(inner_ui)).expect("GuiUi registered");
                let _ = cx.call(&body, &[inner]);
            });
            Ok(Value::Unit)
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
        let cx_ptr = cx as *mut dyn xur_runtime::Context;
        with_ui(this, |ui| {
            ui.vertical(|inner_ui| {
                let cx = unsafe { &mut *cx_ptr };
                let inner = into_value(cx, GuiUi::from_ui(inner_ui)).expect("GuiUi registered");
                let _ = cx.call(&body, &[inner]);
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
        let cx_ptr = cx as *mut dyn xur_runtime::Context;
        with_ui(this, |ui| {
            ui.collapsing(heading.as_ref(), |inner_ui| {
                let cx = unsafe { &mut *cx_ptr };
                let inner = into_value(cx, GuiUi::from_ui(inner_ui)).expect("GuiUi registered");
                let _ = cx.call(&body, &[inner]);
            });
            Ok(Value::Unit)
        })
    });

    ty.raw_function("scroll", |cx, args| {
        let [this, body] = args else {
            return Err(RuntimeError::new("`scroll` takes a fn(ui)"));
        };
        if !matches!(body, Value::Closure(_) | Value::Fn(_)) {
            return Err(RuntimeError::new("body must be a function"));
        }
        let body = body.clone();
        let cx_ptr = cx as *mut dyn xur_runtime::Context;
        with_ui(this, |ui| {
            egui::ScrollArea::vertical().show(ui, |inner_ui| {
                let cx = unsafe { &mut *cx_ptr };
                let inner = into_value(cx, GuiUi::from_ui(inner_ui)).expect("GuiUi registered");
                let _ = cx.call(&body, &[inner]);
            });
            Ok(Value::Unit)
        })
    });

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
        let cx_ptr = cx as *mut dyn xur_runtime::Context;
        with_ui(this, |ui| {
            let ctx = ui.ctx().clone();
            egui::Window::new(title.as_ref())
                .default_width(400.0)
                .show(&ctx, |inner_ui| {
                    let cx = unsafe { &mut *cx_ptr };
                    let inner =
                        into_value(cx, GuiUi::from_ui(inner_ui)).expect("GuiUi registered");
                    if let Err(e) = cx.call(&body, &[inner]) {
                        inner_ui
                            .colored_label(egui::Color32::RED, format!("error: {}", e.message));
                    }
                });
            Ok(Value::Unit)
        })
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
                let inner = into_value(cx, GuiUi::from_ui(inner_ui)).expect("GuiUi registered");
                let _ = cx.call(&body, &[inner]);
            });
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
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn with_ui(
    val: &Value,
    f: impl FnOnce(&mut egui::Ui) -> Result<Value, RuntimeError>,
) -> Result<Value, RuntimeError> {
    let Value::Native(obj) = val else {
        return Err(RuntimeError::new("expected a `Ui`"));
    };
    obj.with(|wrapper: &GuiUi| f(wrapper.ui()))
        .ok_or_else(|| RuntimeError::new("expected a `Ui`"))?
}
