//! `std::reflect::Layout`: where the fields of a struct live in memory, for
//! reading and writing binary data.
//!
//! A layout comes from a Xur struct's attributes, `reflect::layout::<T>()`:
//!
//! ```text
//! #[repr(C)]              // C's alignment and padding (also the default)
//! #[repr(packed)]         // no padding, alignment 1
//! #[repr(align(8))]       // at least this alignment
//! #[endian(Big)]          // byte order of the fields (`Little` by default)
//! struct Header {
//!     magic: u32
//!     #[field(offset = 8)] len: u16   // placed at this offset
//!     #[field(size = 16)] name: bytes // a fixed number of raw bytes
//!     inner: Other                    // another struct, laid out inside
//! }
//! ```
//!
//! or is built at runtime, each method returning a new layout:
//!
//! ```text
//! Layout::new("Header").field("magic", "u32").field_at("len", "u16", 8).big_endian()
//! ```

use std::sync::Arc;

use facet::Facet;
use xur_ast::{Attribute, LitKind, MetaItem, MetaItemInner, MetaItemKind};
use xur_resolve::DefId;
use xur_runtime::convert::into_value;
use xur_runtime::{
    AdtInfo, Context, Record, RunResult, RuntimeError, TypeRef, Value, lock,
};
use xur_span::Symbol;

/// A struct's layout. Layouts never change once built, so they are shared.
#[derive(Facet, Clone)]
#[facet(opaque)]
pub(crate) struct Layout(Arc<LayoutData>);

#[derive(Clone)]
struct LayoutData {
    name: String,
    /// The Xur struct it was made from: reading gives one of those.
    /// Otherwise reading gives a record.
    ctor: Option<DefId>,
    fields: Vec<Field>,
    big_endian: bool,
    packed: bool,
    /// From `align(n)`.
    min_align: usize,
    size: usize,
    align: usize,
}

#[derive(Clone)]
struct Field {
    name: String,
    kind: Kind,
    /// From `#[field(offset = n)]`.
    at: Option<usize>,
    offset: usize,
}

#[derive(Clone)]
enum Kind {
    Int { size: usize, signed: bool },
    Float { size: usize },
    Bool,
    Bytes(usize),
    Struct(Layout),
}

impl Kind {
    /// A primitive type by name: `u32`, `f64`, `bool`.
    fn primitive(name: &str) -> Option<Self> {
        let int = |size, signed| Some(Self::Int { size, signed });
        match name {
            "u8" => int(1, false),
            "u16" => int(2, false),
            "u32" => int(4, false),
            "u64" | "usize" => int(8, false),
            "i8" => int(1, true),
            "i16" => int(2, true),
            "i32" => int(4, true),
            "i64" | "isize" => int(8, true),
            "f32" => Some(Self::Float { size: 4 }),
            "f64" => Some(Self::Float { size: 8 }),
            "bool" => Some(Self::Bool),
            _ => None,
        }
    }

    fn size(&self) -> usize {
        match self {
            Self::Int { size, .. } | Self::Float { size } | Self::Bytes(size) => *size,
            Self::Bool => 1,
            Self::Struct(layout) => layout.0.size,
        }
    }

    fn align(&self) -> usize {
        match self {
            Self::Int { size, .. } | Self::Float { size } => *size,
            Self::Bool | Self::Bytes(_) => 1,
            Self::Struct(layout) => layout.0.align,
        }
    }

    fn name(&self) -> String {
        match self {
            &Self::Int { size, signed } => {
                format!("{}{}", if signed { 'i' } else { 'u' }, size * 8)
            }
            Self::Float { size } => format!("f{}", size * 8),
            Self::Bool => "bool".to_owned(),
            Self::Bytes(len) => format!("bytes[{len}]"),
            Self::Struct(layout) => layout.0.name.clone(),
        }
    }
}

impl LayoutData {
    fn empty(name: String, ctor: Option<DefId>) -> Self {
        Self {
            name,
            ctor,
            fields: Vec::new(),
            big_endian: false,
            packed: false,
            min_align: 1,
            size: 0,
            align: 1,
        }
    }

