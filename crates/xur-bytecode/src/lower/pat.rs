//! Lowering patterns to tests and bindings.

use xur_ast::{LitKind, Pat, PatKind, Path};
use xur_resolve::{DefKind, Res};

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
                self.pat_fields(fields, value, fail);
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

    fn pat_path(&mut self, path: &Path, value: Reg, fail: BlockId) {
        match self.resolutions.res.get(&path.id).copied() {
            Some(Res::Def(DefKind::Variant | DefKind::Struct, ctor)) => {
                self.test_ctor(value, ctor, fail, path.span)
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
