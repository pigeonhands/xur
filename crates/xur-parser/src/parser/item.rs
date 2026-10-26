//! Items, attributes and whole source files, with item-level error recovery.

use xur_ast::{
    Attribute, ConstItem, EnumDef, FieldDef, Fn, FnDecl, FnRetTy, GenericParam, Generics, Item,
    ItemKind, MetaItem, MetaItemInner, MetaItemKind, Module, Param, UseTree, Variant, VariantData,
    Visibility,
};
use xur_span::Span;

use super::expected::Expected;
use super::{PResult, Parser, PathStyle};
use crate::token::{Keyword, TokenKind};

impl Parser<'_> {
    /// Parses a whole source file. An item that fails to parse is reported
    /// and skipped, so every broken item in the file is reported in one pass.
    pub fn parse_module(&mut self) -> Module {
        let mut items = Vec::new();
        while !self.is_at_end() {
            let start = self.pos;
            match self.parse_item() {
                Ok(item) => items.push(item),
                Err(diag) => {
                    self.report(diag);
                    self.recover_item();
                    if self.pos == start {
                        self.bump(); // always make progress
                    }
                }
            }
        }
        Module {
            items,
            span: Span::new(0, self.src.len()),
        }
    }

    /// Whether the current token can start an item.
    pub(super) fn is_item_start(&self) -> bool {
        matches!(
            self.token.kind,
            TokenKind::Hash
                | TokenKind::Keyword(
                    Keyword::Pub
                        | Keyword::Use
                        | Keyword::Const
                        | Keyword::Fn
                        | Keyword::Struct
                        | Keyword::Enum
                )
        )
    }

    pub fn parse_item(&mut self) -> PResult<Item> {
        let lo = self.token.span;
        let attrs = self.parse_outer_attrs()?;
        let vis = self.parse_vis();
        let kind = match self.token.kind {
            TokenKind::Keyword(Keyword::Use) => {
                self.bump();
                ItemKind::Use(self.parse_use_tree()?)
            }
            TokenKind::Keyword(Keyword::Const) => {
                self.bump();
                ItemKind::Const(Box::new(self.parse_const()?))
            }
            TokenKind::Keyword(Keyword::Fn) => {
                self.bump();
                ItemKind::Fn(Box::new(self.parse_fn()?))
            }
            TokenKind::Keyword(Keyword::Struct) => {
                self.bump();
                let ident = self.parse_ident()?;
                let generics = self.parse_generics()?;
                ItemKind::Struct(ident, generics, self.parse_struct_data()?)
            }
            TokenKind::Keyword(Keyword::Enum) => {
                self.bump();
                let ident = self.parse_ident()?;
                let generics = self.parse_generics()?;
                ItemKind::Enum(ident, generics, self.parse_enum_def()?)
            }
            _ => {
                self.expected(Expected::Item);
                return Err(self.unexpected());
            }
        };
        Ok(Item {
            attrs,
            vis,
            kind,
            span: lo.to(self.prev_token.span),
        })
    }

    /// Skips to the start of the next item. Delimited groups are skipped
    /// whole, so an item keyword inside a broken function body is ignored.
    fn recover_item(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.token.kind {
                TokenKind::Eof => return,
                _ if depth == 0 && self.is_item_start() => return,
                TokenKind::LBrace | TokenKind::LParen | TokenKind::LBracket => depth += 1,
                TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            self.bump();
        }
    }

    fn parse_vis(&mut self) -> Visibility {
        if self.eat_noexpect(TokenKind::Keyword(Keyword::Pub)) {
            Visibility::Public(self.prev_token.span)
        } else {
            Visibility::Inherited
        }
    }

    fn parse_outer_attrs(&mut self) -> PResult<Vec<Attribute>> {
        let mut attrs = Vec::new();
        while self.token.kind == TokenKind::Hash {
            let lo = self.token.span;
            self.bump();
            self.expect(TokenKind::LBracket)?;
            let meta = self.parse_meta_item()?;
            self.expect(TokenKind::RBracket)?;
            attrs.push(Attribute {
                meta,
                span: lo.to(self.prev_token.span),
            });
        }
        Ok(attrs)
    }

    /// `path`, `path(inner, ...)` or `path = literal`.
    fn parse_meta_item(&mut self) -> PResult<MetaItem> {
        let lo = self.token.span;
        let path = self.parse_path(PathStyle::Mod)?;
        let kind = if self.eat(TokenKind::LParen) {
            MetaItemKind::List(
                self.parse_delimited(TokenKind::RParen, Self::parse_meta_item_inner)?,
            )
        } else if self.eat(TokenKind::Eq) {
            MetaItemKind::NameValue(self.parse_lit()?)
        } else {
            MetaItemKind::Word
        };
        Ok(MetaItem {
            path,
            kind,
            span: lo.to(self.prev_token.span),
        })
    }

    fn parse_meta_item_inner(&mut self) -> PResult<MetaItemInner> {
        match self.parse_opt_lit() {
            Some(lit) => Ok(MetaItemInner::Lit(lit)),
            None => Ok(MetaItemInner::MetaItem(self.parse_meta_item()?)),
        }
    }

    /// `path;`, after the `use`.
    fn parse_use_tree(&mut self) -> PResult<UseTree> {
        let path = self.parse_path(PathStyle::Mod)?;
        let span = path.span;
        self.expect(TokenKind::Semi)?;
        Ok(UseTree { path, span })
    }

    /// `name: ty = expr;`, after the `const`.
    fn parse_const(&mut self) -> PResult<ConstItem> {
        let ident = self.parse_ident()?;
        let ty = if self.eat(TokenKind::Colon) {
            Some(self.parse_ty()?)
        } else {
            None
        };
        self.expect(TokenKind::Eq)?;
        let expr = Box::new(self.parse_expr()?);
        self.expect(TokenKind::Semi)?;
        Ok(ConstItem { ident, ty, expr })
    }

    /// `name<T>(params) -> ret { body }`, after the `fn`.
    fn parse_fn(&mut self) -> PResult<Fn> {
        let ident = self.parse_ident()?;
        let generics = self.parse_generics()?;
        self.expect(TokenKind::LParen)?;
        let inputs = self.parse_delimited(TokenKind::RParen, Self::parse_fn_param)?;
        let output = if self.eat(TokenKind::Arrow) {
            FnRetTy::Ty(Box::new(self.parse_ty()?))
        } else {
            FnRetTy::Default(self.prev_token.span.shrink_to_hi())
        };
        let body = Box::new(self.parse_block()?);
        Ok(Fn {
            ident,
            generics,
            decl: FnDecl { inputs, output },
            body,
        })
    }

    fn parse_fn_param(&mut self) -> PResult<Param> {
        let lo = self.token.span;
        let pat = self.parse_pat()?;
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_ty()?;
        Ok(Param {
            pat,
            ty,
            span: lo.to(self.prev_token.span),
        })
    }

    /// `<T, U>` after a declaration's name, if present.
    fn parse_generics(&mut self) -> PResult<Generics> {
        let lo = self.token.span;
        if !self.eat(TokenKind::Lt) {
            return Ok(Generics {
                params: Vec::new(),
                span: self.prev_token.span.shrink_to_hi(),
            });
        }
        let params = self.parse_delimited(TokenKind::Gt, |this| {
            Ok(GenericParam {
                ident: this.parse_ident()?,
            })
        })?;
        Ok(Generics {
            params,
            span: lo.to(self.prev_token.span),
        })
    }

    /// The body of a struct: `{ fields }`, `(fields);` or `;`.
    fn parse_struct_data(&mut self) -> PResult<VariantData> {
        if self.eat(TokenKind::LBrace) {
            Ok(VariantData::Struct(self.parse_record_fields()?))
        } else if self.eat(TokenKind::LParen) {
            let fields = self.parse_tuple_fields()?;
            self.expect(TokenKind::Semi)?;
            Ok(VariantData::Tuple(fields))
        } else if self.eat(TokenKind::Semi) {
            Ok(VariantData::Unit)
        } else {
            Err(self.unexpected())
        }
    }

    /// Named fields up to the closing `}`. Unlike Rust, separating commas
    /// are optional: fields may be listed one per line.
    fn parse_record_fields(&mut self) -> PResult<Vec<FieldDef>> {
        let mut fields = Vec::new();
        while !self.eat(TokenKind::RBrace) {
            let lo = self.token.span;
            let attrs = self.parse_outer_attrs()?;
            let vis = self.parse_vis();
            let ident = self.parse_ident()?;
            self.expect(TokenKind::Colon)?;
            let ty = self.parse_ty()?;
            fields.push(FieldDef {
                attrs,
                vis,
                ident: Some(ident),
                ty,
                span: lo.to(self.prev_token.span),
            });
            self.eat(TokenKind::Comma);
        }
        Ok(fields)
    }

    /// Unnamed fields up to the closing `)`.
    fn parse_tuple_fields(&mut self) -> PResult<Vec<FieldDef>> {
        self.parse_delimited(TokenKind::RParen, |this| {
            let lo = this.token.span;
            let attrs = this.parse_outer_attrs()?;
            let vis = this.parse_vis();
            let ty = this.parse_ty()?;
            Ok(FieldDef {
                attrs,
                vis,
                ident: None,
                ty,
                span: lo.to(this.prev_token.span),
            })
        })
    }

    /// `{ variants }`. As with struct fields, separating commas are optional.
    fn parse_enum_def(&mut self) -> PResult<EnumDef> {
        self.expect(TokenKind::LBrace)?;
        let mut variants = Vec::new();
        while !self.eat(TokenKind::RBrace) {
            variants.push(self.parse_variant()?);
            self.eat(TokenKind::Comma);
        }
        Ok(EnumDef { variants })
    }

    fn parse_variant(&mut self) -> PResult<Variant> {
        let lo = self.token.span;
        let attrs = self.parse_outer_attrs()?;
        let ident = self.parse_ident()?;
        let data = if self.eat(TokenKind::LParen) {
            VariantData::Tuple(self.parse_tuple_fields()?)
        } else if self.eat(TokenKind::LBrace) {
            VariantData::Struct(self.parse_record_fields()?)
        } else {
            VariantData::Unit
        };
        Ok(Variant {
            attrs,
            ident,
            data,
            span: lo.to(self.prev_token.span),
        })
    }
}
