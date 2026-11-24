//! `std::decimal` — arbitrary-precision decimal numbers backed by `rust_decimal`.

use facet::Facet;
use rust_decimal::Decimal;
use xur_runtime::Registry;

/// Wrapper so we can derive Facet and register as a native type.
#[derive(Clone, Copy, Facet)]
#[facet(opaque)]
pub struct XurDecimal(pub Decimal);

impl std::fmt::Display for XurDecimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl XurDecimal {
    /// Pack into the .NET `System.Decimal` memory layout as two u64 register
    /// values (System V AMD64 ABI).
    ///
    /// .NET layout: `[flags:u32, hi:u32, lo:u32, mid:u32]`
    /// Register 1 (rdi/rsi): `flags | hi << 32`
    /// Register 2 (rsi/rdx): `lo | mid << 32`
    pub fn to_dotnet_regs(&self) -> [u64; 2] {
        let bytes = self.0.serialize();
        // serialize() = [flags(4), lo(4), mid(4), hi(4)]
        let flags = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let lo = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let mid = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let hi = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        // .NET memory order: [flags, hi, lo, mid]
        let reg0 = (flags as u64) | ((hi as u64) << 32);
        let reg1 = (lo as u64) | ((mid as u64) << 32);
        [reg0, reg1]
    }

    /// Unpack from two u64 register values in the .NET `System.Decimal` ABI layout.
    pub fn from_dotnet_regs(regs: [u64; 2]) -> Self {
        let flags = regs[0] as u32;
        let hi = (regs[0] >> 32) as u32;
        let lo = regs[1] as u32;
        let mid = (regs[1] >> 32) as u32;
        // serialize() format = [flags, lo, mid, hi]
        let mut bytes = [0u8; 16];
        bytes[0..4].copy_from_slice(&flags.to_le_bytes());
        bytes[4..8].copy_from_slice(&lo.to_le_bytes());
        bytes[8..12].copy_from_slice(&mid.to_le_bytes());
        bytes[12..16].copy_from_slice(&hi.to_le_bytes());
        XurDecimal(Decimal::deserialize(bytes))
    }
}

pub(crate) fn register(registry: &Registry) {
    let mut m = registry.module("std::decimal");

    m.ty::<XurDecimal>("Decimal")
        .method("scale", |d: &XurDecimal| -> i64 { d.0.scale() as i64 })
        .method("is_sign_negative", |d: &XurDecimal| -> bool {
            d.0.is_sign_negative()
        })
        .method("is_sign_positive", |d: &XurDecimal| -> bool {
            d.0.is_sign_positive()
        })
        .method("abs", |d: &XurDecimal| -> XurDecimal {
            XurDecimal(d.0.abs())
        })
        .method("floor", |d: &XurDecimal| -> XurDecimal {
            XurDecimal(d.0.floor())
        })
        .method("ceil", |d: &XurDecimal| -> XurDecimal {
            XurDecimal(d.0.ceil())
        })
        .method("round", |d: &XurDecimal| -> XurDecimal {
            XurDecimal(d.0.round())
        })
        .method("round_dp", |d: &XurDecimal, dp: i64| -> XurDecimal {
            XurDecimal(d.0.round_dp(dp as u32))
        })
        .method("to_int", |d: &XurDecimal| -> Result<i64, String> {
            use rust_decimal::prelude::ToPrimitive;
            d.0.to_i64().ok_or_else(|| format!("{} does not fit in i64", d.0))
        })
        .method("to_float", |d: &XurDecimal| -> Result<f64, String> {
            use rust_decimal::prelude::ToPrimitive;
            d.0.to_f64().ok_or_else(|| format!("{} does not fit in f64", d.0))
        });

    m.named_function("from_int", &["value"], |value: i64| -> XurDecimal {
        XurDecimal(Decimal::from(value))
    });
    m.named_function("from_float", &["value"], |value: f64| -> Result<XurDecimal, String> {
        Decimal::try_from(value)
            .map(XurDecimal)
            .map_err(|e| e.to_string())
    });
    m.named_function("from_str", &["value"], |value: String| -> Result<XurDecimal, String> {
        value
            .parse::<Decimal>()
            .map(XurDecimal)
            .map_err(|e| e.to_string())
    });
}
