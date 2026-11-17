// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Codec of the generated `no_std` types, through `hdds_micro`'s
//! `CdrEncoder` / `CdrDecoder`.
//!
//! Those cursors align every primitive to its size from the first byte of
//! their buffer, 8-byte values at 8: the layout of XCDR1 (`PLAIN_CDR`,
//! DDS-`XTypes` 1.3 7.4.1, `MAXALIGN(VERSION1) = 8`). The generated code
//! therefore serializes XCDR1, the body alone: the encapsulation header
//! (`CDR_LE`) is the caller's, written outside the encoder so that the
//! alignment origin stays the first byte of the body.
//!
//! A FINAL structure is its members in order, base members first; an
//! APPENDABLE one is serialized as FINAL under XCDR1 (rule 29); a MUTABLE
//! one is a parameter list (rules 23, 25) and an `@optional` member of a
//! FINAL one a parameter (rule 19): each value is first serialized into a
//! scratch encoder (its own alignment origin, as large as the value's
//! maximum) so that its exact length can precede it (7.4.1.2.2); a MUTABLE
//! union is its discriminator and its case as parameters (rule 28). Maps,
//! bitmasks, bitsets and `fixed` are refused.

#![allow(clippy::uninlined_format_args)]

use std::fmt::Write as _;

use super::super::data_model::{
    self, enum_width, flatten_array, union_case_disc, union_ext, union_label_value, Ext,
};
use super::super::key_holder::{Named, TypeIndex};
use super::super::keywords::rust_ident;
use super::MicroConfig;
use crate::ast::{Struct, Union, UnionLabel};
use crate::types::{IdlType, PrimitiveType};

/// Code emitter for one type.
pub(super) struct MGen<'a> {
    idx: &'a TypeIndex,
    cfg: &'a MicroConfig,
    out: String,
    var: usize,
}

/// A place used as a whole value: `(*x)` becomes `*x`.
fn as_value(place: &str) -> String {
    place
        .strip_prefix('(')
        .and_then(|r| r.strip_suffix(')'))
        .filter(|inner| inner.starts_with('*') && !inner.contains(['(', ')']))
        .map_or_else(|| place.to_string(), str::to_string)
}

impl<'a> MGen<'a> {
    pub(super) const fn new(idx: &'a TypeIndex, cfg: &'a MicroConfig) -> Self {
        Self {
            idx,
            cfg,
            out: String::new(),
            var: 0,
        }
    }

    fn fresh(&mut self, prefix: &str) -> String {
        let v = format!("__{prefix}{}", self.var);
        self.var += 1;
        v
    }

