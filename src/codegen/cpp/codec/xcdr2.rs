// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! XCDR2 codec of the generated C++ types (DDS-`XTypes` 1.3 7.4.3.5.3).
//!
//! One emitter serves structures and unions. Every value is written through
//! the `cdr2::` helpers emitted once per header (see
//! [`super::generate_xcdr2_helpers`]): alignment is `min(size, 4)` measured
//! from the start of the buffer the public entry point was handed (a nested
//! value is aligned where it lands, never against its own first byte), and
//! padding is zero.
//!
//! The data model (extensibility, member ids, DHEADERs of collections,
//! enumeration and bitmask widths, `EMHEADER1` length codes, the
//! must-understand flag) is [`crate::codegen::data_model`], shared with the
//! other backends.

use std::fmt::Write as _;

use super::super::helpers::type_to_cpp;
use super::super::index::DefinitionIndex;
use crate::ast::{Struct, Union, UnionLabel};
use crate::codegen::data_model::{
    self, bitmask_width, collection_has_dheader, enum_width, flatten_array, length_code,
    member_chain, must_understand, union_ext, ChainMember, Ext,
};
use crate::codegen::key_holder::{Named, TypeIndex};
use crate::codegen::keywords::cpp_ident;
use crate::codegen::least_wire;
use crate::types::{IdlType, PrimitiveType};

/// How a resolved type serializes, as far as the C++ codec cares.
enum Kind {
    /// A primitive written as a scalar of a fixed wire type.
    Scalar(Scalar),
    /// `fixed<D, S>`: 16 bytes, aligned at 4.
    Fixed,
    /// `void`: nothing.
    Void,
    /// A string. `array`: held in a `std::array<C, N>` (a bounded string),
    /// read up to its first NUL; otherwise a container with `data()`,
    /// `size()`, `assign()` / `resize()`.
    Str {
        wide: bool,
        bound: usize,
        array: bool,
    },
    Seq {
        inner: IdlType,
        bound: usize,
    },
    Array {
        elem: IdlType,
        dims: Vec<u32>,
    },
    Map {
        key: IdlType,
        value: IdlType,
        bound: usize,
    },
    Aggregate,
    Enum {
        width: usize,
        literals: Vec<i64>,
    },
    Bitmask {
        width: usize,
    },
    Bitset,
    Unknown(String),
}

/// A primitive scalar: its C++ wire type and how the value converts.
#[derive(Clone, Copy)]
enum Scalar {
    Bool,
    Char,
    WChar,
    Num(&'static str),
}

const fn scalar_of(p: &PrimitiveType) -> Option<Scalar> {
    Some(match p {
        PrimitiveType::Boolean => Scalar::Bool,
        PrimitiveType::Char => Scalar::Char,
        PrimitiveType::WChar => Scalar::WChar,
        PrimitiveType::Octet | PrimitiveType::UInt8 => Scalar::Num("std::uint8_t"),
        PrimitiveType::Int8 => Scalar::Num("std::int8_t"),
        PrimitiveType::Short | PrimitiveType::Int16 => Scalar::Num("std::int16_t"),
        PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => Scalar::Num("std::uint16_t"),
        PrimitiveType::Long | PrimitiveType::Int32 => Scalar::Num("std::int32_t"),
        PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => Scalar::Num("std::uint32_t"),
        PrimitiveType::LongLong | PrimitiveType::Int64 => Scalar::Num("std::int64_t"),
        PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => Scalar::Num("std::uint64_t"),
        PrimitiveType::Float => Scalar::Num("float"),
        // `long double` is serialized as the 8-byte float hddsgen maps it to
        // (data_model::primitive_size).
        PrimitiveType::Double | PrimitiveType::LongDouble => Scalar::Num("double"),
        PrimitiveType::String
        | PrimitiveType::WString
        | PrimitiveType::Fixed { .. }
        | PrimitiveType::Void => return None,
    })
}

const fn signed_int(width: usize) -> &'static str {
    match width {
        1 => "std::int8_t",
        2 => "std::int16_t",
        _ => "std::int32_t",
    }
}

const fn unsigned_int(width: usize) -> &'static str {
    match width {
        1 => "std::uint8_t",
        2 => "std::uint16_t",
        4 => "std::uint32_t",
        _ => "std::uint64_t",
    }
}

fn bound_of(b: Option<u32>) -> usize {
    b.map_or(0, |n| n as usize)
}

/// Code emitter: the type index, the output, the base indentation and a
/// counter for unique local names.
struct Gen<'a> {
    idx: &'a TypeIndex,
    out: String,
    indent: String,
    var: usize,
    key_only: bool,
    /// Emitting an XCDR1 decoder: every `cdr2::get<T>` read becomes a read
    /// of the `cdr2::X1` cursor named `origin` (alignment `min(size, 8)`
    /// from its origin), and no DHEADER is read.
    v1: bool,
    origin: String,
}

impl<'a> Gen<'a> {
    fn new(idx: &'a TypeIndex, indent: &str) -> Self {
        Self {
            idx,
            out: String::new(),
            indent: indent.to_string(),
            var: 0,
            key_only: false,
            v1: false,
            origin: String::new(),
        }
    }

    /// An XCDR1 decoder emitter, its origin the `hdds_x1` parameter.
    fn new_v1(idx: &'a TypeIndex, indent: &str) -> Self {
        let mut g = Self::new(idx, indent);
        g.v1 = true;
        g.origin = "hdds_x1".to_string();
        g
    }

    /// Whether a collection of type `ty` carries a DHEADER in the encoding
    /// being emitted (never under XCDR1).
    fn has_dheader(&self, ty: &IdlType) -> bool {
        !self.v1 && collection_has_dheader(self.idx, ty)
    }

    fn fresh(&mut self, prefix: &str) -> String {
        let v = format!("{prefix}{}_", self.var);
        self.var += 1;
        v
    }

    fn line(&mut self, ind: usize, text: &str) {
        self.out.push_str(&self.indent);
        for _ in 0..ind {
            self.out.push_str("    ");
        }
        if self.v1 {
            let get = format!("{}.get<", self.origin);
            self.out.push_str(&text.replace("cdr2::get<", &get));
        } else {
            self.out.push_str(text);
        }
        self.out.push('\n');
    }

    fn kind(&self, ty: &IdlType) -> Kind {
        let resolved = self.idx.resolve(ty);
        match resolved {
            // `string<N>` / `wstring<N>`, held in a `std::array`.
            IdlType::BoundedString { wide, bound } => Kind::Str {
                wide,
                bound: bound as usize,
                array: true,
            },
            IdlType::Primitive(PrimitiveType::String) => Kind::Str {
                wide: false,
                bound: 0,
                array: false,
            },
            IdlType::Primitive(PrimitiveType::WString) => Kind::Str {
                wide: true,
                bound: 0,
                array: false,
            },
            IdlType::Primitive(PrimitiveType::Fixed { .. }) => Kind::Fixed,
            IdlType::Primitive(PrimitiveType::Void) => Kind::Void,
            IdlType::Primitive(p) => scalar_of(&p).map_or(Kind::Void, Kind::Scalar),
            // `sequence<char>` / `sequence<wchar>` too: an element count,
            // no NUL (rule 11, not rules 3-4).
            IdlType::Sequence { inner, bound } => Kind::Seq {
                inner: *inner,
                bound: bound_of(bound),
            },
            IdlType::Array { .. } => {
                let (elem, dims) = flatten_array(self.idx, &resolved)
                    .unwrap_or_else(|| (resolved.clone(), vec![]));
                Kind::Array { elem, dims }
            }
            IdlType::Map { key, value, bound } => Kind::Map {
                key: *key,
                value: *value,
                bound: bound_of(bound),
            },
            IdlType::Named(n) => match self.idx.named(&n) {
                Some(Named::Struct(_) | Named::Union(_)) => Kind::Aggregate,
                Some(Named::Enum {
                    bit_bound,
                    literals,
                }) => Kind::Enum {
                    width: enum_width(*bit_bound),
                    literals: literals.iter().map(|(_, v)| *v).collect(),
                },
                Some(Named::Bitmask { bit_bound }) => Kind::Bitmask {
                    width: bitmask_width(*bit_bound),
                },
                Some(Named::Bitset) => Kind::Bitset,
                Some(Named::Typedef(_)) | None => Kind::Unknown(n),
            },
        }
    }