    /// Places every field and works out the size and alignment: in order,
    /// each at the next offset aligned for it (or at its explicit offset),
    /// with the size rounded up to the alignment, as in C.
    fn compute(mut self) -> RunResult<Layout> {
        let mut end = 0;
        let mut align = 1;
        for field in &mut self.fields {
            let field_align = if self.packed { 1 } else { field.kind.align() };
            field.offset = match field.at {
                Some(at) if at < end => {
                    return Err(RuntimeError::new(format!(
                        "field `{}` at offset {at} overlaps the field before it, which ends at {end}",
                        field.name
                    )));
                }
                Some(at) => at,
                None => end.next_multiple_of(field_align),
            };
            end = field.offset + field.kind.size();
            align = align.max(field_align);
        }
        self.align = align.max(self.min_align);
        self.size = end.next_multiple_of(self.align);
        Ok(Layout(Arc::new(self)))
    }
}

impl Layout {
    pub(crate) fn byte_size(&self) -> usize {
        self.0.size
    }

    pub(crate) fn alignment(&self) -> usize {
        self.0.align
    }

    /// A copy of the layout's description, to build a changed layout from.
    fn data(&self) -> LayoutData {
        LayoutData::clone(&self.0)
    }

    fn field(&self, name: &str) -> RunResult<&Field> {
        self.0
            .fields
            .iter()
            .find(|field| field.name == name)
            .ok_or_else(|| RuntimeError::new(format!("`{}` has no field `{name}`", self.0.name)))
    }

    /// The value at `offset` in `data`: a struct if the layout came from
    /// one, or else a record.
    pub(crate) fn read(&self, data: &[u8], offset: usize) -> RunResult<Value> {
        let end = offset + self.0.size;
        if end > data.len() {
            return Err(RuntimeError::new(format!(
                "`{}` needs {} byte(s) at offset {offset}, but there are only {}",
                self.0.name,
                self.0.size,
                data.len()
            )));
        }
        let values = self
            .0
            .fields
            .iter()
            .map(|field| self.read_field(field, data, offset + field.offset))
            .collect::<RunResult<Vec<_>>>()?;
        Ok(match self.0.ctor {
            Some(ctor) => Value::adt(ctor, values),
            None => {
                let names = self
                    .0
                    .fields
                    .iter()
                    .map(|field| Symbol::intern(&field.name));
                Value::Record(Record::new(names.zip(values).collect()).into())
            }
        })
    }

    fn read_field(&self, field: &Field, data: &[u8], at: usize) -> RunResult<Value> {
        let raw = &data[at..at + field.kind.size()];
        Ok(match &field.kind {
            Kind::Int { signed, .. } => {
                let mut buf = [0u8; 8];
                let value = if self.0.big_endian {
                    buf[8 - raw.len()..].copy_from_slice(raw);
                    let value = u64::from_be_bytes(buf);
                    sign_extend(value, raw.len(), *signed)
                } else {
                    buf[..raw.len()].copy_from_slice(raw);
                    sign_extend(u64::from_le_bytes(buf), raw.len(), *signed)
                };
                Value::Int(value.ok_or_else(|| {
                    RuntimeError::new(format!(
                        "field `{}` does not fit in a Xur integer",
                        field.name
                    ))
                })?)
            }
            Kind::Float { size } => {
                let mut raw = raw.to_vec();
                if self.0.big_endian {
                    raw.reverse();
                }
                Value::Float(match size {
                    4 => f32::from_le_bytes(raw.try_into().unwrap_or_default()).into(),
                    _ => f64::from_le_bytes(raw.try_into().unwrap_or_default()),
                })
            }
            Kind::Bool => Value::Bool(raw[0] != 0),
            Kind::Bytes(_) => Value::Bytes(Arc::new(raw.to_vec().into())),
            Kind::Struct(layout) => layout.read(data, at)?,
        })
    }