    fn line(&mut self, ind: usize, text: &str) {
        for _ in 0..ind {
            self.out.push_str("    ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn seq_cap(&self, bound: Option<u32>) -> usize {
        bound.map_or(self.cfg.max_sequence_len, |b| b as usize)
    }

    /// Why `ty` cannot be serialized by this backend, if it cannot.
    pub(super) fn unsupported(&self, ty: &IdlType) -> Option<String> {
        match self.idx.resolve(ty) {
            IdlType::Primitive(PrimitiveType::Fixed { .. }) => Some("`fixed` is not mapped".into()),
            IdlType::Primitive(_) | IdlType::BoundedString { .. } => None,
            IdlType::Map { .. } => Some("a map needs dynamic allocation".into()),
            IdlType::Sequence { inner, .. } | IdlType::Array { inner, .. } => {
                self.unsupported(&inner)
            }
            IdlType::Named(n) => match self.idx.named(&n) {
                Some(Named::Struct(_) | Named::Union(_) | Named::Enum { .. }) => None,
                Some(Named::Bitmask { .. }) => Some(format!("bitmask `{n}` is not mapped")),
                Some(Named::Bitset) => Some(format!("bitset `{n}` is not mapped")),
                Some(Named::Typedef(_)) | None => Some(format!("type `{n}` is unknown")),
            },
        }
    }

    /// Rust type of `ty` (typedefs resolved).
    pub(super) fn rust_type(&self, ty: &IdlType) -> String {
        match self.idx.resolve(ty) {
            IdlType::Primitive(p) => self.prim_type(p),
            IdlType::Named(n) => crate::codegen::rust_paths::path(&n),
            IdlType::Array { inner, size } => format!("[{}; {size}]", self.rust_type(&inner)),
            IdlType::BoundedString { wide: false, bound } => {
                format!("heapless::String<{bound}>")
            }
            // `wstring<N>`: code points (an unbounded `wstring` is a
            // `heapless::String`).
            IdlType::BoundedString { wide: true, bound } => format!("heapless::Vec<u32, {bound}>"),
            IdlType::Sequence { inner, bound } => {
                format!(
                    "heapless::Vec<{}, {}>",
                    self.rust_type(&inner),
                    self.seq_cap(bound)
                )
            }
            IdlType::Map { .. } => "()".to_string(),
        }
    }

    fn prim_type(&self, p: PrimitiveType) -> String {
        match p {
            PrimitiveType::Octet | PrimitiveType::UInt8 | PrimitiveType::Char => "u8".into(),
            PrimitiveType::Int8 => "i8".into(),
            PrimitiveType::Boolean => "bool".into(),
            PrimitiveType::Short | PrimitiveType::Int16 => "i16".into(),
            PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => "u16".into(),
            PrimitiveType::Long | PrimitiveType::Int32 => "i32".into(),
            PrimitiveType::UnsignedLong | PrimitiveType::UInt32 | PrimitiveType::WChar => {
                "u32".into()
            }
            PrimitiveType::LongLong | PrimitiveType::Int64 => "i64".into(),
            PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => "u64".into(),
            PrimitiveType::Float => "f32".into(),
            PrimitiveType::Double | PrimitiveType::LongDouble => "f64".into(),
            PrimitiveType::String | PrimitiveType::WString => {
                format!("heapless::String<{}>", self.cfg.max_string_len)
            }
            PrimitiveType::Void => "()".into(),
            PrimitiveType::Fixed { .. } => "i128".into(),
        }
    }

    // -------------------------------------------------------------------
    // Encoding: statements writing the value at `place` through `enc`.
    // -------------------------------------------------------------------

    pub(super) fn enc(&mut self, ind: usize, place: &str, ty: &IdlType) {
        let value = as_value(place);
        match self.idx.resolve(ty) {
            IdlType::Primitive(p) => {
                let stmt = match p {
                    PrimitiveType::Octet | PrimitiveType::UInt8 | PrimitiveType::Char => {
                        format!("enc.encode_u8({value})?;")
                    }
                    PrimitiveType::Int8 => format!("enc.encode_i8({value})?;"),
                    PrimitiveType::Boolean => format!("enc.encode_bool({value})?;"),
                    PrimitiveType::Short | PrimitiveType::Int16 => {
                        format!("enc.encode_i16({value})?;")
                    }
                    PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => {
                        format!("enc.encode_u16({value})?;")
                    }
                    PrimitiveType::Long | PrimitiveType::Int32 => {
                        format!("enc.encode_i32({value})?;")
                    }
                    PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => {
                        format!("enc.encode_u32({value})?;")
                    }
                    PrimitiveType::LongLong | PrimitiveType::Int64 => {
                        format!("enc.encode_i64({value})?;")
                    }
                    PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => {
                        format!("enc.encode_u64({value})?;")
                    }
                    PrimitiveType::Float => format!("enc.encode_f32({value})?;"),
                    PrimitiveType::Double | PrimitiveType::LongDouble => {
                        format!("enc.encode_f64({value})?;")
                    }
                    // wchar: one UTF-16 code unit (2 bytes).
                    PrimitiveType::WChar => format!(
                        "enc.encode_u16(u16::try_from({value}).map_err(|_| CdrError::EncodingError)?)?;"
                    ),
                    PrimitiveType::String => format!("enc.encode_string({place}.as_str())?;"),
                    PrimitiveType::WString => {
                        self.enc_wstring(ind, place);
                        return;
                    }
                    PrimitiveType::Void | PrimitiveType::Fixed { .. } => return,
                };
                self.line(ind, &stmt);
            }
            IdlType::BoundedString { wide: false, .. } => {
                self.line(ind, &format!("enc.encode_string({place}.as_str())?;"));
            }
            IdlType::BoundedString { wide: true, .. } => self.enc_wstring_points(ind, place),
            IdlType::Sequence { inner, .. } => {
                self.line(ind, &format!("enc.encode_seq_len({place}.len())?;"));
                if self.is_octet(&inner) {
                    self.line(ind, &format!("enc.encode_bytes(&{place})?;"));
                } else {
                    let x = self.fresh("x");
                    self.line(ind, &format!("for {x} in {place}.iter() {{"));
                    self.enc(ind + 1, &format!("(*{x})"), &inner);
                    self.line(ind, "}");
                }
            }
            IdlType::Array { .. } => {
                let Some((elem, dims)) = flatten_array(self.idx, ty) else {
                    return;
                };
                let mut cur = place.to_string();
                let mut depth = ind;
                for _ in &dims {
                    let x = self.fresh("x");
                    self.line(depth, &format!("for {x} in {cur}.iter() {{"));
                    cur = format!("(*{x})");
                    depth += 1;
                }
                self.enc(depth, &cur, &elem);
                for _ in &dims {
                    depth -= 1;
                    self.line(depth, "}");
                }
            }
            IdlType::Named(n) => match self.idx.named(&n) {
                Some(Named::Enum { bit_bound, .. }) => {
                    let call = match enum_width(*bit_bound) {
                        1 => format!("enc.encode_i8(({value} as i32) as i8)?;"),
                        2 => format!("enc.encode_i16(({value} as i32) as i16)?;"),
                        _ => format!("enc.encode_i32({value} as i32)?;"),
                    };
                    self.line(ind, &call);
                }
                _ => self.line(ind, &format!("{place}.encode(enc)?;")),
            },
            IdlType::Map { .. } => {}
        }
    }

    fn enc_wstring(&mut self, ind: usize, place: &str) {
        self.line(
            ind,
            &format!(
                "enc.encode_u32(u32::try_from({place}.encode_utf16().count() * 2).map_err(|_| CdrError::EncodingError)?)?;"
            ),
        );
        let u = self.fresh("u");
        self.line(
            ind,
            &format!("for {u} in {place}.encode_utf16() {{ enc.encode_u16({u})?; }}"),
        );
    }

    /// A `wstring<N>` held as code points: its UTF-16 byte length, then
    /// the code units.
    fn enc_wstring_points(&mut self, ind: usize, place: &str) {
        let n = self.fresh("n");
        self.line(
            ind,
            &format!(
                "let {n}: usize = {place}.iter().map(|cp| char::from_u32(*cp).map_or(1, char::len_utf16)).sum();"
            ),
        );
        self.line(
            ind,
            &format!(
                "enc.encode_u32(u32::try_from({n} * 2).map_err(|_| CdrError::EncodingError)?)?;"
            ),
        );
        let cp = self.fresh("cp");
        self.line(ind, &format!("for {cp} in {place}.iter() {{"));
        self.line(
            ind + 1,
            &format!(
                "let mut __b = [0u16; 2]; for __u in char::from_u32(*{cp}).ok_or(CdrError::EncodingError)?.encode_utf16(&mut __b) {{ enc.encode_u16(*__u)?; }}"
            ),
        );
        self.line(ind, "}");
    }

    fn is_octet(&self, ty: &IdlType) -> bool {
        matches!(
            self.idx.resolve(ty),
            IdlType::Primitive(PrimitiveType::Octet | PrimitiveType::UInt8)
        )
    }

    // -------------------------------------------------------------------
    // Decoding: an expression of the Rust type, read through `dec`.
    // -------------------------------------------------------------------

    pub(super) fn dec(&mut self, ty: &IdlType) -> String {
        match self.idx.resolve(ty) {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Octet | PrimitiveType::UInt8 | PrimitiveType::Char => {
                    "dec.decode_u8()?".into()
                }
                PrimitiveType::Int8 => "dec.decode_i8()?".into(),
                PrimitiveType::Boolean => "dec.decode_bool()?".into(),
                PrimitiveType::Short | PrimitiveType::Int16 => "dec.decode_i16()?".into(),
                PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => "dec.decode_u16()?".into(),
                PrimitiveType::Long | PrimitiveType::Int32 => "dec.decode_i32()?".into(),
                PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => "dec.decode_u32()?".into(),
                PrimitiveType::LongLong | PrimitiveType::Int64 => "dec.decode_i64()?".into(),
                PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => {
                    "dec.decode_u64()?".into()
                }
                PrimitiveType::Float => "dec.decode_f32()?".into(),
                PrimitiveType::Double | PrimitiveType::LongDouble => "dec.decode_f64()?".into(),
                PrimitiveType::WChar => "u32::from(dec.decode_u16()?)".into(),
                PrimitiveType::String => {
                    format!("dec.decode_string::<{}>()?", self.cfg.max_string_len)
                }
                PrimitiveType::WString => self.dec_wstring(self.cfg.max_string_len),
                PrimitiveType::Void | PrimitiveType::Fixed { .. } => "Default::default()".into(),
            },
            IdlType::BoundedString { wide: false, bound } => {
                format!("dec.decode_string::<{bound}>()?")
            }
            IdlType::BoundedString { wide: true, bound } => {
                let cap = bound;
                let n = self.fresh("n");
                let v = self.fresh("v");
                format!(
                        "{{ let {n} = dec.decode_u32()? as usize; if {n} % 2 != 0 {{ return Err(CdrError::DecodingError); }} \
                         let mut {v} = heapless::Vec::<u32, {cap}>::new(); \
                         for __c in core::char::decode_utf16(dec.decode_bytes({n})?.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]]))) \
                         {{ {v}.push(u32::from(__c.map_err(|_| CdrError::DecodingError)?)).map_err(|_| CdrError::BufferTooSmall)?; }} {v} }}"
                    )
            }
            IdlType::Sequence { inner, bound } => {
                let cap = self.seq_cap(bound);
                let n = self.fresh("n");
                if self.is_octet(&inner) {
                    return format!(
                            "{{ let {n} = dec.decode_seq_len()?; heapless::Vec::<u8, {cap}>::from_slice(dec.decode_bytes({n})?).map_err(|_| CdrError::BufferTooSmall)? }}"
                        );
                }
                let elem = self.dec(&inner);
                let t = self.rust_type(&inner);
                let v = self.fresh("v");
                format!(
                        "{{ let {n} = dec.decode_seq_len()?; let mut {v} = heapless::Vec::<{t}, {cap}>::new(); for _ in 0..{n} {{ {v}.push({elem}).map_err(|_| CdrError::BufferTooSmall)?; }} {v} }}"
                    )
            }
            IdlType::Array { .. } => {
                let Some((elem, dims)) = flatten_array(self.idx, ty) else {
                    return "Default::default()".into();
                };
                self.dec_array(&elem, &dims)
            }
            IdlType::Named(n) => match self.idx.named(&n) {
                Some(Named::Enum { bit_bound, .. }) => {
                    let read = match enum_width(*bit_bound) {
                        1 => "i32::from(dec.decode_i8()?)",
                        2 => "i32::from(dec.decode_i16()?)",
                        _ => "dec.decode_i32()?",
                    };
                    format!(
                        "{}::from_i32({read})?",
                        crate::codegen::rust_paths::path(&n)
                    )
                }
                _ => format!("{}::decode(dec)?", crate::codegen::rust_paths::path(&n)),
            },
            IdlType::Map { .. } => "()".into(),
        }
    }

    fn dec_wstring(&mut self, cap: usize) -> String {
        let n = self.fresh("n");
        let s = self.fresh("s");
        format!(
            "{{ let {n} = dec.decode_u32()? as usize; if {n} % 2 != 0 {{ return Err(CdrError::DecodingError); }} \
             let mut {s} = heapless::String::<{cap}>::new(); \
             for __c in core::char::decode_utf16(dec.decode_bytes({n})?.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]]))) \
             {{ {s}.push(__c.map_err(|_| CdrError::DecodingError)?).map_err(|_| CdrError::BufferTooSmall)?; }} {s} }}"
        )
    }

    /// `[T; N]...` read element by element (row-major).
    fn dec_array(&mut self, elem: &IdlType, dims: &[u32]) -> String {
        let Some((&first, rest)) = dims.split_first() else {
            return self.dec(elem);
        };
        let inner = self.dec_array(elem, rest);
        let mut t = self.rust_type(elem);
        for d in rest.iter().rev() {
            t = format!("[{t}; {d}]");
        }
        let v = self.fresh("a");
        format!(
            "{{ let mut {v} = heapless::Vec::<{t}, {first}>::new(); for _ in 0..{first} {{ {v}.push({inner}).map_err(|_| CdrError::BufferTooSmall)?; }} {v}.into_array().map_err(|_| CdrError::DecodingError)? }}"
        )
    }

    // -------------------------------------------------------------------
    // Maximum serialized size (XCDR1 alignment, bounded capacities).
    // -------------------------------------------------------------------

    /// End offset after the largest value of `ty` written at `offset`;
    /// `None` for a type that reaches itself.
    pub(super) fn max_end(&self, ty: &IdlType, offset: usize, depth: usize) -> Option<usize> {
        if depth > 32 {
            return None;
        }
        let align = |o: usize, a: usize| o.div_ceil(a) * a;
        Some(match self.idx.resolve(ty) {
            IdlType::Primitive(p) => match p {
                PrimitiveType::String => align(offset, 4) + 4 + self.cfg.max_string_len + 1,
                PrimitiveType::WString => align(offset, 4) + 4 + 2 * self.cfg.max_string_len,
                PrimitiveType::WChar => align(offset, 2) + 2,
                PrimitiveType::Void | PrimitiveType::Fixed { .. } => offset,
                p => {
                    let size = data_model::primitive_size(&p).unwrap_or(0);
                    align(offset, size.max(1)) + size
                }
            },
            IdlType::BoundedString { wide: false, bound } => {
                align(offset, 4) + 4 + bound as usize + 1
            }
            IdlType::BoundedString { wide: true, bound } => {
                align(offset, 4) + 4 + 2 * bound as usize
            }
            IdlType::Sequence { inner, bound } => {
                let mut end = align(offset, 4) + 4;
                for _ in 0..self.seq_cap(bound) {
                    end = self.max_end(&inner, end, depth + 1)?;
                }
                end
            }
            IdlType::Array { .. } => {
                let (elem, dims) = flatten_array(self.idx, ty)?;
                let count: usize = dims.iter().map(|d| *d as usize).product();
                let mut end = offset;
                for _ in 0..count {
                    end = self.max_end(&elem, end, depth + 1)?;
                }
                end
            }
            IdlType::Named(n) => match self.idx.named(&n) {
                Some(Named::Enum { bit_bound, .. }) => {
                    let w = enum_width(*bit_bound);
                    align(offset, w) + w
                }
                Some(Named::Struct(s)) => {
                    let s = s.clone();
                    self.struct_max_end(&s, offset, depth + 1)?
                }
                Some(Named::Union(u)) => {
                    let u = u.clone();
                    let w = disc_width(self.idx, &u.discriminator);
                    if union_ext(&u) == Ext::Mutable {
                        // Two parameters (12-byte headers at 4, values from
                        // their own origin) and PID_LIST_END.
                        let mut case_max = 0;
                        for c in &u.cases {
                            case_max =
                                case_max.max(self.max_end(&c.field.field_type, 0, depth + 1)?);
                        }
                        let disc_end = align(offset, 4) + 12 + w;
                        let case_end = align(disc_end, 4) + 12 + case_max;
                        align(case_end, 4) + 4
                    } else {
                        let after = align(offset, w) + w;
                        let mut end = after;
                        for c in &u.cases {
                            end = end.max(self.max_end(&c.field.field_type, after, depth + 1)?);
                        }
                        end
                    }
                }
                _ => offset,
            },
            IdlType::Map { .. } => offset,
        })
    }

    pub(super) fn struct_max_end(&self, s: &Struct, offset: usize, depth: usize) -> Option<usize> {
        let chain = data_model::member_chain(self.idx, s).ok()?;
        let mutable = data_model::struct_ext(s) == Ext::Mutable;
        let align4 = |o: usize| o.div_ceil(4) * 4;
        let mut end = offset;
        for m in &chain {
            if mutable || m.field.is_optional() {
                // Parameter header (12 bytes, aligned at 4), the value from
                // its own origin.
                end = align4(end) + 12 + self.max_end(&m.field.field_type, 0, depth + 1)?;
            } else {
                end = self.max_end(&m.field.field_type, end, depth + 1)?;
            }
        }
        if mutable {
            end = align4(end) + 4;
        }
        Some(end)
    }

    pub(super) fn finish(self) -> String {
        self.out
    }

    /// The default value of `ty` (DDS-XTypes 1.3 7.2.2.4.4.4.7: zero, empty,
    /// the first enumerator, a nested type's default), the value a member
    /// takes when the data has none for it (type evolution, 7.4.1.2.3).
    fn default_expr(&self, ty: &IdlType) -> String {
        match self.idx.resolve(ty) {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Boolean => "false".into(),
                PrimitiveType::Float | PrimitiveType::Double | PrimitiveType::LongDouble => {
                    "0.0".into()
                }
                PrimitiveType::String | PrimitiveType::WString => "heapless::String::new()".into(),
                PrimitiveType::Void => "()".into(),
                _ => "0".into(),
            },
            IdlType::BoundedString { wide: false, .. } => "heapless::String::new()".into(),
            IdlType::BoundedString { wide: true, .. } | IdlType::Sequence { .. } => {
                "heapless::Vec::new()".into()
            }
            IdlType::Array { inner, .. } => {
                format!("core::array::from_fn(|_| {})", self.default_expr(&inner))
            }
            IdlType::Map { .. } => "()".into(),
            IdlType::Named(n) => match self.idx.named(&n) {
                Some(Named::Struct(_) | Named::Union(_)) => {
                    format!("{}::__hdds_default()", crate::codegen::rust_paths::path(&n))
                }
                _ => format!(
                    "<{} as Default>::default()",
                    crate::codegen::rust_paths::path(&n)
                ),
            },
        }
    }
}