    // -----------------------------------------------------------------------
    // Encoding: statements writing the value at `place` (an expression of
    // the member's C++ type) at `dst + offset`.
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_lines)]
    fn enc(&mut self, ind: usize, place: &str, ty: &IdlType) {
        match self.kind(ty) {
            Kind::Scalar(s) => self.enc_scalar(ind, place, s),
            Kind::Fixed => {
                self.line(
                    ind,
                    &format!(
                        "{{ auto b_ = ({place}).to_le_bytes(); if (!cdr2::pad_to_align(dst, offset, len, 4) || !cdr2::put_raw(dst, len, offset, b_.data(), 16)) return -1; }}"
                    ),
                );
            }
            Kind::Void => {}
            Kind::Str { wide, bound, array } => {
                let put = if wide { "put_wstring" } else { "put_string" };
                let n = if array {
                    format!("cdr2::bounded_len(({place}).data(), ({place}).size())")
                } else {
                    format!("({place}).size()")
                };
                self.line(
                    ind,
                    &format!(
                        "if (!cdr2::{put}(dst, len, offset, ({place}).data(), {n}, {bound})) return -1;"
                    ),
                );
            }
            Kind::Seq { inner, bound } => {
                let dh = collection_has_dheader(self.idx, ty);
                self.line(ind, "{");
                let d = self.begin_dheader_enc(ind + 1, dh);
                if bound != 0 {
                    self.line(
                        ind + 1,
                        &format!("if (({place}).size() > {bound}u) return -1;"),
                    );
                }
                self.line(
                    ind + 1,
                    &format!(
                        "if (!cdr2::put_count(dst, len, offset, ({place}).size())) return -1;"
                    ),
                );
                let i = self.fresh("i");
                self.line(
                    ind + 1,
                    &format!("for (std::size_t {i} = 0; {i} < ({place}).size(); ++{i}) {{"),
                );
                self.enc(ind + 2, &format!("({place})[{i}]"), &inner);
                self.line(ind + 1, "}");
                self.end_dheader_enc(ind + 1, d.as_deref());
                self.line(ind, "}");
            }
            Kind::Array { elem, dims } => {
                let dh = collection_has_dheader(self.idx, ty);
                self.line(ind, "{");
                let d = self.begin_dheader_enc(ind + 1, dh);
                let mut cur = format!("({place})");
                let mut depth = ind + 1;
                for dim in &dims {
                    let i = self.fresh("i");
                    self.line(
                        depth,
                        &format!("for (std::size_t {i} = 0; {i} < {dim}u; ++{i}) {{"),
                    );
                    cur = format!("{cur}[{i}]");
                    depth += 1;
                }
                self.enc(depth, &cur, &elem);
                for _ in &dims {
                    depth -= 1;
                    self.line(depth, "}");
                }
                self.end_dheader_enc(ind + 1, d.as_deref());
                self.line(ind, "}");
            }
            Kind::Map { key, value, bound } => {
                let dh = collection_has_dheader(self.idx, ty);
                self.line(ind, "{");
                let d = self.begin_dheader_enc(ind + 1, dh);
                if bound != 0 {
                    self.line(
                        ind + 1,
                        &format!("if (({place}).size() > {bound}u) return -1;"),
                    );
                }
                self.line(
                    ind + 1,
                    &format!(
                        "if (!cdr2::put_count(dst, len, offset, ({place}).size())) return -1;"
                    ),
                );
                let kv = self.fresh("kv");
                self.line(ind + 1, &format!("for (const auto& {kv} : {place}) {{"));
                self.enc(ind + 2, &format!("{kv}.first"), &key);
                self.enc(ind + 2, &format!("{kv}.second"), &value);
                self.line(ind + 1, "}");
                self.end_dheader_enc(ind + 1, d.as_deref());
                self.line(ind, "}");
            }
            Kind::Aggregate => {
                let method = if self.key_only {
                    "encode_key_cdr2_le_at"
                } else {
                    "encode_cdr2_le_at"
                };
                let e = self.fresh("e");
                self.line(
                    ind,
                    &format!("if (int {e} = ({place}).{method}(dst, len, offset)) return {e};"),
                );
            }
            Kind::Enum { width, .. } => {
                let w = signed_int(width);
                self.line(
                    ind,
                    &format!(
                        "if (!cdr2::put<{w}>(dst, len, offset, static_cast<{w}>({place}))) return -1;"
                    ),
                );
            }
            Kind::Bitmask { width } => {
                let w = unsigned_int(width);
                self.line(
                    ind,
                    &format!(
                        "if (!cdr2::put<{w}>(dst, len, offset, static_cast<{w}>(static_cast<std::uint64_t>({place})))) return -1;"
                    ),
                );
            }
            Kind::Bitset => {
                self.line(
                    ind,
                    &format!(
                        "if (!cdr2::put<std::uint64_t>(dst, len, offset, ({place}).to_uint64())) return -1;"
                    ),
                );
            }
            Kind::Unknown(n) => {
                self.line(
                    ind,
                    &format!("return -1; // type `{n}` is not defined in this IDL file"),
                );
            }
        }
    }

    fn enc_scalar(&mut self, ind: usize, place: &str, s: Scalar) {
        let stmt = match s {
            Scalar::Bool => format!(
                "if (!cdr2::put<std::uint8_t>(dst, len, offset, static_cast<std::uint8_t>(({place}) ? 1 : 0))) return -1;"
            ),
            Scalar::Char => format!(
                "if (!cdr2::put<std::uint8_t>(dst, len, offset, static_cast<std::uint8_t>({place}))) return -1;"
            ),
            // wchar: one UTF-16 code unit (7.4.3.5.3, as Cyclone DDS 0.11).
            Scalar::WChar => format!(
                "if (!cdr2::put_wchar(dst, len, offset, static_cast<std::uint32_t>({place}))) return -1;"
            ),
            Scalar::Num(w) => format!(
                "if (!cdr2::put<{w}>(dst, len, offset, static_cast<{w}>({place}))) return -1;"
            ),
        };
        self.line(ind, &stmt);
    }

    /// Reserve a DHEADER when `dh` (XCDR2 rules 9, 12, 15); its variable.
    fn begin_dheader_enc(&mut self, ind: usize, dh: bool) -> Option<String> {
        if !dh {
            return None;
        }
        let d = self.fresh("dh");
        self.line(
            ind,
            &format!(
                "std::size_t {d} = 0; if (!cdr2::begin_len(dst, len, offset, {d})) return -1;"
            ),
        );
        Some(d)
    }

    fn end_dheader_enc(&mut self, ind: usize, d: Option<&str>) {
        if let Some(d) = d {
            self.line(
                ind,
                &format!("if (!cdr2::end_len(dst, offset, {d})) return -1;"),
            );
        }
    }

    // -----------------------------------------------------------------------
    // Decoding: statements reading the value from `src + offset` into the
    // lvalue `place` of the member's C++ type.
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_lines)]
    fn dec(&mut self, ind: usize, place: &str, ty: &IdlType) {
        match self.kind(ty) {
            Kind::Scalar(s) => self.dec_scalar(ind, place, s),
            Kind::Fixed => {
                self.line(ind, "{");
                self.line(ind + 1, "offset = cdr2::align_offset(offset, 4);");
                self.line(ind + 1, "if (!cdr2::can_read(len, offset, 16)) return -1;");
                self.line(ind + 1, "std::array<std::uint8_t, 16> b_;");
                self.line(ind + 1, "std::memcpy(b_.data(), src + offset, 16);");
                self.line(
                    ind + 1,
                    &format!(
                        "{place} = std::remove_reference_t<decltype({place})>::from_le_bytes(b_);"
                    ),
                );
                self.line(ind + 1, "offset += 16;");
                self.line(ind, "}");
            }
            Kind::Void => {}
            Kind::Str {
                wide: false,
                bound,
                array,
            } => {
                let p = self.fresh("p");
                let n = self.fresh("n");
                self.line(ind, "{");
                self.line(
                    ind + 1,
                    &format!(
                        "const char* {p} = nullptr; std::size_t {n} = 0; if (!cdr2::get_string(src, len, offset, {p}, {n}, {bound})) return -1;"
                    ),
                );
                if array {
                    self.line(
                        ind + 1,
                        &format!(
                            "({place}).fill(0); if ({n} != 0) std::memcpy(({place}).data(), {p}, {n});"
                        ),
                    );
                } else {
                    self.line(ind + 1, &format!("({place}).assign({p}, {p} + {n});"));
                }
                self.line(ind, "}");
            }
            Kind::Str {
                wide: true,
                bound,
                array,
            } => {
                let u = self.fresh("u");
                let c = self.fresh("c");
                let w = self.fresh("w");
                self.line(ind, "{");
                self.line(
                    ind + 1,
                    &format!(
                        "const std::uint8_t* {u} = nullptr; std::size_t {c} = 0; std::size_t {w} = 0;"
                    ),
                );
                self.line(
                    ind + 1,
                    &format!(
                        "if (!cdr2::get_wstring(src, len, offset, {u}, {c}, {bound}) || !cdr2::wide_len({u}, {c}, {w})) return -1;"
                    ),
                );
                if array {
                    self.line(ind + 1, &format!("if ({w} > ({place}).size()) return -1;"));
                    self.line(ind + 1, &format!("({place}).fill(0);"));
                } else {
                    self.line(ind + 1, &format!("({place}).resize({w});"));
                }
                self.line(
                    ind + 1,
                    &format!("cdr2::wide_copy({u}, {c}, ({place}).data());"),
                );
                self.line(ind, "}");
            }
            Kind::Seq { inner, bound } => {
                let dh = self.has_dheader(ty);
                self.line(ind, "{");
                let end = self.begin_dheader_dec(ind + 1, dh);
                let n = self.fresh("n");
                self.line(
                    ind + 1,
                    &format!(
                        "std::uint32_t {n} = 0; if (!cdr2::get<std::uint32_t>(src, len, offset, {n})) return -1;"
                    ),
                );
                if bound != 0 {
                    self.line(ind + 1, &format!("if ({n} > {bound}u) return -1;"));
                }
                let elem_ty = type_to_cpp(&inner);
                let min = self.min_wire(&inner);
                self.bound_storage(ind + 1, &n, &format!("sizeof({elem_ty})"), min);
                self.line(ind + 1, &format!("({place}).clear();"));
                self.line(ind + 1, &format!("({place}).reserve({n});"));
                let i = self.fresh("i");
                let x = self.fresh("x");
                self.line(
                    ind + 1,
                    &format!("for (std::uint32_t {i} = 0; {i} < {n}; ++{i}) {{"),
                );
                self.line(ind + 2, &format!("{elem_ty} {x}{{}};"));
                self.dec(ind + 2, &x, &inner);
                self.line(ind + 2, &format!("({place}).push_back(std::move({x}));"));
                self.line(ind + 1, "}");
                self.end_dheader_dec(ind + 1, end.as_deref());
                self.line(ind, "}");
            }
            Kind::Array { elem, dims } => {
                let dh = self.has_dheader(ty);
                self.line(ind, "{");
                let end = self.begin_dheader_dec(ind + 1, dh);
                let mut cur = format!("({place})");
                let mut depth = ind + 1;
                for dim in &dims {
                    let i = self.fresh("i");
                    self.line(
                        depth,
                        &format!("for (std::size_t {i} = 0; {i} < {dim}u; ++{i}) {{"),
                    );
                    cur = format!("{cur}[{i}]");
                    depth += 1;
                }
                self.dec(depth, &cur, &elem);
                for _ in &dims {
                    depth -= 1;
                    self.line(depth, "}");
                }
                self.end_dheader_dec(ind + 1, end.as_deref());
                self.line(ind, "}");
            }
            Kind::Map { key, value, bound } => {
                let dh = self.has_dheader(ty);
                self.line(ind, "{");
                let end = self.begin_dheader_dec(ind + 1, dh);
                let n = self.fresh("n");
                self.line(
                    ind + 1,
                    &format!(
                        "std::uint32_t {n} = 0; if (!cdr2::get<std::uint32_t>(src, len, offset, {n})) return -1;"
                    ),
                );
                if bound != 0 {
                    self.line(ind + 1, &format!("if ({n} > {bound}u) return -1;"));
                }
                let (kt, vt) = (type_to_cpp(&key), type_to_cpp(&value));
                // Every entry takes at least the key and value minima.
                let min = self.min_wire(&key).saturating_add(self.min_wire(&value));
                self.bound_storage(
                    ind + 1,
                    &n,
                    &format!("(sizeof({kt}) + sizeof({vt}) + cdr2::kMapNodeLinks)"),
                    min,
                );
                self.line(ind + 1, &format!("({place}).clear();"));
                let i = self.fresh("i");
                let k = self.fresh("k");
                let v = self.fresh("v");
                self.line(
                    ind + 1,
                    &format!("for (std::uint32_t {i} = 0; {i} < {n}; ++{i}) {{"),
                );
                self.line(ind + 2, &format!("{kt} {k}{{}};"));
                self.line(ind + 2, &format!("{vt} {v}{{}};"));
                self.dec(ind + 2, &k, &key);
                self.dec(ind + 2, &v, &value);
                self.line(
                    ind + 2,
                    &format!("({place})[std::move({k})] = std::move({v});"),
                );
                self.line(ind + 1, "}");
                self.end_dheader_dec(ind + 1, end.as_deref());
                self.line(ind, "}");
            }
            Kind::Aggregate if self.v1 => {
                let e = self.fresh("e");
                let o = self.origin.clone();
                self.line(
                    ind,
                    &format!(
                        "if (int {e} = ({place}).decode_cdr1_le_at(src, len, offset, hdds_budget, {o})) return {e};"
                    ),
                );
            }
            Kind::Aggregate => {
                let e = self.fresh("e");
                self.line(
                    ind,
                    &format!(
                        "if (int {e} = ({place}).decode_cdr2_le_at(src, len, offset, hdds_budget)) return {e};"
                    ),
                );
            }
            Kind::Enum { width, literals } => {
                let w = signed_int(width);
                let t = self.fresh("t");
                // A value that is none of the literals is refused.
                let check = literal_check(&t, &literals);
                self.line(
                    ind,
                    &format!(
                        "{{ {w} {t} = 0; if (!cdr2::get<{w}>(src, len, offset, {t})) return -1; {check} {place} = static_cast<std::remove_reference_t<decltype({place})>>({t}); }}"
                    ),
                );
            }
            Kind::Bitmask { width } => {
                let w = unsigned_int(width);
                let t = self.fresh("t");
                self.line(
                    ind,
                    &format!(
                        "{{ {w} {t} = 0; if (!cdr2::get<{w}>(src, len, offset, {t})) return -1; {place} = static_cast<std::remove_reference_t<decltype({place})>>(static_cast<std::uint64_t>({t})); }}"
                    ),
                );
            }
            Kind::Bitset => {
                let t = self.fresh("t");
                self.line(
                    ind,
                    &format!(
                        "{{ std::uint64_t {t} = 0; if (!cdr2::get<std::uint64_t>(src, len, offset, {t})) return -1; ({place}).from_uint64({t}); }}"
                    ),
                );
            }
            Kind::Unknown(n) => {
                self.line(
                    ind,
                    &format!("return -1; // type `{n}` is not defined in this IDL file"),
                );
            }
        }
    }

    /// [`least_wire::min_wire`] of the encoding being emitted.
    fn min_wire(&self, ty: &IdlType) -> u64 {
        if self.v1 {
            least_wire::min_wire_xcdr1(self.idx, ty)
        } else {
            least_wire::min_wire(self.idx, ty)
        }
    }

    /// Refuse (-1) the `n` elements of `elem` bytes of a sequence or map
    /// before anything is reserved or decoded, beyond what is left of the
    /// budget of the decode (`hdds_budget.storage`: `CDR_MAX_ALLOC_RATIO`
    /// times the body for the storage of every collection of the decode
    /// together), since a C++ element can be far larger than its least wire
    /// size `min` and collections nest; the storage of elements of least
    /// wire size 0 is also charged to the `CDR_MAX_ZERO_WIRE_ELEMENTS` bytes
    /// of the decode (`hdds_budget.zero_wire_count`), since the bytes bound
    /// neither their count nor the work of decoding them.
    fn bound_storage(&mut self, ind: usize, n: &str, elem: &str, min: u64) {
        if min == 0 {
            self.line(
                ind,
                &format!("if (!hdds_budget.zero_wire_count({n}, {elem})) return -1;"),
            );
        }
        self.line(
            ind,
            &format!("if (!hdds_budget.storage({n}, {elem})) return -1;"),
        );
    }

    fn dec_scalar(&mut self, ind: usize, place: &str, s: Scalar) {
        let t = self.fresh("t");
        let stmt = match s {
            Scalar::Bool => format!(
                "{{ std::uint8_t {t} = 0; if (!cdr2::get<std::uint8_t>(src, len, offset, {t}) || {t} > 1) return -1; {place} = ({t} != 0); }}"
            ),
            Scalar::Char => format!(
                "{{ std::uint8_t {t} = 0; if (!cdr2::get<std::uint8_t>(src, len, offset, {t})) return -1; {place} = static_cast<char>({t}); }}"
            ),
            Scalar::WChar => format!(
                "{{ std::uint16_t {t} = 0; if (!cdr2::get<std::uint16_t>(src, len, offset, {t})) return -1; {place} = static_cast<wchar_t>({t}); }}"
            ),
            Scalar::Num(w) => format!(
                "{{ {w} {t} = 0; if (!cdr2::get<{w}>(src, len, offset, {t})) return -1; {place} = {t}; }}"
            ),
        };
        self.line(ind, &stmt);
    }

    /// Read a DHEADER when `dh`; the variable holding the end it delimits.
    fn begin_dheader_dec(&mut self, ind: usize, dh: bool) -> Option<String> {
        if !dh {
            return None;
        }
        let e = self.fresh("end");
        self.line(
            ind,
            &format!("std::size_t {e} = 0; if (!cdr2::get_len(src, len, offset, {e})) return -1;"),
        );
        Some(e)
    }

    fn end_dheader_dec(&mut self, ind: usize, end: Option<&str>) {
        if let Some(e) = end {
            self.line(ind, &format!("if (!cdr2::skip_to(offset, {e})) return -1;"));
        }
    }
}

