//! Lowering patterns to tests and bindings.

use xur_ast::{LitKind, Pat, PatField, PatKind, Path, VariantData};
use xur_resolve::{DefKind, Res};

use xur_span::Symbol;

use super::Lowerer;
use crate::ir::{BlockId, Constant, DefRef, InstKind, Reg, Terminator};

impl Lowerer<'_> {
    /// Matches `value` against `pat`, binding its variables, and continues
    /// in a new current block; jumps to `fail` if it doesn't match.
    pub(super) fn pat(&mut self, pat: &Pat, value: Reg, fail: BlockId) {
        let span = pat.span;
        match &pat.kind {
            PatKind::Wild => {}
            PatKind::Ident(ident) => match self.resolutions.res.get(&pat.id).copied() {
                Some(Res::Local(binding)) if binding == pat.id => {
                    let reg = self.new_reg();
                    self.emit(
                        InstKind::Copy {
                            dst: reg,
                            src: value,
                        },
                        span,
                    );
                    self.locals.insert(pat.id, reg);
                }
                // A unit variant, unit struct or constant named alone.
                Some(Res::Def(DefKind::Variant | DefKind::Struct, ctor)) => {
                    self.test_ctor(value, ctor, fail, span)
                }
                Some(res @ Res::Def(DefKind::NativeVariant(_), _)) => {
                    self.test_record(res, value, fail, span)
                }
                Some(Res::Def(DefKind::Const, def)) => {
                    let constant = self.new_reg();
                    self.emit(
                        InstKind::Def {
                            dst: constant,
                            def: DefRef::Const(def),
                        },
                        span,
                    );
                    self.test_eq(value, constant, fail, span);
                }
                _ => {
                    self.error(format!("cannot match against `{ident}`"), span);
                }
            },
            PatKind::Lit(lit) => {
                let constant = match &lit.kind {
                    LitKind::Str(s) => Constant::Str(s.as_str().into()),
                    LitKind::ByteStr(bytes) => Constant::Bytes(bytes.clone()),
                    &LitKind::Int(i) => Constant::Int(i64::try_from(i).unwrap_or(i64::MAX)),
                    LitKind::Float(digits) => {
                        Constant::Float(digits.as_str().parse().unwrap_or(f64::NAN))
                    }
                    &LitKind::Bool(b) => Constant::Bool(b),
                    LitKind::Err => Constant::Unit,
                };
                let constant = self.constant(constant, span);
                self.test_eq(value, constant, fail, span);
            }
            PatKind::Path(path) => self.pat_path(path, value, fail),
            PatKind::TupleStruct(path, fields) => {
                self.pat_path(path, value, fail);
                if self.is_native_ctor(path) {
                    // A record's tuple fields are named `0`, `1`, ...
                    for (index, field) in fields.iter().enumerate() {
                        let name = Symbol::intern(&index.to_string());
                        self.pat_record_field(name, field, value, fail);
                    }
                } else {
                    self.pat_fields(fields, value, fail);
                }
            }
            PatKind::Struct { path, fields, rest } => {
                self.pat_path(path, value, fail);
                if self.is_native_ctor(path) {
                    // The VM finds the fields by name; a field the record
                    // lacks makes the pattern a runtime error. Without `..`,
                    // the pattern must name every field, which the record's
                    // arity tells us at runtime since its fields aren't
                    // known while lowering.
                    self.reject_duplicate_fields(fields);
                    if !*rest {
                        let is = self.new_reg();
                        let len = fields.len() as u32;
                        self.emit(
                            InstKind::RecordArity {
                                dst: is,
                                value,
                                len,
                            },
                            span,
                        );
                        self.test(is, fail, span);
                    }
                    for field in fields {
                        self.pat_record_field(field.ident.name, &field.pat, value, fail);
                    }
                } else {
                    self.pat_named_fields(path, fields, *rest, value, fail);
                }
            }
            PatKind::Tuple(pats) => {
                let is = self.new_reg();
                let len = pats.len() as u32;
                self.emit(
                    InstKind::IsTuple {
                        dst: is,
                        value,
                        len,
                    },
                    span,
                );
                self.test(is, fail, span);
                self.pat_fields(pats, value, fail);
            }
            PatKind::List(pats, rest) => {
                let is = self.new_reg();
                let len = pats.len() as u32;
                self.emit(
                    InstKind::IsList {
                        dst: is,
                        value,
                        len,
                        at_least: *rest,
                    },
                    span,
                );
                self.test(is, fail, span);
                self.pat_fields(pats, value, fail);
            }
            PatKind::Paren(inner) => self.pat(inner, value, fail),
            PatKind::Err => self.terminate(Terminator::Goto(fail), span),
        }
    }

    /// Binds a pattern that is expected to always match, as in `let`, a
    /// parameter or a `for` loop; not matching is a runtime error.
    pub(super) fn bind_irrefutable(&mut self, pat: &Pat, value: Reg) {
        // The common case, a plain binding, needs no test or copy.
        if let PatKind::Ident(_) = pat.kind
            && self.resolutions.res.get(&pat.id) == Some(&Res::Local(pat.id))
        {
            self.locals.insert(pat.id, value);
            return;
        }
        let fail = self.new_block();
        self.pat(pat, value, fail);
        let current = self.current;
        self.switch_to(fail);
        self.terminate(Terminator::NoMatch, pat.span);
        self.switch_to(current);
    }

    /// `let Ok(x) = expr?;` — binds the pattern or early-returns the value.
    pub(super) fn bind_try(&mut self, pat: &Pat, value: Reg) {
        let fail = self.new_block();
        self.pat(pat, value, fail);
        let current = self.current;
        self.switch_to(fail);
        self.terminate(Terminator::TryReturn(value), pat.span);
        self.switch_to(current);
    }

    fn pat_path(&mut self, path: &Path, value: Reg, fail: BlockId) {
        match self.resolutions.res.get(&path.id).copied() {
            Some(Res::Def(DefKind::Variant | DefKind::Struct, ctor)) => {
                self.test_ctor(value, ctor, fail, path.span)
            }
            Some(res @ Res::Def(DefKind::NativeVariant(_) | DefKind::NativeType(_), _)) => {
                self.test_record(res, value, fail, path.span)
            }
            Some(Res::Def(DefKind::Const, def)) => {
                let constant = self.new_reg();
                self.emit(
                    InstKind::Def {
                        dst: constant,
                        def: DefRef::Const(def),
                    },
                    path.span,
                );
                self.test_eq(value, constant, fail, path.span);
            }
            _ => {
                self.error("expected a variant, struct or constant", path.span);
            }
        }
    }

    fn pat_fields(&mut self, fields: &[Pat], value: Reg, fail: BlockId) {
        for (index, field) in fields.iter().enumerate() {
            let element = self.new_reg();
            let index = index as u32;
            self.emit(
                InstKind::Project {
                    dst: element,
                    value,
                    index,
                },
                field.span,
            );
            self.pat(field, element, fail);
        }
    }

    /// Matches the fields named in a struct pattern; without `..`, every
    /// field must be named.
    fn pat_named_fields(
        &mut self,
        path: &Path,
        fields: &[PatField],
        rest: bool,
        value: Reg,
        fail: BlockId,
    ) {
        let declared = match self.resolutions.res.get(&path.id).copied() {
            Some(Res::Def(DefKind::Variant | DefKind::Struct, ctor)) => {
                match self.ctor_shape(ctor) {
                    Some(VariantData::Struct(declared)) => declared,
                    _ => {
                        self.error("this has no named fields; use `Name(...)`", path.span);
                        return;
                    }
                }
            }
            // `pat_path` has reported it.
            _ => return,
        };
        let position = |name| {
            declared
                .iter()
                .position(|d| d.ident.map(|i| i.name) == Some(name))
        };
        if !rest {
            let missing: Vec<_> = declared
                .iter()
                .filter_map(|d| d.ident)
                .filter(|ident| fields.iter().all(|field| field.ident.name != ident.name))
                .map(|ident| format!("`{ident}`"))
                .collect();
            if !missing.is_empty() {
                let message = format!("pattern does not mention field(s) {}", missing.join(", "));
                self.error(message, path.span);
                return;
            }
        }
        for field in fields {
            let Some(index) = position(field.ident.name) else {
                let message = format!("no field `{}` in this struct", field.ident);
                self.error(message, field.span);
                return;
            };
            let element = self.new_reg();
            let index = index as u32;
            self.emit(
                InstKind::Project {
                    dst: element,
                    value,
                    index,
                },
                field.span,
            );
            self.pat(&field.pat, element, fail);
        }
    }

    fn is_native_ctor(&self, path: &Path) -> bool {
        let res = self.resolutions.res.get(&path.id).copied();
        res.and_then(|res| self.native_ctor(res)).is_some()
    }

    /// Tests that `value` is a record of the native struct or variant `res`.
    fn test_record(&mut self, res: Res, value: Reg, fail: BlockId, span: xur_span::Span) {
        let Some((ty, variant)) = self.native_ctor(res) else {
            return;
        };
        let is = self.new_reg();
        self.emit(
            InstKind::IsRecord {
                dst: is,
                value,
                ty,
                variant,
            },
            span,
        );
        self.test(is, fail, span);
    }

    /// Reports fields named more than once in a struct pattern.
    fn reject_duplicate_fields(&mut self, fields: &[PatField]) {
        for (i, field) in fields.iter().enumerate() {
            if fields[..i]
                .iter()
                .any(|earlier| earlier.ident.name == field.ident.name)
            {
                self.error(
                    format!("field `{}` bound more than once", field.ident),
                    field.span,
                );
            }
        }
    }

    fn pat_record_field(&mut self, name: Symbol, pat: &Pat, value: Reg, fail: BlockId) {
        let field = self.new_reg();
        self.emit(
            InstKind::Field {
                dst: field,
                base: value,
                field: name,
            },
            pat.span,
        );
        self.pat(pat, field, fail);
    }

    fn test_ctor(
        &mut self,
        value: Reg,
        ctor: xur_resolve::DefId,
        fail: BlockId,
        span: xur_span::Span,
    ) {
        let is = self.new_reg();
        self.emit(
            InstKind::IsCtor {
                dst: is,
                value,
                ctor,
            },
            span,
        );
        self.test(is, fail, span);
    }

    fn test_eq(&mut self, value: Reg, expected: Reg, fail: BlockId, span: xur_span::Span) {
        let is = self.new_reg();
        let op = xur_ast::BinOpKind::Eq;
        self.emit(
            InstKind::Binary {
                dst: is,
                op,
                lhs: value,
                rhs: expected,
            },
            span,
        );
        self.test(is, fail, span);
    }

    /// Continues in a new block if `cond` holds, else jumps to `fail`.
    fn test(&mut self, cond: Reg, fail: BlockId, span: xur_span::Span) {
        let then = self.new_block();
        self.terminate(
            Terminator::Branch {
                cond,
                then,
                otherwise: fail,
            },
            span,
        );
        self.switch_to(then);
    }
}