/// `__hdds_default()` of a union (inside its `impl`): the case the
/// discriminator's default value selects (0, FALSE or the first
/// enumerator, DDS-XTypes 1.3 7.2.2.4.4.4.7), else the `default` case,
/// with its value's default. When that value selects no case the spec's
/// union holds the discriminator alone; the Rust enum has no such state
/// and takes its first case instead.
pub(super) fn union_default(
    idx: &TypeIndex,
    cfg: &MicroConfig,
    u: &Union,
    variant: &dyn Fn(&str, usize) -> String,
) -> String {
    let g = MGen::new(idx, cfg);
    let disc_default = match idx.resolve(&u.discriminator) {
        IdlType::Named(n) => match idx.named(&n) {
            Some(Named::Enum { literals, .. }) => literals.first().map_or(0, |(_, v)| *v),
            _ => 0,
        },
        _ => 0,
    };
    let selects = |c: &crate::ast::UnionCase| {
        c.labels.iter().any(|l| match l {
            UnionLabel::Value(v) => {
                union_label_value(idx, &u.discriminator, v) == Some(disc_default)
            }
            UnionLabel::Default => false,
        })
    };
    let pick = u
        .cases
        .iter()
        .position(selects)
        .or_else(|| {
            u.cases
                .iter()
                .position(|c| c.labels.iter().any(|l| matches!(l, UnionLabel::Default)))
        })
        .unwrap_or(0);
    let body = u.cases.get(pick).map_or_else(
        || "compile_error!(\"hddsgen: a union without cases\")".to_string(),
        |case| {
            format!(
                "Self::{}({})",
                variant(&case.field.name, pick),
                g.default_expr(&case.field.field_type)
            )
        },
    );
    format!(
        "\n    /// The default value of this union (DDS-XTypes 1.3 7.2.2.4.4.4.7).\n    \
         #[doc(hidden)]\n    #[must_use]\n    pub fn __hdds_default() -> Self {{\n        {body}\n    }}\n"
    )
}