/// `switch` refusing (`return -1`) a value of `var` that is none of
/// `literals` (an enumeration read from the wire).
fn literal_check(var: &str, literals: &[i64]) -> String {
    let mut values: Vec<i64> = literals.to_vec();
    values.sort_unstable();
    values.dedup();
    let mut cases = String::new();
    for v in values {
        let _ = write!(cases, "case {v}: ");
    }
    format!("switch ({var}) {{ {cases}break; default: return -1; }}")
}

/// The four public entry points around the bodies of `encode_cdr2_le_at`
/// and `decode_cdr2_le_at` (bodies indented one level below `indent`).
fn entry_points(
    indent: &str,
    kind: &str,
    ext: Ext,
    type_name: &str,
    enc_body: &str,
    dec_body: &str,
    xcdr1_body: &str,
) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{indent}/// Extensibility of this {kind} (DDS-XTypes 1.3 7.2.2.4.4), as the hdds C\n\
         {indent}/// API numbers it: 0 FINAL, 1 APPENDABLE, 2 MUTABLE. The hdds C++ SDK's\n\
         {indent}/// create_writer<T>() labels every sample with it (Table 60: CDR2_LE,\n\
         {indent}/// D_CDR2_LE, PL_CDR2_LE).\n\
         {indent}static constexpr std::uint32_t hdds_extensibility = {}u;\n\
         {indent}/// DDS type name of this {kind}: its IDL qualified name (`Mod::Sub::Name`),\n\
         {indent}/// the name the Rust backend gives the same type.\n\
         {indent}static constexpr const char* hdds_type_name = \"{type_name}\";\n",
        ext.hdds_code()
    );
    let _ = writeln!(
        out,
        "{indent}/// Encode this {kind} as XCDR2 little endian (DDS-XTypes 1.3 7.4.3.5.3) at\n\
         {indent}/// `dst + offset`, alignment measured from `dst`; advances `offset`.\n\
         {indent}/// Returns 0 on success, negative on error (buffer too small, a bound\n\
         {indent}/// exceeded, a value the wire cannot carry)."
    );
    let _ = writeln!(
        out,
        "{indent}[[nodiscard]] int encode_cdr2_le_at(std::uint8_t* dst, std::size_t len, std::size_t& offset) const noexcept {{"
    );
    // An empty FINAL structure is zero bytes on the wire: its body uses no
    // parameter (`-Werror=unused-parameter`).
    if enc_body.trim().is_empty() {
        let _ = writeln!(out, "{indent}    (void)dst; (void)len; (void)offset;");
    }
    out.push_str(enc_body);
    let _ = writeln!(out, "{indent}    return 0;");
    let _ = writeln!(out, "{indent}}}\n");
    let _ = writeln!(
        out,
        "{indent}/// Encode from the start of `dst`: the number of bytes written, or -1."
    );
    let _ = writeln!(
        out,
        "{indent}[[nodiscard]] int encode_cdr2_le(std::uint8_t* dst, std::size_t len) const noexcept {{"
    );
    let _ = writeln!(out, "{indent}    std::size_t offset = 0;");
    let _ = writeln!(
        out,
        "{indent}    if (encode_cdr2_le_at(dst, len, offset) != 0) return -1;"
    );
    let _ = writeln!(out, "{indent}    return static_cast<int>(offset);");
    let _ = writeln!(out, "{indent}}}\n");
    let _ = writeln!(
        out,
        "{indent}/// Decode this {kind} from XCDR2 little endian at `src + offset`,\n\
         {indent}/// alignment measured from `src`; advances `offset`. Returns 0 on\n\
         {indent}/// success, negative on malformed or truncated input."
    );
    let _ = writeln!(
        out,
        "{indent}[[nodiscard]] int decode_cdr2_le_at(const std::uint8_t* src, std::size_t len, std::size_t& offset) noexcept {{"
    );
    let _ = writeln!(
        out,
        "{indent}    cdr2::Budget hdds_budget = cdr2::Budget::for_body(offset < len ? len - offset : 0);"
    );
    let _ = writeln!(
        out,
        "{indent}    return decode_cdr2_le_at(src, len, offset, hdds_budget);"
    );
    let _ = writeln!(out, "{indent}}}\n");
    let _ = writeln!(
        out,
        "{indent}/// Decode as `decode_cdr2_le_at`, the storage charged to `hdds_budget`\n\
         {indent}/// (the budget of the enclosing decode, shared by its nested values)."
    );
    let _ = writeln!(
        out,
        "{indent}[[nodiscard]] int decode_cdr2_le_at(const std::uint8_t* src, std::size_t len, std::size_t& offset, cdr2::Budget& hdds_budget) noexcept {{"
    );
    let _ = writeln!(out, "{indent}    (void)hdds_budget;");
    if dec_body.trim().is_empty() {
        let _ = writeln!(out, "{indent}    (void)src; (void)len; (void)offset;");
    }
    out.push_str(dec_body);
    let _ = writeln!(out, "{indent}    return 0;");
    let _ = writeln!(out, "{indent}}}\n");
    let _ = writeln!(
        out,
        "{indent}/// Decode from the start of `src`: the number of bytes read, or -1."
    );
    let _ = writeln!(
        out,
        "{indent}[[nodiscard]] int decode_cdr2_le(const std::uint8_t* src, std::size_t len) noexcept {{"
    );
    let _ = writeln!(out, "{indent}    std::size_t offset = 0;");
    let _ = writeln!(
        out,
        "{indent}    if (decode_cdr2_le_at(src, len, offset) != 0) return -1;"
    );
    let _ = writeln!(out, "{indent}    return static_cast<int>(offset);");
    let _ = writeln!(out, "{indent}}}");
    out.push_str(&xcdr1_entry_points(indent, kind, xcdr1_body));
    out
}

