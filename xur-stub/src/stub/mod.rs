#[cfg(feature = "egui")]
pub(crate) mod egui;
#[cfg(feature = "hooking")]
pub(crate) mod hooking;
#[cfg(feature = "il2cpp-runtime")]
pub(crate) mod il2cpp;
mod re;
#[cfg(feature = "il2cpp-unity")]
mod time;

use xur_runtime::Registry;

pub(crate) fn register(registry: &Registry) {
    re::register(registry);
    #[cfg(feature = "il2cpp-unity")]
    time::register(registry);
    #[cfg(feature = "egui")]
    egui::register(registry);
    #[cfg(feature = "hooking")]
    hooking::register(registry);
    #[cfg(feature = "il2cpp-runtime")]
    il2cpp::register(registry);
}