/// The discriminator's Rust holder type and serialized width.
fn disc_holder(idx: &TypeIndex, ty: &IdlType) -> (&'static str, &'static str, usize) {
    match idx.resolve(ty) {
        IdlType::Primitive(p) => match p {
            PrimitiveType::Boolean => ("bool", "bool", 1),
            PrimitiveType::Octet | PrimitiveType::UInt8 | PrimitiveType::Char => ("u8", "u8", 1),
            PrimitiveType::Int8 => ("i8", "i8", 1),
            PrimitiveType::WChar | PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => {
                ("u16", "u16", 2)
            }
            PrimitiveType::Short | PrimitiveType::Int16 => ("i16", "i16", 2),
            PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => ("u32", "u32", 4),
            PrimitiveType::LongLong | PrimitiveType::Int64 => ("i64", "i64", 8),
            PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => ("u64", "u64", 8),
            _ => ("i32", "i32", 4),
        },
        IdlType::Named(n) => match idx.named(&n) {
            Some(Named::Enum { bit_bound, .. }) => match enum_width(*bit_bound) {
                1 => ("i8", "i8", 1),
                2 => ("i16", "i16", 2),
                _ => ("i32", "i32", 4),
            },
            _ => ("i32", "i32", 4),
        },
        _ => ("i32", "i32", 4),
    }
}