/// The XCDR1 decoders (DDS-XTypes 1.3 7.4.1) around `xcdr1_body`, and the
/// decoder picked from the encapsulation identifier of a sample.
fn xcdr1_entry_points(indent: &str, kind: &str, xcdr1_body: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "\n{indent}/// Decode this {kind} from XCDR1 little endian (DDS-XTypes 1.3 7.4.1: the\n\
         {indent}/// body of a CDR_LE or PL_CDR_LE sample) at `src + offset`, alignment\n\
         {indent}/// measured from `src`; advances `offset`. Returns 0 on success,\n\
         {indent}/// negative on malformed or truncated input."
    );
    let _ = writeln!(
        out,
        "{indent}[[nodiscard]] int decode_cdr1_le_at(const std::uint8_t* src, std::size_t len, std::size_t& offset) noexcept {{"
    );
    let _ = writeln!(
        out,
        "{indent}    cdr2::Budget hdds_budget = cdr2::Budget::for_body(offset < len ? len - offset : 0);"
    );
    let _ = writeln!(
        out,
        "{indent}    return decode_cdr1_le_at(src, len, offset, hdds_budget, cdr2::X1{{0}});"
    );
    let _ = writeln!(out, "{indent}}}\n");
    let _ = writeln!(
        out,
        "{indent}/// Decode as `decode_cdr1_le_at`, the storage charged to `hdds_budget`,\n\
         {indent}/// alignment measured from the origin of `hdds_x1` (the enclosing\n\
         {indent}/// decode's)."
    );
    let _ = writeln!(
        out,
        "{indent}[[nodiscard]] int decode_cdr1_le_at(const std::uint8_t* src, std::size_t len, std::size_t& offset, cdr2::Budget& hdds_budget, cdr2::X1 hdds_x1) noexcept {{"
    );
    let _ = writeln!(out, "{indent}    (void)hdds_budget; (void)hdds_x1;");
    if xcdr1_body.trim().is_empty() {
        let _ = writeln!(out, "{indent}    (void)src; (void)len; (void)offset;");
    }
    out.push_str(xcdr1_body);
    let _ = writeln!(out, "{indent}    return 0;");
    let _ = writeln!(out, "{indent}}}\n");
    let _ = writeln!(
        out,
        "{indent}/// Decode XCDR1 from the start of `src`: the number of bytes read, or -1."
    );
    let _ = writeln!(
        out,
        "{indent}[[nodiscard]] int decode_cdr1_le(const std::uint8_t* src, std::size_t len) noexcept {{"
    );
    let _ = writeln!(out, "{indent}    std::size_t offset = 0;");
    let _ = writeln!(
        out,
        "{indent}    if (decode_cdr1_le_at(src, len, offset) != 0) return -1;"
    );
    let _ = writeln!(out, "{indent}    return static_cast<int>(offset);");
    let _ = writeln!(out, "{indent}}}\n");
    let _ = writeln!(
        out,
        "{indent}/// Decode the body of a sample whose encapsulation identifier is\n\
         {indent}/// `encapsulation_id` (DDS-XTypes 1.3 Table 60): CDR_LE (0x0001) and\n\
         {indent}/// PL_CDR_LE (0x0003) as XCDR1, CDR2_LE (0x0007), D_CDR2_LE (0x0009) and\n\
         {indent}/// PL_CDR2_LE (0x000b) as XCDR2, anything else refused. The number of\n\
         {indent}/// bytes read, or -1."
    );
    let _ = writeln!(
        out,
        "{indent}[[nodiscard]] int decode_by_encapsulation(std::uint16_t encapsulation_id, const std::uint8_t* src, std::size_t len) noexcept {{"
    );
    let _ = writeln!(out, "{indent}    switch (encapsulation_id) {{");
    let _ = writeln!(
        out,
        "{indent}    case 0x0001u: case 0x0003u: return decode_cdr1_le(src, len);"
    );
    let _ = writeln!(
        out,
        "{indent}    case 0x0007u: case 0x0009u: case 0x000bu: return decode_cdr2_le(src, len);"
    );
    let _ = writeln!(out, "{indent}    default: return -1;");
    let _ = writeln!(out, "{indent}    }}");
    let _ = writeln!(out, "{indent}}}");
    out
}

