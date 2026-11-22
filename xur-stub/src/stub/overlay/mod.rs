mod egui;

use xur_runtime::Registry;

pub(crate) fn register(registry: &Registry) {
    egui::register(registry);
}