/// The IDL primitive a discriminator holder serializes as.
fn holder_type(holder: &str) -> IdlType {
    IdlType::Primitive(match holder {
        "bool" => PrimitiveType::Boolean,
        "u8" => PrimitiveType::Octet,
        "i8" => PrimitiveType::Int8,
        "u16" => PrimitiveType::UnsignedShort,
        "i16" => PrimitiveType::Short,
        "u32" => PrimitiveType::UnsignedLong,
        "i64" => PrimitiveType::LongLong,
        "u64" => PrimitiveType::UnsignedLongLong,
        _ => PrimitiveType::Long,
    })
}

/// Rust expression of discriminator value `disc` in its holder type.
fn holder_literal(holder: &str, disc: i64) -> String {
    match holder {
        "bool" => (disc != 0).to_string(),
        "i64" => format!("{disc}i64"),
        h => format!("{disc}i64 as {h}"),
    }
}

/// The encode / decode methods of a MUTABLE union under XCDR1 (rule 28):
/// the discriminator as parameter 0 (`FLAG_2`: it always must be
/// understood, 7.2.2.4.4.4.6), the selected case as a parameter with its
/// id (`data_model::union_case_ids`, from 1, DDS-XTypes 1.3 7.3.1.2.1.1),
/// `PID_LIST_END`.
#[allow(clippy::too_many_lines)]
fn mutable_union_methods(
    idx: &TypeIndex,
    cfg: &MicroConfig,
    u: &Union,
    variant: &dyn Fn(&str, usize) -> String,
) -> String {
    let (holder, read, _) = disc_holder(idx, &u.discriminator);
    let disc_ty = holder_type(holder);
    let ids = data_model::union_case_ids(idx, u);
    let mut g = MGen::new(idx, cfg);
    g.line(
        1,
        "pub fn encode(&self, enc: &mut CdrEncoder) -> Result<(), CdrError> {",
    );
    g.line(2, "match self {");
    for (i, case) in u.cases.iter().enumerate() {
        let name = variant(&case.field.name, i);
        let disc = union_case_disc(idx, u, i).unwrap_or(0);
        let is_void = matches!(
            case.field.field_type,
            IdlType::Primitive(PrimitiveType::Void)
        );
        g.line(
            3,
            &format!("Self::{name}({}) => {{", if is_void { "_" } else { "__v" }),
        );
        g.enc_param_of(4, 0, true, &disc_ty, Some(&holder_literal(holder, disc)));
        if !is_void {
            let mu = data_model::must_understand(&case.field);
            g.enc_param_of(4, ids[i], mu, &case.field.field_type, Some("(*__v)"));
        }
        g.line(3, "}");
    }
    g.line(2, "}");
    g.enc_pad4(2);
    g.line(2, "enc.encode_u16(0x7f02)?; enc.encode_u16(0)?;");
    g.line(2, "Ok(())");
    g.line(1, "}");
    g.line(0, "");
    g.line(
        1,
        "pub fn decode(dec: &mut CdrDecoder) -> Result<Self, CdrError> {",
    );
    g.line(2, "let mut __disc: Option<i64> = None;");
    g.line(2, "let mut __value: Option<Self> = None;");
    g.line(2, "loop {");
    g.dec_param_header(3);
    g.line(
        3,
        "let Some((__id, __mu, __bytes)) = __param else { break };",
    );
    g.line(3, "let mut __sub = CdrDecoder::new(__bytes);");
    g.line(3, "let dec = &mut __sub;");
    g.line(3, "match __id {");
    let read_expr = match read {
        "bool" => "i64::from(dec.decode_bool()?)",
        "u8" => "i64::from(dec.decode_u8()?)",
        "i8" => "i64::from(dec.decode_i8()?)",
        "u16" => "i64::from(dec.decode_u16()?)",
        "i16" => "i64::from(dec.decode_i16()?)",
        "u32" => "i64::from(dec.decode_u32()?)",
        "i64" => "dec.decode_i64()?",
        "u64" => "dec.decode_u64()? as i64",
        _ => "i64::from(dec.decode_i32()?)",
    };
    g.line(4, &format!("0 => {{ __disc = Some({read_expr}); }}"));
    for (i, case) in u.cases.iter().enumerate() {
        if matches!(
            case.field.field_type,
            IdlType::Primitive(PrimitiveType::Void)
        ) {
            continue;
        }
        let name = variant(&case.field.name, i);
        let value = g.dec(&case.field.field_type);
        g.line(
            4,
            &format!(
                "{:#x} => {{ if __value.is_some() {{ return Err(CdrError::DecodingError); }} __value = Some(Self::{name}({value})); }}",
                ids[i]
            ),
        );
    }
    g.line(
        4,
        "_ => { if __mu { return Err(CdrError::DecodingError); } }",
    );
    g.line(3, "}");
    g.line(2, "}");
    g.line(2, "let __disc = __disc.ok_or(CdrError::DecodingError)?;");
    g.line(2, "match (__disc, __value) {");
    let mut default_variant = None;
    for (i, case) in u.cases.iter().enumerate() {
        let name = variant(&case.field.name, i);
        let is_void = matches!(
            case.field.field_type,
            IdlType::Primitive(PrimitiveType::Void)
        );
        let mut pats = Vec::new();
        for l in &case.labels {
            match l {
                UnionLabel::Default => default_variant = Some((name.clone(), is_void)),
                UnionLabel::Value(v) => {
                    if let Some(n) = union_label_value(idx, &u.discriminator, v) {
                        pats.push(n.to_string());
                    }
                }
            }
        }
        if pats.is_empty() {
            continue;
        }
        let pat = pats.join(" | ");
        if is_void {
            g.line(3, &format!("({pat}, None) => Ok(Self::{name}(())),"));
        } else {
            g.line(
                3,
                &format!("({pat}, Some(__v @ Self::{name}(_))) => Ok(__v),"),
            );
        }
    }
    if let Some((name, is_void)) = default_variant {
        if is_void {
            g.line(3, &format!("(_, None) => Ok(Self::{name}(())),"));
        } else {
            g.line(3, &format!("(_, Some(__v @ Self::{name}(_))) => Ok(__v),"));
        }
    }
    g.line(3, "_ => Err(CdrError::DecodingError),");
    g.line(2, "}");
    g.line(1, "}");
    g.finish()
}

