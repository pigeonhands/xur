use egui::Context;
use facet::Facet;
use xur_runtime::{RuntimeError, Value};

#[derive(Facet, Clone)]
#[facet(opaque)]
pub struct EguiCtx(usize);

unsafe impl Send for EguiCtx {}
unsafe impl Sync for EguiCtx {}

impl EguiCtx {
    pub fn from_ctx(ctx: &Context) -> Self {
        Self(ctx as *const Context as usize)
    }

    pub fn ctx(&self) -> &Context {
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
pub struct EguiUi(usize);

unsafe impl Send for EguiUi {}
unsafe impl Sync for EguiUi {}

impl EguiUi {
    pub fn from_ui(ui: &mut egui::Ui) -> Self {
        Self(ui as *mut egui::Ui as usize)
    }

    pub fn ui(&self) -> &mut egui::Ui {
        unsafe { &mut *(self.0 as *mut egui::Ui) }
    }
}

impl std::fmt::Display for EguiUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ui")
    }
}

pub fn with_ctx(
    val: &Value,
    f: impl FnOnce(&Context) -> Result<Value, RuntimeError>,
) -> Result<Value, RuntimeError> {
    let Value::Native(obj) = val else {
        return Err(RuntimeError::new("expected a `Ctx`"));
    };
    obj.with(|wrapper: &EguiCtx| f(wrapper.ctx()))
        .ok_or_else(|| RuntimeError::new("expected a `Ctx`"))?
}

pub fn with_ui(
    val: &Value,
    f: impl FnOnce(&mut egui::Ui) -> Result<Value, RuntimeError>,
) -> Result<Value, RuntimeError> {
    let Value::Native(obj) = val else {
        return Err(RuntimeError::new("expected a `Ui`"));
    };
    obj.with(|wrapper: &EguiUi| f(wrapper.ui()))
        .ok_or_else(|| RuntimeError::new("expected a `Ui`"))?
}
