mod types;
mod widgets;
#[cfg(feature = "gui")]
mod desktop;

pub use types::{EguiCtx, EguiUi, Texture, with_ctx, with_ui};

use xur_runtime::Registry;

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::gui", |m| {
        let registry = m.registry();
        m.ty::<Texture>("Texture");
        widgets::register_ctx_methods(registry, m);
        widgets::register_ui_methods(registry, m);
        #[cfg(feature = "gui")]
        desktop::register_desktop(m);
    });
    // Expand eagerly so Ctx/Ui types are available to other modules
    // (backends call ty_existing during their own lazy init).
    registry.expand(&["std".into(), "gui".into()]);
}