fn disc_width(idx: &TypeIndex, ty: &IdlType) -> usize {
    disc_holder(idx, ty).2
}

/// Place of a chain member read from `self` (base members through `base`).
fn member_place(m: &data_model::ChainMember) -> String {
    let mut place = String::from("self");
    for _ in 0..m.base_depth {
        place.push_str(".base");
    }
    let _ = write!(place, ".{}", rust_ident(&m.field.name));
    place
}

impl MGen<'_> {
    /// Zero padding up to the next multiple of 4 from the encoder's origin.
    fn enc_pad4(&mut self, ind: usize) {
        self.line(
            ind,
            "let __pad = (4 - enc.position() % 4) % 4; enc.encode_bytes(&[0u8; 3][..__pad])?;",
        );
    }

    /// The XCDR1 parameter of a member (rule 25, 7.4.1.2.2): `PID_EXTENDED`
    /// with `FLAG_M`, 8, the member header (`FLAG_2` when the member must be
    /// understood, the id), the exact value length, the value serialized
    /// from its own origin (a scratch encoder as large as its maximum).
    /// `value` is `None` for an absent optional member (length 0).
    fn enc_param(&mut self, ind: usize, m: &data_model::ChainMember, value: Option<&str>) {
        let mu = data_model::must_understand(&m.field);
        self.enc_param_of(ind, m.id, mu, &m.field.field_type, value);
    }

    /// The XCDR1 parameter of member `id` of type `ty` (see `enc_param`).
    fn enc_param_of(&mut self, ind: usize, id: u32, mu: bool, ty: &IdlType, value: Option<&str>) {
        let header = (u32::from(mu) << 30) | (id & 0x0fff_ffff);
        self.enc_pad4(ind);
        self.line(
            ind,
            &format!("enc.encode_u16(0x7f01)?; enc.encode_u16(8)?; enc.encode_u32({header:#x})?;"),
        );
        let Some(place) = value else {
            self.line(ind, "enc.encode_u32(0)?;");
            return;
        };
        let max = self.max_end(ty, 0, 1).unwrap_or(0);
        let t = self.fresh("t");
        let n = self.fresh("n");
        self.line(ind, &format!("let mut {t} = [0u8; {max}];"));
        self.line(ind, &format!("let {n} = {{"));
        self.line(
            ind + 1,
            &format!("let mut __sub = CdrEncoder::new(&mut {t});"),
        );
        self.line(ind + 1, "{");
        self.line(ind + 2, "let enc = &mut __sub;");
        self.enc(ind + 2, place, ty);
        self.line(ind + 1, "}");
        self.line(ind + 1, "__sub.position()");
        self.line(ind, "};");
        self.line(
            ind,
            &format!(
                "enc.encode_u32(u32::try_from({n}).map_err(|_| CdrError::EncodingError)?)?; enc.encode_bytes(&{t}[..{n}])?;"
            ),
        );
    }

    /// Read the next XCDR1 parameter header: `(id, must understand, value
    /// bytes)`, or `None` at `PID_LIST_END`.
    fn dec_param_header(&mut self, ind: usize) {
        self.line(
            ind,
            "let __pad = (4 - dec.position() % 4) % 4; dec.decode_bytes(__pad)?;",
        );
        self.line(ind, "let __pid = dec.decode_u16()?;");
        self.line(ind, "let __len = dec.decode_u16()?;");
        self.line(
            ind,
            "let __param = if __pid & 0x3fff == 0x3f02 { None } else if __pid & 0x3fff == 0x3f01 {",
        );
        self.line(ind + 1, "let __h = dec.decode_u32()?;");
        self.line(ind + 1, "let __size = dec.decode_u32()? as usize;");
        self.line(
            ind + 1,
            "Some((__h & 0x0fff_ffff, __h & 0x4000_0000 != 0, dec.decode_bytes(__size)?))",
        );
        self.line(ind, "} else {");
        self.line(
            ind + 1,
            "Some((u32::from(__pid & 0x3fff), __pid & 0x4000 != 0, dec.decode_bytes(usize::from(__len))?))",
        );
        self.line(ind, "};");
    }
}