/// `bool last_ ...; get_pl1(...)`: the next XCDR1 parameter's header.
fn read_param(g: &mut Gen<'_>, ind: usize) {
    g.line(
        ind,
        "bool last_ = false; bool mu_ = false; std::uint32_t id_ = 0; std::size_t mend_ = 0;",
    );
    g.line(
        ind,
        "if (!cdr2::get_pl1(src, len, offset, last_, id_, mu_, mend_)) return -1;",
    );
}

/// `cdr2::X1 x1_N{offset};`: the origin of a parameter's value (its first
/// byte), made current; returns the origin to restore.
fn param_origin(g: &mut Gen<'_>, ind: usize) -> String {
    let x = g.fresh("x1_");
    g.line(ind, &format!("cdr2::X1 {x}{{offset}}; (void){x};"));
    std::mem::replace(&mut g.origin, x)
}

/// The XCDR1 decoder body of a structure (DDS-XTypes 1.3 7.4.1): FINAL and
/// APPENDABLE alike, no DHEADER (a member of an APPENDABLE structure past
/// the end of the data at its default, 7.4.1.2.3), an optional member as a
/// parameter (rule 19); MUTABLE as a parameter list closed by
/// `PID_LIST_END` (rules 24-25), members by id, an unknown one skipped
/// unless it must be understood, a missing one at its default.
fn struct_dec1(
    types: &TypeIndex,
    chain: &[ChainMember],
    ext: Ext,
    body: &str,
    fastdds_compat: bool,
) -> String {
    let mut g = Gen::new_v1(types, body);
    match ext {
        Ext::Final | Ext::Appendable => {
            let app = ext == Ext::Appendable;
            for m in chain {
                let place = member_place(m, fastdds_compat);
                let ind = if app {
                    let reset = default_assign(&g, &place, m);
                    g.line(0, &format!("if (offset >= len) {{ {reset} }} else {{"));
                    1
                } else {
                    0
                };
                if m.field.is_optional() {
                    g.line(ind, "{");
                    read_param(&mut g, ind + 1);
                    g.line(ind + 1, "(void)id_; (void)mu_;");
                    g.line(ind + 1, "if (last_) return -1;");
                    g.line(ind + 1, "if (mend_ > offset) {");
                    let saved = param_origin(&mut g, ind + 2);
                    let v = g.fresh("v");
                    let ty = type_to_cpp(&m.field.field_type);
                    g.line(ind + 2, &format!("{ty} {v}{{}};"));
                    g.dec(ind + 2, &v, &m.field.field_type);
                    g.line(ind + 2, &format!("{place} = std::move({v});"));
                    g.origin = saved;
                    g.line(ind + 1, &format!("}} else {{ {place}.reset(); }}"));
                    g.line(ind + 1, "if (!cdr2::skip_to(offset, mend_)) return -1;");
                    g.line(ind, "}");
                } else {
                    g.dec(ind, &place, &m.field.field_type);
                }
                if app {
                    g.line(0, "}");
                }
            }
        }
        Ext::Mutable => {
            for m in chain {
                let place = member_place(m, fastdds_compat);
                let reset = default_assign(&g, &place, m);
                g.line(0, &reset);
            }
            g.line(0, "for (;;) {");
            read_param(&mut g, 1);
            g.line(1, "if (last_) break;");
            let saved = param_origin(&mut g, 1);
            g.line(1, "switch (id_) {");
            for m in chain {
                let place = member_place(m, fastdds_compat);
                g.line(1, &format!("case {:#x}u: {{ // {}", m.id, m.field.name));
                if m.field.is_optional() {
                    let v = g.fresh("v");
                    let ty = type_to_cpp(&m.field.field_type);
                    g.line(2, &format!("{ty} {v}{{}};"));
                    g.dec(2, &v, &m.field.field_type);
                    g.line(2, &format!("{place} = std::move({v});"));
                } else {
                    g.dec(2, &place, &m.field.field_type);
                }
                g.line(2, "break;");
                g.line(1, "}");
            }
            g.line(1, "default: if (mu_) return -1; break;");
            g.line(1, "}");
            g.origin = saved;
            g.line(1, "if (!cdr2::skip_to(offset, mend_)) return -1;");
            g.line(0, "}");
        }
    }
    g.out
}

