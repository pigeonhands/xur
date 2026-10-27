//! Naming written types at runtime.

use xur_ast::{Ty, TyKind};
use xur_resolve::{DefId, DefKind, Res, Resolutions};

/// How a written type is named at runtime, e.g. by `type_name`: `u32`,
/// `Point`, `[u8]`, or `_` for a type left to be inferred.
pub fn type_name(resolutions: &Resolutions, ty: &Ty) -> String {
    match &ty.kind {
        TyKind::Path(path) => match resolutions.res.get(&path.id) {
            Some(Res::Def(_, def)) => resolutions.defs.get(*def).name.to_string(),
            Some(Res::PrimTy(prim)) => prim.name().to_owned(),
            _ => "_".to_owned(),
        },
        TyKind::Tuple(tys) => {
            let names: Vec<_> = tys.iter().map(|ty| type_name(resolutions, ty)).collect();
            format!("({})", names.join(", "))
        }
        TyKind::Paren(ty) => type_name(resolutions, ty),
        TyKind::List(ty) => format!("[{}]", type_name(resolutions, ty)),
        TyKind::Infer | TyKind::ImplicitSelf | TyKind::Err => "_".to_owned(),
    }
}

/// The struct, enum or native type a written type names, if any.
pub fn type_def(resolutions: &Resolutions, ty: &Ty) -> Option<DefId> {
    match &ty.kind {
        TyKind::Path(path) => match resolutions.res.get(&path.id)? {
            &Res::Def(DefKind::Struct | DefKind::Enum | DefKind::NativeType(_), def) => Some(def),
            _ => None,
        },
        TyKind::Paren(ty) => type_def(resolutions, ty),
        _ => None,
    }
}