/// The encode / decode methods of a structure (inside its `impl`):
/// FINAL / APPENDABLE members in order, base members first, an `@optional`
/// member as a parameter (rule 19); MUTABLE a parameter list closed by
/// `PID_LIST_END` (rules 23, 25).
pub(super) fn struct_methods(idx: &TypeIndex, cfg: &MicroConfig, s: &Struct) -> String {
    let Ok(chain) = data_model::member_chain(idx, s) else {
        return String::new();
    };
    let mutable = data_model::struct_ext(s) == Ext::Mutable;
    let appendable = data_model::struct_ext(s) == Ext::Appendable;
    let mut g = MGen::new(idx, cfg);
    g.line(
        1,
        "pub fn encode(&self, enc: &mut CdrEncoder) -> Result<(), CdrError> {",
    );
    for m in &chain {
        let place = member_place(m);
        if m.field.is_optional() {
            // Absent: left out of a MUTABLE list, an empty parameter in a
            // FINAL / APPENDABLE structure.
            g.line(2, &format!("if let Some(__o) = &{place} {{"));
            g.enc_param(3, m, Some("(*__o)"));
            if !mutable {
                g.line(2, "} else {");
                g.enc_param(3, m, None);
            }
            g.line(2, "}");
        } else if mutable {
            g.enc_param(2, m, Some(&place));
        } else {
            g.enc(2, &place, &m.field.field_type);
        }
    }
    if mutable {
        g.enc_pad4(2);
        g.line(2, "enc.encode_u16(0x7f02)?; enc.encode_u16(0)?;");
    }
    g.line(2, "Ok(())");
    g.line(1, "}");
    g.line(0, "");
    g.line(
        1,
        "pub fn decode(dec: &mut CdrDecoder) -> Result<Self, CdrError> {",
    );
    let locals: Vec<String> = (0..chain.len()).map(|i| format!("__f{i}")).collect();
    if mutable {
        for local in &locals {
            g.line(2, &format!("let mut {local} = None;"));
        }
        g.line(2, "loop {");
        g.dec_param_header(3);
        g.line(
            3,
            "let Some((__id, __mu, __bytes)) = __param else { break };",
        );
        g.line(3, "let mut __sub = CdrDecoder::new(__bytes);");
        g.line(3, "let dec = &mut __sub;");
        g.line(3, "match __id {");
        for (m, local) in chain.iter().zip(&locals) {
            let value = g.dec(&m.field.field_type);
            g.line(4, &format!("{:#x} => {{ {local} = Some({value}); }}", m.id));
        }
        g.line(
            4,
            "_ => { if __mu { return Err(CdrError::DecodingError); } }",
        );
        g.line(3, "}");
        g.line(2, "}");
        for (m, local) in chain.iter().zip(&locals) {
            if !m.field.is_optional() {
                // A member the writer's type does not have takes its
                // default (DDS-XTypes 1.3 7.4.1.2.3, spec text lines
                // 6195-6203).
                let default = g.default_expr(&m.field.field_type);
                g.line(
                    2,
                    &format!("let {local} = {local}.unwrap_or_else(|| {default});"),
                );
            }
        }
    } else {
        dec_in_order(&mut g, &chain, &locals, appendable);
    }
    let literal = struct_literal(idx, s, &s.name, &chain, &locals, 0);
    g.line(2, &format!("Ok({literal})"));
    g.line(1, "}");
    struct_default(&mut g, s, &chain);
    g.finish()
}

/// FINAL / APPENDABLE members in order; an APPENDABLE member the data ends
/// before (a writer whose type has fewer members) takes its default.
fn dec_in_order(
    g: &mut MGen<'_>,
    chain: &[data_model::ChainMember],
    locals: &[String],
    appendable: bool,
) {
    for (m, local) in chain.iter().zip(locals) {
        if appendable {
            // A writer whose type has fewer members ends its data before
            // this one (XCDR1 has no DHEADER: only the outermost type
            // can see it): default.
            let default = if m.field.is_optional() {
                "None".to_string()
            } else {
                g.default_expr(&m.field.field_type)
            };
            g.line(
                2,
                &format!("let {local} = if dec.remaining() == 0 {{ {default} }} else {{"),
            );
        } else {
            g.line(2, &format!("let {local} = {{"));
        }
        if m.field.is_optional() {
            g.line(2, &format!("let {local} = {{"));
            g.dec_param_header(3);
            g.line(
                3,
                "let Some((_, _, __bytes)) = __param else { return Err(CdrError::DecodingError) };",
            );
            g.line(3, "if __bytes.is_empty() { None } else {");
            g.line(4, "let mut __sub = CdrDecoder::new(__bytes);");
            g.line(4, "let dec = &mut __sub;");
            let value = g.dec(&m.field.field_type);
            g.line(4, &format!("Some({value})"));
            g.line(3, "}");
            g.line(2, "};");
            g.line(2, &format!("{local} }};"));
        } else {
            let value = g.dec(&m.field.field_type);
            g.line(2, &format!("{value} }};"));
        }
    }
}

/// `__hdds_default()` of a structure (inside its `impl`): every member at
/// its default (DDS-XTypes 1.3 7.2.2.4.4.4.7), `None` when optional.
fn struct_default(g: &mut MGen<'_>, s: &Struct, chain: &[data_model::ChainMember]) {
    g.line(0, "");
    g.line(
        1,
        "/// The default value of this type (DDS-XTypes 1.3 7.2.2.4.4.4.7), what a",
    );
    g.line(1, "/// member takes when a writer's type has none for it.");
    g.line(1, "#[doc(hidden)]");
    g.line(1, "#[must_use]");
    g.line(1, "pub fn __hdds_default() -> Self {");
    let defaults: Vec<String> = chain
        .iter()
        .map(|m| {
            if m.field.is_optional() {
                "None".to_string()
            } else {
                g.default_expr(&m.field.field_type)
            }
        })
        .collect();
    let literal = struct_literal(g.idx, s, &s.name, chain, &defaults, 0);
    g.line(2, &literal);
    g.line(1, "}");
}

/// The literal of `s` (at `depth` base hops from the decoded type) from the
/// decoded locals; `@non_serialized` members take their default.
fn struct_literal(
    idx: &TypeIndex,
    s: &Struct,
    path: &str,
    chain: &[data_model::ChainMember],
    locals: &[String],
    depth: usize,
) -> String {
    let mut lit = format!("{path} {{ ");
    if let Some(base) = &s.base_struct {
        if let Some(b) = idx.struct_def(base) {
            let b = b.clone();
            let _ = write!(
                lit,
                "base: {}, ",
                struct_literal(
                    idx,
                    &b,
                    &crate::codegen::rust_paths::path(base),
                    chain,
                    locals,
                    depth + 1
                )
            );
        }
    }
    for field in &s.fields {
        let ident = rust_ident(&field.name);
        let local = chain
            .iter()
            .zip(locals)
            .find(|(m, _)| m.base_depth == depth && m.owner == s.name && m.field.name == field.name)
            .map_or_else(|| "Default::default()".to_string(), |(_, l)| l.clone());
        let _ = write!(lit, "{ident}: {local}, ");
    }
    lit.push('}');
    lit
}

