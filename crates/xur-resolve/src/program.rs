use std::io;
use std::path::Path;

use xur_errors::Diag;
use xur_span::FileLoader;

use crate::loader::{Loader, normalize};
use crate::{Crate, ModuleIdx, Resolutions, resolve_modules};

/// A loaded, name-resolved program that can grow while it runs.
///
/// [`Program::load`] loads the root file and every module it imports with
/// a literal path. [`Program::import`] adds more modules later, as a
/// dynamic `std::import` does; existing definitions, node ids and spans stay
/// valid.
pub struct Program {
    pub krate: Crate,
    /// Only `None` while a batch of modules is being resolved.
    resolutions: Option<Resolutions>,
    loader: Loader,
}

impl Program {
    /// Loads and resolves the program rooted at `root`.
    ///
    /// Fails only if `root` itself cannot be read; everything else wrong
    /// with the program is returned as diagnostics.
    pub fn load(root: &Path, file_loader: Box<dyn FileLoader>) -> io::Result<(Self, Vec<Diag>)> {
        let src = file_loader.read_file(root)?;
        let mut loader = Loader::new(file_loader);
        let mut diags = Vec::new();
        let mut krate = Crate::with_prelude(&mut loader, &mut diags);
        let root = loader.add_tree(&mut krate, normalize(root), src, &mut diags);
        debug_assert_eq!(root, krate.root);

        let (resolutions, resolve_diags) =
            resolve_modules(&krate, Resolutions::new(), 0..krate.modules.len());
        diags.extend(resolve_diags);
        let program = Self {
            krate,
            resolutions: Some(resolutions),
            loader,
        };
        Ok((program, diags))
    }

    pub fn resolutions(&self) -> &Resolutions {
        self.resolutions
            .as_ref()
            .expect("resolutions are only taken while resolving")
    }

    /// Loads and resolves the module at `path` (relative to the working
    /// directory) and anything it imports, unless already loaded.
    ///
    /// Fails if `path` cannot be read. Diagnostics are for the newly loaded
    /// modules only.
    pub fn import(&mut self, path: &Path) -> io::Result<(ModuleIdx, Vec<Diag>)> {
        let path = normalize(path);
        if let Some(loaded) = self.loader.loaded(&path) {
            return Ok((loaded, Vec::new()));
        }
        let src = self.loader.read(&path)?;
        let first_new = self.krate.modules.len();
        let mut diags = Vec::new();
        let idx = self.loader.add_tree(&mut self.krate, path, src, &mut diags);

        let resolutions = self
            .resolutions
            .take()
            .expect("resolutions are only taken while resolving");
        let (resolutions, resolve_diags) = resolve_modules(
            &self.krate,
            resolutions,
            first_new..self.krate.modules.len(),
        );
        self.resolutions = Some(resolutions);
        diags.extend(resolve_diags);
        Ok((idx, diags))
    }
}
