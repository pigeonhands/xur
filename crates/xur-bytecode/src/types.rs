//! Naming written types at runtime, and the runtime checks for them.

use xur_ast::{Ty, TyKind};
use xur_resolve::{DefId, DefKind, PrimTy, Res, Resolutions};

use crate::ir::RtType;

/// The runtime check for a written type, or `None` when nothing about it can
/// be checked (an inferred or generic type).
pub fn rt_type(resolutions: &Resolutions, ty: &Ty) -> Option<RtType> {
    match rt_type_or_any(resolutions, ty) {
        RtType::Any => None,
        ty => Some(ty),
    }
}

fn rt_type_or_any(resolutions: &Resolutions, ty: &Ty) -> RtType {
    match &ty.kind {
        TyKind::Path(path) => match resolutions.res.get(&path.id) {
            Some(Res::PrimTy(prim)) => prim_rt_type(*prim),
            Some(&Res::Def(DefKind::Struct | DefKind::Enum, def)) => RtType::Adt {
                def,
                name: resolutions.def_path(def),
            },
            Some(&Res::Def(DefKind::NativeType(id), def)) => RtType::Native {
                id,
                name: resolutions.def_path(def),
            },
            _ => RtType::Any,
        },
        TyKind::Tuple(tys) if tys.is_empty() => RtType::Unit,
        TyKind::Tuple(tys) => RtType::Tuple(
            tys.iter()
                .map(|ty| rt_type_or_any(resolutions, ty))
                .collect(),
        ),
        TyKind::Paren(ty) => rt_type_or_any(resolutions, ty),
        TyKind::Ptr(_) => RtType::Ptr,
        TyKind::List(_) => RtType::List,
        TyKind::Fn { params, ret } => RtType::Fn {
            params: params.iter().map(|ty| rt_type_or_any(resolutions, ty)).collect(),
            ret: Box::new(match ret {
                Some(ty) => rt_type_or_any(resolutions, ty),
                None => RtType::Unit,
            }),
        },
        TyKind::Record | TyKind::Infer | TyKind::ImplicitSelf | TyKind::Err => RtType::Any,
    }
}

pub(crate) fn prim_rt_type(prim: PrimTy) -> RtType {
    let int = |name, min, max| RtType::Int { name, min, max };
    match prim {
        PrimTy::Bool => RtType::Bool,
        PrimTy::Str => RtType::Str,
        PrimTy::Bytes => RtType::Bytes,
        PrimTy::F32 => RtType::Float { name: "f32" },
        PrimTy::F64 => RtType::Float { name: "f64" },
        PrimTy::U8 => int("u8", 0, u8::MAX.into()),
        PrimTy::U16 => int("u16", 0, u16::MAX.into()),
        PrimTy::U32 => int("u32", 0, u32::MAX.into()),
        PrimTy::U64 => int("u64", 0, i64::MAX),
        PrimTy::Usize => int("usize", 0, i64::MAX),
        PrimTy::I8 => int("i8", i8::MIN.into(), i8::MAX.into()),
        PrimTy::I16 => int("i16", i16::MIN.into(), i16::MAX.into()),
        PrimTy::I32 => int("i32", i32::MIN.into(), i32::MAX.into()),
        PrimTy::I64 => int("i64", i64::MIN, i64::MAX),
        PrimTy::Isize => int("isize", i64::MIN, i64::MAX),
        PrimTy::Type => RtType::Type,
    }
}

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
        TyKind::Ptr(ty) => format!("*{}", type_name(resolutions, ty)),
        TyKind::List(ty) => format!("[{}]", type_name(resolutions, ty)),
        TyKind::Fn { params, ret } => {
            let params: Vec<_> = params.iter().map(|ty| type_name(resolutions, ty)).collect();
            let ret = match ret {
                Some(ty) => type_name(resolutions, ty),
                None => "()".to_owned(),
            };
            format!("fn({}) -> {}", params.join(", "), ret)
        }
        TyKind::Record => "@".to_owned(),
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