    /// Writes the fields of `value` at `offset` in `data`, leaving padding
    /// as it was.
    pub(crate) fn write(
        &self,
        cx: &mut dyn Context,
        data: &mut Vec<u8>,
        offset: usize,
        value: &Value,
    ) -> RunResult<()> {
        let end = offset + self.0.size;
        if end > data.len() {
            return Err(RuntimeError::new(format!(
                "`{}` needs {} byte(s) at offset {offset}, but there are only {}",
                self.0.name,
                self.0.size,
                data.len()
            )));
        }
        for field in &self.0.fields {
            let field_value = cx.field(value, &field.name)?;
            let at = offset + field.offset;
            let size = field.kind.size();
            let raw = match (&field.kind, &field_value) {
                (Kind::Int { size, signed }, Value::Int(i)) => {
                    if !int_fits(*i, *size, *signed) {
                        return Err(RuntimeError::new(format!(
                            "{i} does not fit in field `{}` ({})",
                            field.name,
                            field.kind.name()
                        )));
                    }
                    let bytes = i.to_le_bytes();
                    self.ordered(&bytes[..*size])
                }
                (Kind::Float { size: 4 }, &Value::Float(f)) => {
                    self.ordered(&(f as f32).to_le_bytes())
                }
                (Kind::Float { .. }, &Value::Float(f)) => self.ordered(&f.to_le_bytes()),
                (Kind::Bool, &Value::Bool(b)) => vec![b.into()],
                (Kind::Bytes(len), Value::Bytes(bytes)) => {
                    let mut bytes = lock(bytes).clone();
                    if bytes.len() > *len {
                        return Err(RuntimeError::new(format!(
                            "field `{}` holds {len} byte(s), not {}",
                            field.name,
                            bytes.len()
                        )));
                    }
                    bytes.resize(*len, 0);
                    bytes
                }
                (Kind::Struct(layout), inner) => {
                    layout.write(cx, data, at, inner)?;
                    continue;
                }
                (kind, other) => {
                    return Err(RuntimeError::new(format!(
                        "field `{}` is a {}, but the value is {}",
                        field.name,
                        kind.name(),
                        cx.describe_type(other)
                    )));
                }
            };
            data[at..at + size].copy_from_slice(&raw);
        }
        Ok(())
    }

    /// Little-endian bytes in the layout's byte order.
    fn ordered(&self, little: &[u8]) -> Vec<u8> {
        let mut bytes = little.to_vec();
        if self.0.big_endian {
            bytes.reverse();
        }
        bytes
    }

    /// The fields as records: `{ name, ty, offset, size }`.
    fn field_records(&self) -> Value {
        let fields = self
            .0
            .fields
            .iter()
            .map(|field| {
                let entries = [
                    ("name", Value::str(&field.name)),
                    ("ty", Value::str(&field.kind.name())),
                    ("offset", Value::Int(field.offset as i64)),
                    ("size", Value::Int(field.kind.size() as i64)),
                ];
                let fields = entries
                    .into_iter()
                    .map(|(name, value)| (Symbol::intern(name), value))
                    .collect();
                Value::Record(Record::new(fields).into())
            })
            .collect();
        Value::list(fields)
    }
}

impl std::fmt::Display for Layout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let data = &self.0;
        write!(
            f,
            "{} (size {}, align {}) {{",
            data.name, data.size, data.align
        )?;
        for (i, field) in data.fields.iter().enumerate() {
            let sep = if i == 0 { " " } else { ", " };
            write!(
                f,
                "{sep}{}: {} @ {}",
                field.name,
                field.kind.name(),
                field.offset
            )?;
        }
        write!(f, " }}")
    }
}

/// `value`, the low `size` bytes of an integer, as an `i64`.
fn sign_extend(value: u64, size: usize, signed: bool) -> Option<i64> {
    let bits = size as u32 * 8;
    if signed && bits < 64 {
        let shift = 64 - bits;
        Some(((value << shift) as i64) >> shift)
    } else if signed {
        Some(value as i64)
    } else {
        i64::try_from(value).ok()
    }
}

fn int_fits(i: i64, size: usize, signed: bool) -> bool {
    let bits = size as u32 * 8;
    match (signed, bits) {
        (true, 64) => true,
        (true, _) => (-(1i64 << (bits - 1))..1i64 << (bits - 1)).contains(&i),
        (false, 64) => i >= 0,
        (false, _) => (0..1i64 << bits).contains(&i),
    }
}

// Layouts of Xur structs.