/// `this->member` of a chain member (base members are inherited; in the
/// FastDDS-compatible flavour members are `m_`-prefixed and `protected`).
fn member_place(m: &ChainMember, fastdds_compat: bool) -> String {
    let ident = cpp_ident(&m.field.name);
    if fastdds_compat {
        format!("this->m_{ident}")
    } else {
        format!("this->{ident}")
    }
}

/// The statement giving the member at `place` its default value: no value
/// for an optional member (`reset()`: assigning an empty `std::optional`
/// makes g++ warn `-Wmaybe-uninitialized` on the moved-from payload under
/// `-O1 -fsanitize=undefined`), the first enumerator for an enumeration
/// (DDS-XTypes 1.3 Table 9, spec text line 2341; `@default_literal` is not
/// recorded by the parser), the
/// value-initialized object otherwise (zero, empty, a nested structure's
/// own defaults).
fn default_assign(g: &Gen<'_>, place: &str, m: &ChainMember) -> String {
    if m.field.is_optional() {
        return format!("{place}.reset();");
    }
    let ty = format!("std::remove_reference_t<decltype({place})>");
    match g.kind(&m.field.field_type) {
        Kind::Enum { literals, .. } if !literals.is_empty() => {
            format!("{place} = static_cast<{ty}>({});", literals[0])
        }
        _ => format!("{place} = {ty}{{}};"),
    }
}

/// Codec of a structure: the public entry points (`indent` is the member
/// indentation). A structure whose base chain cannot be resolved gets a
/// `static_assert` naming the reason.
pub(super) fn struct_codec(
    idx: &DefinitionIndex,
    s: &Struct,
    indent: &str,
    fastdds_compat: bool,
    type_name: &str,
) -> String {
    struct_codec_impl(idx, s, indent, fastdds_compat, false, type_name)
        + &struct_codec_impl(idx, s, indent, fastdds_compat, true, type_name)
}

#[allow(clippy::too_many_lines)]
fn struct_codec_impl(
    idx: &DefinitionIndex,
    s: &Struct,
    indent: &str,
    fastdds_compat: bool,
    key_only: bool,
    type_name: &str,
) -> String {
    let types = &idx.keys;
    let chain = match member_chain(types, s) {
        Ok(c) => c,
        Err(reason) => {
            return format!(
                "{indent}static_assert(sizeof(int) == 0, \"hddsgen: {}: {}\");\n",
                s.name,
                reason.replace('"', "'")
            );
        }
    };
    let chain = if key_only {
        data_model::key_members(chain)
    } else {
        chain
    };
    let ext = data_model::struct_ext(s);
    let body = format!("{indent}    ");

    // ---- encode ----
    let mut g = Gen::new(types, &body);
    g.key_only = key_only;
    match ext {
        Ext::Final | Ext::Appendable => {
            // Rule 30 (line 7013): an APPENDABLE structure is its DHEADER,
            // then the FINAL form.
            let d = g.begin_dheader_enc(0, ext == Ext::Appendable);
            for m in &chain {
                let place = member_place(m, fastdds_compat);
                if m.field.is_optional() {
                    // Rule 20: a presence flag, then the value when present.
                    g.line(
                        0,
                        &format!(
                            "if (!cdr2::put<std::uint8_t>(dst, len, offset, static_cast<std::uint8_t>({place}.has_value() ? 1 : 0))) return -1;"
                        ),
                    );
                    g.line(0, &format!("if ({place}.has_value()) {{"));
                    g.enc(1, &format!("(*{place})"), &m.field.field_type);
                    g.line(0, "}");
                } else {
                    g.enc(0, &place, &m.field.field_type);
                }
            }
            g.end_dheader_enc(0, d.as_deref());
        }
        Ext::Mutable => {
            // Rules 21-22: DHEADER, then per member an EMHEADER1 (aligned at
            // 4), a NEXTINT for length code 4, the value.
            let d = g.begin_dheader_enc(0, true);
            for m in &chain {
                emit_mutable_member(&mut g, m, fastdds_compat);
            }
            g.end_dheader_enc(0, d.as_deref());
        }
    }
    let enc_body = std::mem::take(&mut g.out);

    if key_only {
        return key_entry_points(indent, &enc_body);
    }

    // ---- decode ----
    match ext {
        Ext::Final | Ext::Appendable => {
            let end = g.begin_dheader_dec(0, ext == Ext::Appendable);
            for m in &chain {
                let place = member_place(m, fastdds_compat);
                // APPENDABLE: a member after the DHEADER end is absent from
                // an older writer's type and takes its default value, an
                // optional one no value (7.4.1.2.3, spec text lines
                // 6195-6203; Table 9 / lines 3390-3393).
                let ind = end.as_deref().map_or(0, |e| {
                    let reset = default_assign(&g, &place, m);
                    g.line(0, &format!("if (offset >= {e}) {{ {reset} }} else {{"));
                    1
                });
                if m.field.is_optional() {
                    let t = g.fresh("t");
                    let v = g.fresh("v");
                    let ty = type_to_cpp(&m.field.field_type);
                    g.line(
                        ind,
                        &format!(
                            "{{ std::uint8_t {t} = 0; if (!cdr2::get<std::uint8_t>(src, len, offset, {t}) || {t} > 1) return -1;"
                        ),
                    );
                    g.line(
                        ind + 1,
                        &format!("if ({t} == 0) {{ {place}.reset(); }} else {{"),
                    );
                    g.line(ind + 2, &format!("{ty} {v}{{}};"));
                    g.dec(ind + 2, &v, &m.field.field_type);
                    g.line(ind + 2, &format!("{place} = std::move({v});"));
                    g.line(ind + 1, "}");
                    g.line(ind, "}");
                } else {
                    g.dec(ind, &place, &m.field.field_type);
                }
                if end.is_some() {
                    g.line(0, "}");
                }
            }
            // An appendable reader skips what a newer writer appended.
            g.end_dheader_dec(0, end.as_deref());
        }
        Ext::Mutable => {
            // A member absent from the data takes its default value, an
            // optional one no value (7.4.1.2.3, spec text lines 6195-6203):
            // never what the object held.
            for m in &chain {
                let place = member_place(m, fastdds_compat);
                let reset = default_assign(&g, &place, m);
                g.line(0, &reset);
            }
            g.line(
                0,
                "std::size_t end_ = 0; if (!cdr2::get_len(src, len, offset, end_)) return -1;",
            );
            g.line(0, "while (offset < end_) {");
            g.line(
                1,
                "bool mu_ = false; std::uint32_t id_ = 0; std::size_t mend_ = 0;",
            );
            g.line(
                1,
                "if (!cdr2::get_emheader(src, len, offset, end_, mu_, id_, mend_)) return -1;",
            );
            g.line(1, "switch (id_) {");
            for m in &chain {
                let place = member_place(m, fastdds_compat);
                g.line(1, &format!("case {:#x}u: {{ // {}", m.id, m.field.name));
                if m.field.is_optional() {
                    let v = g.fresh("v");
                    let ty = type_to_cpp(&m.field.field_type);
                    g.line(2, &format!("{ty} {v}{{}};"));
                    g.dec(2, &v, &m.field.field_type);
                    g.line(2, &format!("{place} = std::move({v});"));
                } else {
                    g.dec(2, &place, &m.field.field_type);
                }
                g.line(2, "break;");
                g.line(1, "}");
            }
            // An unknown member is skipped, unless it must be understood.
            g.line(1, "default: if (mu_) return -1; break;");
            g.line(1, "}");
            g.line(1, "if (!cdr2::skip_to(offset, mend_)) return -1;");
            g.line(0, "}");
            g.line(0, "if (!cdr2::skip_to(offset, end_)) return -1;");
        }
    }
    let dec_body = g.out;
    let xcdr1_body = struct_dec1(types, &chain, ext, &body, fastdds_compat);
    entry_points(
        indent,
        "struct",
        ext,
        type_name,
        &enc_body,
        &dec_body,
        &xcdr1_body,
    )
}

