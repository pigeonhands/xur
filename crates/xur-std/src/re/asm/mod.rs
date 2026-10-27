//! `std::re::asm`: assembly and disassembly.

mod x86;

use xur_runtime::Registry;
use xur_runtime::registry::ModuleBuilder;

pub(super) fn register(registry: &Registry, re: &mut ModuleBuilder<'_>) {
    let mut asm = re.submodule("asm");
    x86::register(registry, &mut asm);
}