/// The layout of the struct `def`, from its attributes.
pub(crate) fn layout_of_def(cx: &dyn Context, def: DefId, depth: usize) -> RunResult<Layout> {
    let Some(info) = cx.adt_info(def) else {
        return Err(RuntimeError::new(
            "only structs and enum variants have layouts",
        ));
    };
    layout_of_adt(cx, &info, depth)
}

fn layout_of_adt(cx: &dyn Context, info: &AdtInfo, depth: usize) -> RunResult<Layout> {
    if depth > 32 {
        return Err(RuntimeError::new(format!(
            "`{}` contains itself",
            info.name
        )));
    }
    let mut data = LayoutData::empty(info.name.clone(), Some(info.ctor));
    for attr in info.attrs {
        apply_struct_attr(&mut data, attr)?;
    }
    for (name, field) in info.field_names().into_iter().zip(&info.fields) {
        let mut at = None;
        let mut size = None;
        for attr in field.attrs {
            if attr_name(&attr.meta) == Some("field") {
                for (key, value) in name_values(&attr.meta)? {
                    match key.as_str() {
                        "offset" => at = Some(value),
                        "size" => size = Some(value),
                        _ => return Err(attr_error(attr, "expected `offset = n` or `size = n`")),
                    }
                }
            }
        }
        let kind = field_kind(cx, &field.ty, size, depth)
            .map_err(|error| RuntimeError::new(format!("field `{name}`: {}", error.message)))?;
        data.fields.push(Field {
            name,
            kind,
            at,
            offset: 0,
        });
    }
    data.compute()
}

fn field_kind(
    cx: &dyn Context,
    ty: &TypeRef,
    size: Option<usize>,
    depth: usize,
) -> RunResult<Kind> {
    if ty.name == "bytes" {
        return size
            .map(Kind::Bytes)
            .ok_or_else(|| RuntimeError::new("`bytes` fields need a size: `#[field(size = n)]`"));
    }
    if let Some(kind) = Kind::primitive(&ty.name) {
        return Ok(kind);
    }
    match ty.def {
        Some(def) => Ok(Kind::Struct(layout_of_def(cx, def, depth + 1)?)),
        None => Err(RuntimeError::new(format!("`{}` has no layout", ty.name))),
    }
}

/// `#[repr(..)]` and `#[endian(..)]`; other attributes are ignored.
fn apply_struct_attr(data: &mut LayoutData, attr: &Attribute) -> RunResult<()> {
    let meta = &attr.meta;
    match attr_name(meta) {
        Some("repr") => {
            for item in list(meta) {
                match (attr_name(item), &item.kind) {
                    (Some("C" | "Rust"), MetaItemKind::Word) => {}
                    (Some("packed"), MetaItemKind::Word) => data.packed = true,
                    (Some("align"), MetaItemKind::List(args)) => {
                        let align = match args.as_slice() {
                            [MetaItemInner::Lit(lit)] => match lit.kind {
                                LitKind::Int(n) if n.is_power_of_two() => n as usize,
                                _ => return Err(attr_error(attr, "alignments are powers of two")),
                            },
                            _ => return Err(attr_error(attr, "expected `align(n)`")),
                        };
                        data.min_align = data.min_align.max(align);
                    }
                    _ => return Err(attr_error(attr, "expected `C`, `packed` or `align(n)`")),
                }
            }
        }
        Some("endian") => {
            for item in list(meta) {
                data.big_endian = match attr_name(item) {
                    Some("Little") => false,
                    Some("Big") => true,
                    _ => return Err(attr_error(attr, "expected `Little` or `Big`")),
                };
            }
        }
        _ => {}
    }
    Ok(())
}

fn attr_name(meta: &MetaItem) -> Option<&'static str> {
    meta.path.as_ident().map(|ident| ident.name.as_str())
}

/// The items of `name(a, b)`.
fn list(meta: &MetaItem) -> impl Iterator<Item = &MetaItem> {
    let items = match &meta.kind {
        MetaItemKind::List(items) => items.as_slice(),
        _ => &[],
    };
    items.iter().filter_map(|item| match item {
        MetaItemInner::MetaItem(meta) => Some(meta),
        MetaItemInner::Lit(_) => None,
    })
}