/// The encode / decode methods of a union (inside its `impl`); `variant`
/// names the Rust variant of a case.
pub(super) fn union_methods(
    idx: &TypeIndex,
    cfg: &MicroConfig,
    u: &Union,
    variant: &dyn Fn(&str, usize) -> String,
) -> String {
    if union_ext(u) == Ext::Mutable {
        return mutable_union_methods(idx, cfg, u, variant);
    }
    let (holder, read, width) = disc_holder(idx, &u.discriminator);
    let mut g = MGen::new(idx, cfg);
    g.line(
        1,
        "pub fn encode(&self, enc: &mut CdrEncoder) -> Result<(), CdrError> {",
    );
    g.line(2, "match self {");
    for (i, case) in u.cases.iter().enumerate() {
        let name = variant(&case.field.name, i);
        let disc = union_case_disc(idx, u, i).unwrap_or(0);
        let is_void = matches!(
            case.field.field_type,
            IdlType::Primitive(PrimitiveType::Void)
        );
        g.line(
            3,
            &format!("Self::{name}({}) => {{", if is_void { "_" } else { "__v" }),
        );
        let write = match holder {
            "bool" => format!("enc.encode_bool({})?;", disc != 0),
            "u8" => format!("enc.encode_u8({disc}i64 as u8)?;"),
            "i8" => format!("enc.encode_i8({disc}i64 as i8)?;"),
            "u16" => format!("enc.encode_u16({disc}i64 as u16)?;"),
            "i16" => format!("enc.encode_i16({disc}i64 as i16)?;"),
            "u32" => format!("enc.encode_u32({disc}i64 as u32)?;"),
            "i64" => format!("enc.encode_i64({disc}i64)?;"),
            "u64" => format!("enc.encode_u64({disc}i64 as u64)?;"),
            _ => format!("enc.encode_i32({disc}i64 as i32)?;"),
        };
        g.line(4, &write);
        if !is_void {
            g.enc(4, "(*__v)", &case.field.field_type);
        }
        g.line(3, "}");
    }
    g.line(2, "}");
    g.line(2, "Ok(())");
    g.line(1, "}");
    g.line(0, "");
    g.line(
        1,
        "pub fn decode(dec: &mut CdrDecoder) -> Result<Self, CdrError> {",
    );
    let read_expr = match (read, width) {
        ("bool", _) => "i64::from(dec.decode_bool()?)",
        ("u8", _) => "i64::from(dec.decode_u8()?)",
        ("i8", _) => "i64::from(dec.decode_i8()?)",
        ("u16", _) => "i64::from(dec.decode_u16()?)",
        ("i16", _) => "i64::from(dec.decode_i16()?)",
        ("u32", _) => "i64::from(dec.decode_u32()?)",
        ("i64", _) => "dec.decode_i64()?",
        ("u64", _) => "dec.decode_u64()? as i64",
        _ => "i64::from(dec.decode_i32()?)",
    };
    g.line(2, &format!("let disc: i64 = {read_expr};"));
    g.line(2, "Ok(match disc {");
    let mut default_arm = None;
    for (i, case) in u.cases.iter().enumerate() {
        let name = variant(&case.field.name, i);
        let is_void = matches!(
            case.field.field_type,
            IdlType::Primitive(PrimitiveType::Void)
        );
        let value = if is_void {
            "()".to_string()
        } else {
            g.dec(&case.field.field_type)
        };
        let arm = format!("Self::{name}({value})");
        let mut pats = Vec::new();
        for l in &case.labels {
            match l {
                UnionLabel::Default => default_arm = Some(arm.clone()),
                UnionLabel::Value(v) => {
                    if let Some(n) = union_label_value(idx, &u.discriminator, v) {
                        pats.push(n.to_string());
                    }
                }
            }
        }
        if !pats.is_empty() {
            g.line(3, &format!("{} => {arm},", pats.join(" | ")));
        }
    }
    match default_arm {
        Some(arm) => g.line(3, &format!("_ => {arm},")),
        None => g.line(3, "_ => return Err(CdrError::DecodingError),"),
    }
    g.line(2, "})");
    g.line(1, "}");
    let _ = width;
    g.finish()
}

/// Why a structure cannot be generated by this backend, if it cannot.
pub(super) fn struct_refusal(idx: &TypeIndex, cfg: &MicroConfig, s: &Struct) -> Option<String> {
    let g = MGen::new(idx, cfg);
    if let Err(why) = data_model::member_chain(idx, s) {
        return Some(format!("struct `{}`: {why}", s.name));
    }
    for f in &s.fields {
        if let Some(why) = g.unsupported(&f.field_type) {
            return Some(format!("member `{}` of `{}`: {why}", f.name, s.name));
        }
    }
    None
}

/// Why a union cannot be generated by this backend, if it cannot.
pub(super) fn union_refusal(idx: &TypeIndex, cfg: &MicroConfig, u: &Union) -> Option<String> {
    let g = MGen::new(idx, cfg);
    for c in &u.cases {
        if let Some(why) = g.unsupported(&c.field.field_type) {
            return Some(format!("case `{}` of `{}`: {why}", c.field.name, u.name));
        }
    }
    None
}

/// The `from_i32` and encode / decode methods of an enumeration (inside
/// its `impl`), at the `@bit_bound` width.
pub(super) fn enum_methods(e: &crate::ast::Enum) -> String {
    let width = enum_width(e.annotations.iter().find_map(|a| match a {
        crate::types::Annotation::BitBound(n) => Some(*n),
        _ => None,
    }));
    let mut out = String::new();
    out.push_str("    /// The enumerator of `v`, an error for a value it does not have.\n");
    out.push_str("    pub fn from_i32(v: i32) -> Result<Self, CdrError> {\n");
    out.push_str("        match v {\n");
    for (v, value) in e.variants.iter().zip(e.resolved_values()) {
        let _ = writeln!(out, "            {value} => Ok(Self::{}),", v.name);
    }
    out.push_str("            _ => Err(CdrError::DecodingError),\n");
    out.push_str("        }\n    }\n\n");
    let (enc, dec) = match width {
        1 => (
            "enc.encode_i8((*self as i32) as i8)",
            "Self::from_i32(i32::from(dec.decode_i8()?))",
        ),
        2 => (
            "enc.encode_i16((*self as i32) as i16)",
            "Self::from_i32(i32::from(dec.decode_i16()?))",
        ),
        _ => (
            "enc.encode_i32(*self as i32)",
            "Self::from_i32(dec.decode_i32()?)",
        ),
    };
    let _ = write!(
        out,
        "    pub fn encode(&self, enc: &mut CdrEncoder) -> Result<(), CdrError> {{\n        {enc}\n    }}\n\n\
         \x20   pub fn decode(dec: &mut CdrDecoder) -> Result<Self, CdrError> {{\n        {dec}\n    }}\n"
    );
    out
}