/// One member of a MUTABLE structure (rule 22): `EMHEADER1` with the length
/// code of `data_model::length_code` and the M flag of
/// `data_model::must_understand`, a NEXTINT for length code 4 only (for 5
/// to 7 the value's own leading length word is the NEXTINT), the value. An
/// absent optional member is left out.
fn emit_mutable_member(g: &mut Gen<'_>, m: &ChainMember, fastdds_compat: bool) {
    let place = member_place(m, fastdds_compat);
    if m.field.is_optional() {
        g.line(0, &format!("if ({place}.has_value()) {{"));
        emit_member_value(g, 1, m, &format!("(*{place})"));
        g.line(0, "}");
    } else {
        emit_member_value(g, 0, m, &place);
    }
}

/// `EMHEADER1`, NEXTINT for length code 4, then the value at `value`.
fn emit_member_value(g: &mut Gen<'_>, ind: usize, m: &ChainMember, value: &str) {
    let lc = length_code(g.idx, &m.field.field_type);
    let mu = must_understand(&m.field);
    g.line(
        ind,
        &format!(
            "if (!cdr2::put_emheader(dst, len, offset, {mu}, {lc}u, {:#x}u)) return -1; // {}",
            m.id, m.field.name
        ),
    );
    if lc == 4 {
        let n = g.fresh("nx");
        g.line(ind, "{");
        g.line(
            ind + 1,
            &format!(
                "std::size_t {n} = 0; if (!cdr2::begin_len(dst, len, offset, {n})) return -1;"
            ),
        );
        g.enc(ind + 1, value, &m.field.field_type);
        g.line(
            ind + 1,
            &format!("if (!cdr2::end_len(dst, offset, {n})) return -1;"),
        );
        g.line(ind, "}");
    } else {
        g.enc(ind, value, &m.field.field_type);
    }
}

/// The discriminator's wire type and how `_d` converts to and from it:
/// enumerations at their `@bit_bound` width, octet / char / boolean 1 byte,
/// short 2, long 4, long long 8.
/// An enumeration discriminator also gives its literal values.
fn disc_wire(idx: &TypeIndex, ty: &IdlType) -> (&'static str, Option<Scalar>, Option<Vec<i64>>) {
    match idx.resolve(ty) {
        IdlType::Primitive(p) => match scalar_of(&p) {
            Some(Scalar::Num(w)) => (w, None, None),
            Some(Scalar::Bool) => ("std::uint8_t", Some(Scalar::Bool), None),
            Some(Scalar::Char) => ("std::uint8_t", Some(Scalar::Char), None),
            Some(Scalar::WChar) => ("std::uint16_t", Some(Scalar::WChar), None),
            None => ("std::int32_t", None, None),
        },
        IdlType::Named(n) => match idx.named(&n) {
            Some(Named::Enum {
                bit_bound,
                literals,
            }) => (
                signed_int(enum_width(*bit_bound)),
                None,
                Some(literals.iter().map(|(_, v)| *v).collect()),
            ),
            _ => ("std::int32_t", None, None),
        },
        _ => ("std::int32_t", None, None),
    }
}

/// Codec of a union. FINAL / APPENDABLE (rules 26, 29, 30): the
/// discriminator at its type's width, then the selected member; an
/// APPENDABLE union is preceded by its DHEADER. MUTABLE (rule 27, line
/// 6986): a DHEADER, the discriminator as member 0 (its `EMHEADER1` with
/// the M flag: line 2299), then the selected case as a member with its id
/// of `data_model::union_case_ids`. `direct`: the members are fields of the
/// struct (a member type is not trivially copyable), otherwise they live in
/// the anonymous `_u`.
pub(in crate::codegen::cpp) fn union_codec(
    idx: &DefinitionIndex,
    u: &Union,
    indent: &str,
    direct: bool,
    type_name: &str,
) -> String {
    let mut out = union_codec_impl(idx, u, indent, direct, false, type_name);
    let mut key_union = u.clone();
    if u.discriminator_is_key() {
        key_union.cases.clear();
    }
    out.push_str(&union_codec_impl(
        idx, &key_union, indent, direct, true, type_name,
    ));
    out
}

fn key_entry_points(indent: &str, body: &str) -> String {
    format!("{indent}[[nodiscard]] int encode_key_cdr2_le_at(std::uint8_t* dst, std::size_t len, std::size_t& offset) const noexcept {{\n{indent}    (void)dst; (void)len; (void)offset;\n{body}{indent}    return 0;\n{indent}}}\n{indent}[[nodiscard]] int encode_key_cdr2_le(std::uint8_t* dst, std::size_t len) const noexcept {{\n{indent}    std::size_t offset = 0;\n{indent}    if (encode_key_cdr2_le_at(dst, len, offset)) return -1;\n{indent}    return static_cast<int>(offset);\n{indent}}}\n")
}