/// The `key = n` items of `name(key = n, ...)`.
fn name_values(meta: &MetaItem) -> RunResult<Vec<(String, usize)>> {
    list(meta)
        .map(|item| match (&item.kind, attr_name(item)) {
            (MetaItemKind::NameValue(lit), Some(key)) => match lit.kind {
                LitKind::Int(n) => Ok((key.to_owned(), n as usize)),
                _ => Err(RuntimeError::new(format!("`{key}` must be a number"))),
            },
            _ => Err(RuntimeError::new("expected `name = value`")),
        })
        .collect()
}

fn attr_error(attr: &Attribute, expected: &str) -> RuntimeError {
    let name = attr_name(&attr.meta).unwrap_or("attribute");
    RuntimeError::new(format!("bad `#[{name}(..)]`: {expected}")).at(attr.span)
}

// Registration.

pub(super) fn register(module: &mut xur_runtime::registry::ModuleBuilder<'_>) {
    module
        .raw_function("layout", |cx, args| {
            let ([ty], []) = (cx.type_args(), args) else {
                return Err(RuntimeError::new(
                    "`layout` takes a type argument: `layout::<T>()`",
                ));
            };
            let Some(def) = ty.def else {
                return Err(RuntimeError::new(format!("`{}` has no layout", ty.name)));
            };
            let layout = layout_of_def(cx, def, 0)?;
            into_value(cx, layout)
        })
        .raw_function("layout_of", |cx, args| {
            let [Value::Adt(adt)] = args else {
                return Err(RuntimeError::new("`layout_of` takes a struct value"));
            };
            let layout = layout_of_def(cx, adt.ctor, 0)?;
            into_value(cx, layout)
        });
    module
        .ty::<Layout>("Layout")
        .function("new", |name: String| {
            Layout(Arc::new(LayoutData::empty(name, None)))
        })
        .method("name", |l: &Layout| l.0.name.clone())
        .method("size", |l: &Layout| l.0.size)
        .method("align", |l: &Layout| l.0.align)
        .method("is_big_endian", |l: &Layout| l.0.big_endian)
        .method("to_string", |l: &Layout| l.to_string())
        .raw_function("fields", |_, args| Ok(layout_arg(args)?.field_records()))
        .raw_function("offset_of", |_, args| {
            let [_, Value::Str(name)] = args else {
                return Err(RuntimeError::new("`offset_of` takes a field name"));
            };
            Ok(Value::Int(layout_arg(args)?.field(name)?.offset as i64))
        })
        .raw_function("field", |_, args| {
            let [_, Value::Str(name), Value::Str(ty)] = args else {
                return Err(RuntimeError::new(
                    "`field` takes a name and a type: `field(\"len\", \"u16\")`",
                ));
            };
            let kind = Kind::primitive(ty)
                .ok_or_else(|| RuntimeError::new(format!("`{ty}` is not a primitive type")))?;
            add_field(args, name, kind, None)
        })
        .raw_function("field_at", |_, args| {
            let [_, Value::Str(name), Value::Str(ty), Value::Int(at)] = args else {
                return Err(RuntimeError::new(
                    "`field_at` takes a name, a type and an offset",
                ));
            };
            let kind = Kind::primitive(ty)
                .ok_or_else(|| RuntimeError::new(format!("`{ty}` is not a primitive type")))?;
            add_field(args, name, kind, Some(usize_arg(*at)?))
        })
        .raw_function("bytes", |_, args| {
            let [_, Value::Str(name), Value::Int(len)] = args else {
                return Err(RuntimeError::new("`bytes` takes a name and a length"));
            };
            add_field(args, name, Kind::Bytes(usize_arg(*len)?), None)
        })
        .raw_function("nested", |_, args| {
            let [_, Value::Str(name), inner] = args else {
                return Err(RuntimeError::new("`nested` takes a name and a layout"));
            };
            let inner = layout_arg(std::slice::from_ref(inner))?;
            add_field(args, name, Kind::Struct(inner), None)
        })
        .raw_function("packed", |_, args| options(args, |data| data.packed = true))
        .raw_function("big_endian", |_, args| {
            options(args, |data| data.big_endian = true)
        })
        .raw_function("little_endian", |_, args| {
            options(args, |data| data.big_endian = false)
        })
        .raw_function("aligned", |_, args| {
            let [_, Value::Int(align)] = args else {
                return Err(RuntimeError::new("`aligned` takes an alignment"));
            };
            let align = usize_arg(*align)?;
            if !align.is_power_of_two() {
                return Err(RuntimeError::new("alignments are powers of two"));
            }
            options(args, |data| data.min_align = align)
        })
        .raw_function("read", |_, args| {
            let [_, Value::Bytes(data), Value::Int(offset)] = args else {
                return Err(RuntimeError::new("`read` takes bytes and an offset"));
            };
            layout_arg(args)?.read(&lock(data), usize_arg(*offset)?)
        })
        .raw_function("from_bytes", |_, args| {
            let [_, Value::Bytes(data)] = args else {
                return Err(RuntimeError::new("`from_bytes` takes bytes"));
            };
            layout_arg(args)?.read(&lock(data), 0)
        })
        .raw_function("write", |cx, args| {
            let [_, Value::Bytes(data), Value::Int(offset), value] = args else {
                return Err(RuntimeError::new(
                    "`write` takes bytes, an offset and a value",
                ));
            };
            let layout = layout_arg(args)?;
            // Write into a copy, so the value's fields can be read while
            // the buffer is not locked, and a failed write changes nothing.
            let mut buffer = lock(data).clone();
            layout.write(cx, &mut buffer, usize_arg(*offset)?, value)?;
            *lock(data) = buffer;
            Ok(Value::Unit)
        })
        .raw_function("to_bytes", |cx, args| {
            let [_, value] = args else {
                return Err(RuntimeError::new("`to_bytes` takes a value"));
            };
            let layout = layout_arg(args)?;
            let mut buffer = vec![0; layout.0.size];
            layout.write(cx, &mut buffer, 0, value)?;
            Ok(Value::Bytes(Arc::new(buffer.into())))
        });
}