#[allow(clippy::too_many_lines)]
fn union_codec_impl(
    idx: &DefinitionIndex,
    u: &Union,
    indent: &str,
    direct: bool,
    key_only: bool,
    type_name: &str,
) -> String {
    let types = &idx.keys;
    let ext = union_ext(u);
    let mutable = ext == Ext::Mutable;
    let ids = data_model::union_case_ids(&idx.keys, u);
    let disc_lc = data_model::discriminator_length_code(types, &u.discriminator);
    let (wire, conv, literals) = disc_wire(types, &u.discriminator);
    let body = format!("{indent}    ");
    let place_of = |name: &str| {
        if direct {
            format!("this->{name}")
        } else {
            format!("this->_u.{name}")
        }
    };
    let is_void = |ty: &IdlType| matches!(ty, IdlType::Primitive(PrimitiveType::Void));
    let has_default = u
        .cases
        .iter()
        .any(|c| c.labels.contains(&UnionLabel::Default));
    let labels = |g: &mut Gen<'_>, ind: usize, labels: &[UnionLabel]| {
        for label in labels {
            match label {
                UnionLabel::Value(v) => {
                    let l = super::super::unions::label_to_cpp(idx, &u.discriminator, v);
                    g.line(ind, &format!("case {l}:"));
                }
                UnionLabel::Default => g.line(ind, "default:"),
            }
        }
    };

    // A boolean discriminator is switched on as an int (-Wswitch-bool).
    let switch_d = if matches!(conv, Some(Scalar::Bool)) {
        "switch (static_cast<int>(this->_d)) {"
    } else {
        "switch (this->_d) {"
    };

    // ---- encode ----
    let mut g = Gen::new(types, &body);
    g.key_only = key_only;
    let d = g.begin_dheader_enc(0, ext != Ext::Final);
    let disc = match conv {
        Some(Scalar::Bool) => "static_cast<std::uint8_t>(this->_d ? 1 : 0)".to_string(),
        Some(Scalar::WChar) => {
            g.line(
                0,
                "if (static_cast<std::uint32_t>(this->_d) > 0xFFFFu) return -1;",
            );
            format!("static_cast<{wire}>(this->_d)")
        }
        _ => format!("static_cast<{wire}>(this->_d)"),
    };
    if mutable {
        // The discriminator always must be understood.
        g.line(
            0,
            &format!(
                "if (!cdr2::put_emheader(dst, len, offset, true, {disc_lc}u, 0u)) return -1; // discriminator"
            ),
        );
    }
    g.line(
        0,
        &format!("if (!cdr2::put<{wire}>(dst, len, offset, {disc})) return -1;"),
    );
    g.line(0, switch_d);
    for (i, case) in u.cases.iter().enumerate() {
        labels(&mut g, 0, &case.labels);
        g.line(0, "{");
        if !is_void(&case.field.field_type) {
            let place = place_of(&case.field.name);
            if mutable {
                let m = ChainMember {
                    field: case.field.clone(),
                    owner: u.name.clone(),
                    base_depth: 0,
                    id: ids[i],
                };
                emit_member_value(&mut g, 1, &m, &place);
            } else {
                g.enc(1, &place, &case.field.field_type);
            }
        }
        g.line(1, "break;");
        g.line(0, "}");
    }
    if !has_default {
        // No member selected: the discriminator alone.
        g.line(0, "default: break;");
    }
    g.line(0, "}");
    g.end_dheader_enc(0, d.as_deref());
    let enc_body = std::mem::take(&mut g.out);

    if key_only {
        return key_entry_points(indent, &enc_body);
    }

    // ---- decode ----
    let assign = match conv {
        Some(Scalar::Bool) => "this->_d = (d_ != 0);".to_string(),
        _ => "this->_d = static_cast<std::remove_reference_t<decltype(this->_d)>>(d_);".to_string(),
    };
    let check = if matches!(conv, Some(Scalar::Bool)) {
        " || d_ > 1"
    } else {
        ""
    };
    // An enumeration discriminator that is none of its literals is refused.
    let lit = literals
        .as_deref()
        .map(|l| format!("{} ", literal_check("d_", l)))
        .unwrap_or_default();
    let read_disc = format!(
        "{{ {wire} d_ = 0; if (!cdr2::get<{wire}>(src, len, offset, d_){check}) return -1; {lit}{assign} }}"
    );
    if mutable {
        // Members by id; the discriminator (member 0) is required, and the
        // case member read must be the one it selects.
        g.line(0, "bool disc_ = false; std::int64_t got_ = -1;");
        g.line(
            0,
            "std::size_t end_ = 0; if (!cdr2::get_len(src, len, offset, end_)) return -1;",
        );
        g.line(0, "while (offset < end_) {");
        g.line(
            1,
            "bool mu_ = false; std::uint32_t id_ = 0; std::size_t mend_ = 0;",
        );
        g.line(
            1,
            "if (!cdr2::get_emheader(src, len, offset, end_, mu_, id_, mend_)) return -1;",
        );
        g.line(1, "switch (id_) {");
        g.line(1, &format!("case 0u: {read_disc} disc_ = true; break;"));
        for (i, case) in u.cases.iter().enumerate() {
            if is_void(&case.field.field_type) {
                continue;
            }
            g.line(
                1,
                &format!("case {:#x}u: {{ // {}", ids[i], case.field.name),
            );
            g.dec(2, &place_of(&case.field.name), &case.field.field_type);
            g.line(2, &format!("got_ = {};", ids[i]));
            g.line(2, "break;");
            g.line(1, "}");
        }
        g.line(1, "default: if (mu_) return -1; break;");
        g.line(1, "}");
        g.line(1, "if (!cdr2::skip_to(offset, mend_)) return -1;");
        g.line(0, "}");
        g.line(0, "if (!cdr2::skip_to(offset, end_) || !disc_) return -1;");
        g.line(0, "std::int64_t want_ = -1;");
        g.line(0, switch_d);
        for (i, case) in u.cases.iter().enumerate() {
            labels(&mut g, 0, &case.labels);
            if is_void(&case.field.field_type) {
                g.line(1, "break;");
            } else {
                g.line(1, &format!("want_ = {}; break;", ids[i]));
            }
        }
        if !has_default {
            g.line(0, "default: break;");
        }
        g.line(0, "}");
        g.line(0, "if (got_ != want_) return -1;");
    } else {
        let end = g.begin_dheader_dec(0, ext == Ext::Appendable);
        g.line(0, &read_disc);
        g.line(0, switch_d);
        for case in &u.cases {
            labels(&mut g, 0, &case.labels);
            g.line(0, "{");
            if !is_void(&case.field.field_type) {
                g.dec(1, &place_of(&case.field.name), &case.field.field_type);
            }
            g.line(1, "break;");
            g.line(0, "}");
        }
        if !has_default {
            g.line(0, "default: break;");
        }
        g.line(0, "}");
        g.end_dheader_dec(0, end.as_deref());
    }
    let dec_body = g.out;

    // ---- XCDR1 decode (DDS-XTypes 1.3 7.4.1): FINAL and APPENDABLE alike
    // (no DHEADER), MUTABLE as the parameter list of the discriminator
    // (member 0) and the selected case, checked as under XCDR2 ----
    let mut g = Gen::new_v1(types, &body);
    if mutable {
        g.line(0, "bool disc_ = false; std::int64_t got_ = -1;");
        g.line(0, "for (;;) {");
        read_param(&mut g, 1);
        g.line(1, "if (last_) break;");
        let saved = param_origin(&mut g, 1);
        g.line(1, "switch (id_) {");
        g.line(1, &format!("case 0u: {read_disc} disc_ = true; break;"));
        for (i, case) in u.cases.iter().enumerate() {
            if is_void(&case.field.field_type) {
                continue;
            }
            g.line(
                1,
                &format!("case {:#x}u: {{ // {}", ids[i], case.field.name),
            );
            g.line(2, "if (got_ != -1) return -1;");
            g.dec(2, &place_of(&case.field.name), &case.field.field_type);
            g.line(2, &format!("got_ = {};", ids[i]));
            g.line(2, "break;");
            g.line(1, "}");
        }
        g.line(1, "default: if (mu_) return -1; break;");
        g.line(1, "}");
        g.origin = saved;
        g.line(1, "if (!cdr2::skip_to(offset, mend_)) return -1;");
        g.line(0, "}");
        g.line(0, "if (!disc_) return -1;");
        g.line(0, "std::int64_t want_ = -1;");
        g.line(0, switch_d);
        for (i, case) in u.cases.iter().enumerate() {
            labels(&mut g, 0, &case.labels);
            if is_void(&case.field.field_type) {
                g.line(1, "break;");
            } else {
                g.line(1, &format!("want_ = {}; break;", ids[i]));
            }
        }
        if !has_default {
            g.line(0, "default: break;");
        }
        g.line(0, "}");
        g.line(0, "if (got_ != want_) return -1;");
    } else {
        g.line(0, &read_disc);
        g.line(0, switch_d);
        for case in &u.cases {
            labels(&mut g, 0, &case.labels);
            g.line(0, "{");
            if !is_void(&case.field.field_type) {
                g.dec(1, &place_of(&case.field.name), &case.field.field_type);
            }
            g.line(1, "break;");
            g.line(0, "}");
        }
        if !has_default {
            g.line(0, "default: break;");
        }
        g.line(0, "}");
    }
    let xcdr1_body = g.out;
    entry_points(
        indent,
        "union",
        ext,
        type_name,
        &enc_body,
        &dec_body,
        &xcdr1_body,
    )
}