/// The layout of a function's one type argument, which takes no other
/// arguments than `args`.
pub(crate) fn type_arg_layout(cx: &dyn Context, args: &[Value], name: &str) -> RunResult<Layout> {
    let ([ty], []) = (cx.type_args(), args) else {
        return Err(RuntimeError::new(format!(
            "`{name}` takes a type argument: `{name}::<T>()`"
        )));
    };
    let Some(def) = ty.def else {
        return Err(RuntimeError::new(format!("`{}` has no layout", ty.name)));
    };
    layout_of_def(cx, def, 0)
}

fn layout_arg(args: &[Value]) -> RunResult<Layout> {
    match args.first() {
        Some(Value::Native(object)) => object.with(|layout: &Layout| layout.clone()),
        _ => None,
    }
    .ok_or_else(|| RuntimeError::new("expected a `Layout`"))
}

fn usize_arg(i: i64) -> RunResult<usize> {
    usize::try_from(i).map_err(|_| RuntimeError::new(format!("{i} is not a valid size or offset")))
}

/// A copy of the layout with `name` added.
fn add_field(args: &[Value], name: &str, kind: Kind, at: Option<usize>) -> RunResult<Value> {
    let layout = layout_arg(args)?;
    if layout.0.fields.iter().any(|field| field.name == name) {
        return Err(RuntimeError::new(format!(
            "`{}` already has a field `{name}`",
            layout.0.name
        )));
    }
    let mut data = layout.data();
    data.fields.push(Field {
        name: name.to_owned(),
        kind,
        at,
        offset: 0,
    });
    native(&args[0], data.compute()?)
}

/// A copy of the layout with its options changed.
fn options(args: &[Value], change: impl FnOnce(&mut LayoutData)) -> RunResult<Value> {
    let mut data = layout_arg(args)?.data();
    change(&mut data);
    native(&args[0], data.compute()?)
}

/// A new native object of the same type as `like`.
fn native(like: &Value, layout: Layout) -> RunResult<Value> {
    let Value::Native(object) = like else {
        return Err(RuntimeError::new("expected a `Layout`"));
    };
    Ok(Value::Native(Arc::new(xur_runtime::NativeObject::new(
        object.ty,
        Box::new(layout),
    ))))
}
