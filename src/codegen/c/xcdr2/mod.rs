// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! XCDR2 codec of the generated C and C-micro types (DDS-`XTypes` 1.3
//! 7.4.3.5.3, "Complete Serialization Rules").
//!
//! Every structure and union gets an encoder and a decoder working on the
//! cursor of [`runtime`]: `(const T* v, W* c)` and `(T* v, R* c)`. The
//! cursor position is measured from the start of the buffer the public
//! entry point received, so a nested value is aligned where it lands in
//! the enclosing stream, never relative to its own first byte.
//!
//! What the specification fixes at generation time (extensibility, member
//! ids, length codes, DHEADERs of collections, enumeration and bitmask
//! widths, the must-understand flag) comes from
//! [`crate::codegen::data_model`]:
//!
//! - FINAL: the members in order, base members first (rules 6-7);
//! - APPENDABLE: a DHEADER, then the FINAL form (rule 30); the decoder skips
//!   what follows the members it knows, up to the DHEADER's end;
//! - MUTABLE: a DHEADER, then per member an aligned `EMHEADER1` (M flag,
//!   length code, member id), a NEXTINT when the length code is 4, the
//!   value (rules 21-22); absent optional members are omitted; the decoder
//!   dispatches on the member id and skips an unknown member unless it
//!   carries the M flag;
//! - an optional member of a FINAL or APPENDABLE structure: a presence byte,
//!   then the value when present (rule 20);
//! - a sequence, array or map of non-primitive elements: a DHEADER first
//!   (rules 9, 12, 15);
//! - a union: the discriminator at its type's width, then the selected
//!   member; an APPENDABLE union has a DHEADER (rules 26, 30); a MUTABLE
//!   union is a DHEADER, the discriminator as member 0 and the selected
//!   case as a member (rule 27).
//!
//! The code it emits is C89 (declarations first, no `//` comments) with
//! `<stdint.h>`.

pub mod runtime;
mod xcdr1;

pub use xcdr1::{prototype_dec1, struct_dec1, union_dec1};

use std::fmt::Write as _;

use super::helpers::{c_name, last_ident};
use super::CGenerator;
use crate::ast::{Field, Struct, Union, UnionLabel};
use crate::codegen::data_model::{
    self, bitmask_width, collection_has_dheader, enum_width, flatten_array, length_code,
    member_chain, must_understand, union_ext, ChainMember, Ext,
};
use crate::codegen::key_holder::{Named, TypeIndex};
use crate::codegen::keywords::c_ident;
use crate::codegen::least_wire;
use crate::types::{IdlType, PrimitiveType};

/// How a backend holds the values (the parts of the codec that differ).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    /// The C backend: `char*` / `wchar_t*` strings (bounded: `char[N+1]`,
    /// `wchar_t[N+1]`), sequences `{ T* data; uint32_t len; }`, maps,
    /// `wchar_t` characters, `fixed`, bitsets.
    C,
    /// The C-micro backend: strings in `char[]` (unbounded: `max_string`
    /// bytes with the NUL), wide strings in `uint16_t[]`, sequences
    /// `{ T data[cap]; uint32_t count; }` (unbounded: `max_seq`), `wchar` in
    /// `uint16_t`; no map, bitset nor `fixed`.
    Micro {
        /// Bytes of an unbounded string holder, NUL included.
        max_string: u32,
        /// Capacity of an unbounded sequence.
        max_seq: u32,
    },
}

impl Flavor {
    /// Prefix of the runtime functions.
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::C => "hdds_x2c_",
            Self::Micro { .. } => "hdds_x2m_",
        }
    }

    /// Write cursor type.
    #[must_use]
    pub const fn wcur(self) -> &'static str {
        match self {
            Self::C => "hdds_x2c_w",
            Self::Micro { .. } => "hdds_cdr_t",
        }
    }

    /// Read cursor type.
    #[must_use]
    pub const fn rcur(self) -> &'static str {
        match self {
            Self::C => "hdds_x2c_r",
            Self::Micro { .. } => "hdds_cdr_t",
        }
    }

    const fn pos(self) -> &'static str {
        match self {
            Self::C => "size_t",
            Self::Micro { .. } => "uint32_t",
        }
    }

    /// Return type of the codec functions.
    #[must_use]
    pub const fn ret(self) -> &'static str {
        match self {
            Self::C => "int",
            Self::Micro { .. } => "int32_t",
        }
    }

    const fn einv(self) -> &'static str {
        match self {
            Self::C => "(-CDR_INVALID_DATA)",
            Self::Micro { .. } => "HDDS_CDR_ERR_INVALID",
        }
    }

    const fn eeof(self) -> &'static str {
        match self {
            Self::C => "(-CDR_UNEXPECTED_EOF)",
            Self::Micro { .. } => "HDDS_CDR_ERR_UNDERFLOW",
        }
    }

    /// C name of the named type `name` as this backend declares it.
    #[must_use]
    pub fn type_name(self, name: &str) -> String {
        match self {
            Self::C => last_ident(name).to_string(),
            Self::Micro { .. } => micro_type_name(name),
        }
    }

    /// Encoder and decoder names of the structure or union `name`.
    #[must_use]
    pub fn fn_names(self, name: &str) -> (String, String) {
        match self {
            Self::C => {
                let n = c_name(name);
                (format!("{n}__xenc"), format!("{n}__xdec"))
            }
            Self::Micro { .. } => {
                let n = snake(&self.type_name(name));
                (format!("{n}_encode"), format!("{n}_decode"))
            }
        }
    }

    /// Name of the function setting a structure or union to its default
    /// value (DDS-`XTypes` 1.3 Table 9).
    #[must_use]
    pub fn def_name(self, name: &str) -> String {
        match self {
            Self::C => format!("{}__xdef", c_name(name)),
            Self::Micro { .. } => format!("{}_xdefault", snake(&self.type_name(name))),
        }
    }

    /// The holder type of a primitive.
    #[must_use]
    pub const fn prim(self, p: &PrimitiveType) -> &'static str {
        match p {
            PrimitiveType::Boolean => match self {
                Self::C => "bool",
                Self::Micro { .. } => "uint8_t",
            },
            PrimitiveType::Char => "char",
            PrimitiveType::WChar => match self {
                Self::C => "wchar_t",
                Self::Micro { .. } => "uint16_t",
            },
            PrimitiveType::Octet | PrimitiveType::UInt8 => "uint8_t",
            PrimitiveType::Int8 => "int8_t",
            PrimitiveType::Short | PrimitiveType::Int16 => "int16_t",
            PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => "uint16_t",
            PrimitiveType::Long | PrimitiveType::Int32 => "int32_t",
            PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => "uint32_t",
            PrimitiveType::LongLong | PrimitiveType::Int64 => "int64_t",
            PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => "uint64_t",
            PrimitiveType::Float => "float",
            PrimitiveType::Double | PrimitiveType::LongDouble => "double",
            PrimitiveType::Fixed { .. } => "cdr_fixed128_t",
            PrimitiveType::String => "char*",
            PrimitiveType::WString => "wchar_t*",
            PrimitiveType::Void => "void",
        }
    }
}

/// The C name of the named type `name` in the C-micro backend: the one
/// naming function of its declarations, member types, base structures and
/// codec functions. `name` is already the scoped C name
/// (`codegen::c::scoping`: `m::inner::E` is `m_inner_E`); a `::` left in
/// it gets the same `_` separator.
#[must_use]
pub fn micro_type_name(name: &str) -> String {
    name.replace("::", "_")
}

/// `CamelCase` to `snake_case` (the C-micro function prefix).
#[must_use]
pub fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Whether the C declaration of `ty` is an anonymous structure (a sequence
/// or map, alone or as the element of an array), which no cast can name.
fn has_anonymous_struct(ty: &IdlType) -> bool {
    match ty {
        IdlType::Sequence { .. } | IdlType::Map { .. } => true,
        IdlType::Array { inner, .. } => has_anonymous_struct(inner),
        _ => false,
    }
}

/// How a resolved type serializes.
enum Kind {
    Prim(PrimitiveType),
    Str {
        wide: bool,
        bound: Option<u32>,
    },
    Seq {
        inner: IdlType,
        bound: Option<u32>,
    },
    Array {
        elem: IdlType,
        dims: Vec<u32>,
    },
    Map {
        key: IdlType,
        value: IdlType,
        bound: Option<u32>,
    },
    Aggregate(String),
    Enum {
        name: String,
        width: usize,
        values: Vec<i64>,
    },
    Bitmask {
        name: String,
        width: usize,
    },
    Bitset,
    Unknown(String),
}

const fn uint(width: usize) -> &'static str {
    match width {
        1 => "uint8_t",
        2 => "uint16_t",
        4 => "uint32_t",
        _ => "uint64_t",
    }
}

const fn sint(width: usize) -> &'static str {
    match width {
        1 => "int8_t",
        2 => "int16_t",
        4 => "int32_t",
        _ => "int64_t",
    }
}

const fn bits(width: usize) -> usize {
    width * 8
}

/// Code emitter: statements, the loop counters to declare first.
struct Gen<'a> {
    idx: &'a TypeIndex,
    fl: Flavor,
    out: String,
    key_only: bool,
    counters: usize,
    fresh: usize,
    error: Option<String>,
    /// Emitting an XCDR1 decoder (C flavor only, see [`xcdr1`]).
    v1: bool,
    /// The variable holding the XCDR1 alignment origin of the value being
    /// decoded (the start of the buffer, or of the enclosing parameter).
    origin: String,
}

impl<'a> Gen<'a> {
    const fn new(idx: &'a TypeIndex, fl: Flavor) -> Self {
        Self {
            idx,
            fl,
            out: String::new(),
            key_only: false,
            counters: 0,
            fresh: 0,
            error: None,
            v1: false,
            origin: String::new(),
        }
    }

    /// Whether a collection of type `ty` carries a DHEADER in the encoding
    /// being emitted (never under XCDR1).
    fn has_dheader(&self, ty: &IdlType) -> bool {
        !self.v1 && collection_has_dheader(self.idx, ty)
    }

    /// `PREFIX_TRY(rNN(c, &t));` reading an unsigned integer of `width`
    /// bytes into `t`: under XCDR1 an 8-byte one aligns at 8 from the
    /// origin (`hdds_x1c_r64`).
    fn read_uint(&mut self, ind: usize, width: usize, t: &str) {
        if self.v1 && width == 8 {
            let o = self.origin.clone();
            self.line(ind, &format!("HDDS_X2C_TRY(hdds_x1c_r64(c, {o}, &{t}));"));
        } else {
            self.try_(ind, &format!("r{}(c, &{t})", bits(width)));
        }
    }

    fn line(&mut self, ind: usize, text: &str) {
        for _ in 0..ind {
            self.out.push_str("    ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn counter(&mut self) -> String {
        let v = format!("hx_i{}", self.counters);
        self.counters += 1;
        v
    }

    fn fresh(&mut self, prefix: &str) -> String {
        let v = format!("hx_{prefix}{}", self.fresh);
        self.fresh += 1;
        v
    }

    fn fail(&mut self, reason: String) {
        if self.error.is_none() {
            self.error = Some(reason);
        }
    }

    /// `PREFIX_TRY(call);`
    fn try_(&mut self, ind: usize, call: &str) {
        let m = self.fl.prefix().to_ascii_uppercase();
        self.line(ind, &format!("{m}TRY({}{call});", self.fl.prefix()));
    }

    fn counter_decls(&self) -> String {
        if self.counters == 0 {
            return String::new();
        }
        let names: Vec<String> = (0..self.counters).map(|i| format!("hx_i{i}")).collect();
        format!("    uint32_t {};\n", names.join(", "))
    }

    fn kind(&self, ty: &IdlType) -> Kind {
        let resolved = self.idx.resolve(ty);
        match resolved {
            IdlType::BoundedString { wide, bound } => Kind::Str {
                wide,
                bound: Some(bound),
            },
            IdlType::Primitive(PrimitiveType::String) => Kind::Str {
                wide: false,
                bound: None,
            },
            IdlType::Primitive(PrimitiveType::WString) => Kind::Str {
                wide: true,
                bound: None,
            },
            IdlType::Primitive(p) => Kind::Prim(p),
            // `sequence<char>` / `sequence<wchar>` are sequences (an element
            // count, no NUL; 7.4.3.5.3 rule 11), not strings (rules 3-4).
            IdlType::Sequence { inner, bound } => Kind::Seq {
                inner: *inner,
                bound,
            },
            IdlType::Array { .. } => {
                let (elem, dims) = flatten_array(self.idx, &resolved)
                    .unwrap_or_else(|| (resolved.clone(), vec![]));
                Kind::Array { elem, dims }
            }
            IdlType::Map { key, value, bound } => Kind::Map {
                key: *key,
                value: *value,
                bound,
            },
            IdlType::Named(n) => match self.idx.named(&n) {
                Some(Named::Struct(_) | Named::Union(_)) => Kind::Aggregate(n),
                Some(Named::Enum {
                    bit_bound,
                    literals,
                }) => {
                    let mut values: Vec<i64> = literals.iter().map(|(_, v)| *v).collect();
                    values.sort_unstable();
                    values.dedup();
                    Kind::Enum {
                        name: n.clone(),
                        width: enum_width(*bit_bound),
                        values,
                    }
                }
                Some(Named::Bitmask { bit_bound }) => Kind::Bitmask {
                    name: n.clone(),
                    width: bitmask_width(*bit_bound),
                },
                Some(Named::Bitset) => Kind::Bitset,
                Some(Named::Typedef(_)) | None => Kind::Unknown(n),
            },
        }
    }

    /// Capacity a decoder checks a sequence count against, if any.
    fn seq_cap(&self, bound: Option<u32>) -> Option<u32> {
        match self.fl {
            Flavor::C => bound,
            Flavor::Micro { max_seq, .. } => Some(bound.unwrap_or(max_seq)),
        }
    }

    fn count_place(&self, place: &str) -> String {
        match self.fl {
            Flavor::C => format!("{place}.len"),
            Flavor::Micro { .. } => format!("{place}.count"),
        }
    }

    /// Statements setting `place` to the default value of `ty`
    /// (DDS-`XTypes` 1.3 Table 9, spec text lines 2311-2349): zero, false,
    /// an empty string, an empty sequence or map, the first literal of an
    /// enumeration, every element of an array, a structure or union through
    /// its default function. An unbounded C string gets a one-byte "" from
    /// the reader's allocator.
    fn default_value(&mut self, ind: usize, place: &str, ty: &IdlType) {
        match self.kind(ty) {
            Kind::Prim(PrimitiveType::Fixed { .. }) => {
                self.line(ind, &format!("memset(&({place}), 0, sizeof({place}));"));
            }
            Kind::Prim(_) | Kind::Bitmask { .. } => self.line(ind, &format!("{place} = 0;")),
            Kind::Str { wide, bound } => {
                if self.fl == Flavor::C && bound.is_none() {
                    let p = self.fresh("p");
                    let (unit, cty) = if wide {
                        ("sizeof(wchar_t)", "wchar_t")
                    } else {
                        ("1", "char")
                    };
                    self.line(ind, "{");
                    self.line(ind + 1, &format!("void* {p};"));
                    self.try_(ind + 1, &format!("ralloc(c, 1, (size_t){unit}, &{p})"));
                    self.line(ind + 1, &format!("(({cty}*){p})[0] = 0;"));
                    self.line(ind + 1, &format!("{place} = ({cty}*){p};"));
                    self.line(ind, "}");
                } else {
                    self.line(ind, &format!("({place})[0] = 0;"));
                }
            }
            Kind::Seq { .. } => match self.fl {
                Flavor::C => {
                    self.line(ind, &format!("{place}.data = NULL;"));
                    self.line(ind, &format!("{place}.len = 0;"));
                }
                Flavor::Micro { .. } => self.line(ind, &format!("{place}.count = 0;")),
            },
            Kind::Map { .. } => {
                self.line(ind, &format!("{place}.keys = NULL;"));
                self.line(ind, &format!("{place}.values = NULL;"));
                self.line(ind, &format!("{place}.len = 0;"));
            }
            Kind::Array { elem, dims } => {
                let (inner_ind, cur) = self.open_loops(ind, place, &dims);
                self.default_value(inner_ind, &cur, &elem);
                self.close_loops(inner_ind, dims.len());
            }
            Kind::Aggregate(n) => {
                let d = self.fl.def_name(&n);
                self.line(
                    ind,
                    &format!(
                        "{}TRY({d}(&({place}), c));",
                        self.fl.prefix().to_ascii_uppercase()
                    ),
                );
            }
            Kind::Enum { name, .. } => {
                let first = match self.idx.named(&name) {
                    Some(Named::Enum { literals, .. }) => literals.first().map_or(0, |l| l.1),
                    _ => 0,
                };
                let ename = self.fl.type_name(&name);
                self.line(ind, &format!("{place} = ({ename})({first});"));
            }
            Kind::Bitset => self.line(ind, &format!("({place}).bits = 0;")),
            Kind::Unknown(n) => self.fail(format!("type `{n}` is not defined in this IDL file")),
        }
    }

    /// `ptr = storage for n elements` from the reader's allocator (C: the
    /// pointer holders of sequences and maps), charged to the budget of the
    /// decode (`ralloc`: `CDR_MAX_ALLOC_RATIO` times the bytes of the body,
    /// at least `CDR_MAX_ZERO_WIRE_ELEMENTS` bytes, for every request of
    /// the decode together), since the C holder of an element can be far
    /// larger than its least wire form. `elem` is the declared element type
    /// of `ptr`: the pointer is assigned through a cast to it, so the code
    /// is valid C++ as well as C.
    fn alloc(&mut self, ind: usize, ptr: &str, n: &str, elem: &IdlType) {
        let p = self.fresh("p");
        self.line(ind, "{");
        self.line(ind + 1, &format!("void* {p};"));
        self.try_(
            ind + 1,
            &format!("ralloc(c, (size_t){n}, (size_t)sizeof(*({ptr})), &{p})"),
        );
        if has_anonymous_struct(elem) {
            // The element type is an anonymous structure (a nested sequence
            // or map): it cannot be named in a cast, so the pointer is
            // stored byte for byte.
            self.line(ind + 1, &format!("memcpy(&({ptr}), &{p}, sizeof({ptr}));"));
        } else {
            let ty = CGenerator::decl(elem, "(*)");
            self.line(ind + 1, &format!("{ptr} = ({ty}){p};"));
        }
        self.line(ind, "}");
    }

    /// A lower bound of the XCDR2 size of any value of `ty`
    /// ([`least_wire::min_wire`]).
    fn min_wire(&self, ty: &IdlType) -> u64 {
        if self.v1 {
            least_wire::min_wire_xcdr1(self.idx, ty)
        } else {
            least_wire::min_wire(self.idx, ty)
        }
    }

    /// `if (n > bytes left / min_wire) return EOF;`: a count the body
    /// cannot hold is refused before any storage is requested. Elements of
    /// least wire size 0 (an empty FINAL structure) consume no byte, so the
    /// body bounds nothing: in C their storage (`elem`, the C size of one)
    /// is charged to the `CDR_MAX_ZERO_WIRE_ELEMENTS` bytes of the whole
    /// decode and refused (`INVALID_DATA`) beyond them (a C-micro count is
    /// bounded by the holder's capacity already, and its holders are the
    /// caller's).
    fn bound_count(&mut self, ind: usize, n: &str, min: u64, elem: &str) {
        if min == 0 {
            if self.fl == Flavor::C {
                self.try_(ind, &format!("rzcount(c, {n}, (size_t)({elem}))"));
            }
            return;
        }
        let min = min.min(u64::from(u32::MAX));
        self.line(
            ind,
            &format!(
                "if (c->pos > c->size || ({}){n} > (c->size - c->pos) / {min}u) {{ return {}; }}",
                self.fl.pos(),
                self.fl.eeof()
            ),
        );
    }

    // ---------------------------------------------------------------
    // Encoding: statements writing the value at `place` through `c`.
    // ---------------------------------------------------------------

    fn enc(&mut self, ind: usize, place: &str, ty: &IdlType) {
        match self.kind(ty) {
            Kind::Prim(p) => self.enc_prim(ind, place, &p),
            Kind::Str { wide, bound } => self.enc_str(ind, place, wide, bound),
            Kind::Seq { inner, bound } => self.enc_seq(ind, place, ty, &inner, bound),
            Kind::Array { elem, dims } => {
                let dh = collection_has_dheader(self.idx, ty);
                let d = self.fresh("d");
                if dh {
                    self.line(ind, "{");
                    self.line(ind + 1, &format!("{} {d};", self.fl.pos()));
                    self.try_(ind + 1, &format!("wbegin(c, &{d})"));
                }
                let i0 = if dh { ind + 1 } else { ind };
                let (inner_ind, cur) = self.open_loops(i0, place, &dims);
                self.enc(inner_ind, &cur, &elem);
                self.close_loops(inner_ind, dims.len());
                if dh {
                    self.try_(ind + 1, &format!("wend(c, {d})"));
                    self.line(ind, "}");
                }
            }
            Kind::Map { key, value, bound } => {
                if self.fl != Flavor::C {
                    self.fail("a map is not supported by this backend".into());
                    return;
                }
                let dh = collection_has_dheader(self.idx, ty);
                let d = self.fresh("d");
                self.line(ind, "{");
                if dh {
                    self.line(ind + 1, &format!("{} {d};", self.fl.pos()));
                }
                if let Some(b) = bound {
                    self.line(
                        ind + 1,
                        &format!("if ({place}.len > {b}u) {{ return {}; }}", self.fl.einv()),
                    );
                }
                if dh {
                    self.try_(ind + 1, &format!("wbegin(c, &{d})"));
                }
                self.try_(ind + 1, &format!("w32(c, (uint32_t)({place}.len))"));
                let i = self.counter();
                self.line(
                    ind + 1,
                    &format!("for ({i} = 0; {i} < (uint32_t)({place}.len); ++{i}) {{"),
                );
                self.enc(ind + 2, &format!("{place}.keys[{i}]"), &key);
                self.enc(ind + 2, &format!("{place}.values[{i}]"), &value);
                self.line(ind + 1, "}");
                if dh {
                    self.try_(ind + 1, &format!("wend(c, {d})"));
                }
                self.line(ind, "}");
            }
            Kind::Aggregate(n) => {
                let (e, _) = self.fl.fn_names(&n);
                let e = encoder_name(&e, self.key_only);
                self.line(
                    ind,
                    &format!(
                        "{}TRY({e}(&({place}), c));",
                        self.fl.prefix().to_ascii_uppercase()
                    ),
                );
            }
            Kind::Enum { width, .. } => {
                self.try_(
                    ind,
                    &format!(
                        "w{}(c, ({})({})({place}))",
                        bits(width),
                        uint(width),
                        sint(width)
                    ),
                );
            }
            Kind::Bitmask { width, .. } => {
                self.try_(
                    ind,
                    &format!("w{}(c, ({})({place}))", bits(width), uint(width)),
                );
            }
            Kind::Bitset => {
                if self.fl == Flavor::C {
                    self.try_(ind, &format!("w64(c, (uint64_t)(({place}).bits))"));
                } else {
                    self.fail("a bitset is not supported by this backend".into());
                }
            }
            Kind::Unknown(n) => self.fail(format!("type `{n}` is not defined in this IDL file")),
        }
    }

    fn open_loops(&mut self, ind: usize, place: &str, dims: &[u32]) -> (usize, String) {
        let mut cur = place.to_string();
        let mut depth = ind;
        for d in dims {
            let i = self.counter();
            self.line(depth, &format!("for ({i} = 0; {i} < {d}u; ++{i}) {{"));
            cur = format!("{cur}[{i}]");
            depth += 1;
        }
        (depth, cur)
    }

    fn close_loops(&mut self, inner_ind: usize, n: usize) {
        let mut depth = inner_ind;
        for _ in 0..n {
            depth -= 1;
            self.line(depth, "}");
        }
    }

    fn enc_seq(
        &mut self,
        ind: usize,
        place: &str,
        ty: &IdlType,
        inner: &IdlType,
        bound: Option<u32>,
    ) {
        let dh = collection_has_dheader(self.idx, ty);
        let count = self.count_place(place);
        let d = self.fresh("d");
        self.line(ind, "{");
        if dh {
            self.line(ind + 1, &format!("{} {d};", self.fl.pos()));
        }
        if let Some(cap) = self.seq_cap(bound) {
            self.line(
                ind + 1,
                &format!("if ({count} > {cap}u) {{ return {}; }}", self.fl.einv()),
            );
        }
        if dh {
            self.try_(ind + 1, &format!("wbegin(c, &{d})"));
        }
        self.try_(ind + 1, &format!("w32(c, (uint32_t)({count}))"));
        let i = self.counter();
        self.line(
            ind + 1,
            &format!("for ({i} = 0; {i} < (uint32_t)({count}); ++{i}) {{"),
        );
        self.enc(ind + 2, &format!("{place}.data[{i}]"), inner);
        self.line(ind + 1, "}");
        if dh {
            self.try_(ind + 1, &format!("wend(c, {d})"));
        }
        self.line(ind, "}");
    }

    fn enc_str(&mut self, ind: usize, place: &str, wide: bool, bound: Option<u32>) {
        let call = match (self.fl, wide) {
            (Flavor::C, false) => format!("wstr(c, {place}, {}u)", bound.unwrap_or(0)),
            (Flavor::C, true) => format!("wwstr(c, {place}, {}u)", bound.unwrap_or(0)),
            (Flavor::Micro { max_string, .. }, w) => {
                let b = bound.unwrap_or_else(|| max_string.saturating_sub(1));
                if b == 0 {
                    self.fail("an unbounded string needs a holder of at least 2 bytes".into());
                    return;
                }
                if w {
                    format!("wstr16(c, {place}, {b}u)")
                } else {
                    format!("wstr(c, {place}, {b}u)")
                }
            }
        };
        self.try_(ind, &call);
    }

    fn enc_prim(&mut self, ind: usize, place: &str, p: &PrimitiveType) {
        let call = match p {
            PrimitiveType::Boolean => format!("w8(c, (uint8_t)(({place}) ? 1 : 0))"),
            PrimitiveType::Octet
            | PrimitiveType::UInt8
            | PrimitiveType::Int8
            | PrimitiveType::Char => format!("w8(c, (uint8_t)({place}))"),
            PrimitiveType::Short
            | PrimitiveType::Int16
            | PrimitiveType::UnsignedShort
            | PrimitiveType::UInt16 => format!("w16(c, (uint16_t)({place}))"),
            PrimitiveType::WChar => match self.fl {
                Flavor::C => format!("wwchar(c, {place})"),
                Flavor::Micro { .. } => format!("w16(c, (uint16_t)({place}))"),
            },
            PrimitiveType::Long
            | PrimitiveType::Int32
            | PrimitiveType::UnsignedLong
            | PrimitiveType::UInt32 => format!("w32(c, (uint32_t)({place}))"),
            PrimitiveType::Float => format!("wf32(c, {place})"),
            PrimitiveType::LongLong
            | PrimitiveType::Int64
            | PrimitiveType::UnsignedLongLong
            | PrimitiveType::UInt64 => format!("w64(c, (uint64_t)({place}))"),
            PrimitiveType::Double | PrimitiveType::LongDouble => {
                format!("wf64(c, (double)({place}))")
            }
            PrimitiveType::Fixed { .. } => {
                if self.fl != Flavor::C {
                    self.fail("`fixed` is not supported by this backend".into());
                    return;
                }
                let f = self.fresh("fx");
                self.line(ind, "{");
                self.line(ind + 1, &format!("uint8_t {f}[16];"));
                self.try_(ind + 1, "pad(c, 4)");
                self.line(ind + 1, &format!("cdr_fixed128_to_le(&({place}), {f});"));
                self.try_(ind + 1, &format!("raw(c, {f}, 16)"));
                self.line(ind, "}");
                return;
            }
            PrimitiveType::String | PrimitiveType::WString | PrimitiveType::Void => return,
        };
        self.try_(ind, &call);
    }

    // ---------------------------------------------------------------
    // Decoding: statements reading the value into `place` through `c`.
    // ---------------------------------------------------------------

    fn dec(&mut self, ind: usize, place: &str, ty: &IdlType) {
        match self.kind(ty) {
            Kind::Prim(p) => self.dec_prim(ind, place, &p),
            Kind::Str { wide, bound } => self.dec_str(ind, place, wide, bound),
            Kind::Seq { inner, bound } => self.dec_seq(ind, place, ty, &inner, bound),
            Kind::Array { elem, dims } => {
                let dh = self.has_dheader(ty);
                let e = self.fresh("e");
                if dh {
                    self.line(ind, "{");
                    self.line(ind + 1, &format!("{} {e};", self.fl.pos()));
                    self.try_(ind + 1, &format!("rbegin(c, &{e})"));
                }
                let i0 = if dh { ind + 1 } else { ind };
                let (inner_ind, cur) = self.open_loops(i0, place, &dims);
                self.dec(inner_ind, &cur, &elem);
                self.close_loops(inner_ind, dims.len());
                if dh {
                    self.try_(ind + 1, &format!("rskip(c, {e})"));
                    self.line(ind, "}");
                }
            }
            Kind::Map { key, value, bound } => {
                if self.fl != Flavor::C {
                    self.fail("a map is not supported by this backend".into());
                    return;
                }
                let dh = self.has_dheader(ty);
                let e = self.fresh("e");
                let n = self.fresh("n");
                self.line(ind, "{");
                self.line(ind + 1, &format!("{} {e};", self.fl.pos()));
                self.line(ind + 1, &format!("uint32_t {n};"));
                self.line(ind + 1, &format!("(void){e};"));
                if dh {
                    self.try_(ind + 1, &format!("rbegin(c, &{e})"));
                }
                self.try_(ind + 1, &format!("r32(c, &{n})"));
                if let Some(b) = bound {
                    self.line(
                        ind + 1,
                        &format!("if ({n} > {b}u) {{ return {}; }}", self.fl.einv()),
                    );
                }
                // Every entry takes at least the key and value minima.
                let min = self.min_wire(&key).saturating_add(self.min_wire(&value));
                self.bound_count(
                    ind + 1,
                    &n,
                    min,
                    &format!("sizeof(*({place}.keys)) + sizeof(*({place}.values))"),
                );
                self.alloc(ind + 1, &format!("{place}.keys"), &n, &key);
                self.alloc(ind + 1, &format!("{place}.values"), &n, &value);
                self.line(ind + 1, &format!("{place}.len = {n};"));
                let i = self.counter();
                self.line(ind + 1, &format!("for ({i} = 0; {i} < {n}; ++{i}) {{"));
                self.dec(ind + 2, &format!("{place}.keys[{i}]"), &key);
                self.dec(ind + 2, &format!("{place}.values[{i}]"), &value);
                self.line(ind + 1, "}");
                if dh {
                    self.try_(ind + 1, &format!("rskip(c, {e})"));
                }
                self.line(ind, "}");
            }
            Kind::Aggregate(n) if self.v1 => {
                let d = xcdr1::dec1_name(&n);
                let o = self.origin.clone();
                self.line(ind, &format!("HDDS_X2C_TRY({d}(&({place}), c, {o}));"));
            }
            Kind::Aggregate(n) => {
                let (_, d) = self.fl.fn_names(&n);
                self.line(
                    ind,
                    &format!(
                        "{}TRY({d}(&({place}), c));",
                        self.fl.prefix().to_ascii_uppercase()
                    ),
                );
            }
            Kind::Enum {
                name,
                width,
                values,
            } => {
                let t = self.fresh("t");
                let v = self.fresh("v");
                self.line(ind, "{");
                self.line(ind + 1, &format!("{} {t};", uint(width)));
                self.line(ind + 1, &format!("int32_t {v};"));
                self.read_uint(ind + 1, width, &t);
                self.line(ind + 1, &format!("{v} = (int32_t)({}){t};", sint(width)));
                let cases: Vec<String> = values.iter().map(|x| format!("case {x}:")).collect();
                self.line(
                    ind + 1,
                    &format!(
                        "switch ({v}) {{ {} break; default: return {}; }}",
                        cases.join(" "),
                        self.fl.einv()
                    ),
                );
                let ename = self.fl.type_name(&name);
                self.line(ind + 1, &format!("{place} = ({ename}){v};"));
                self.line(ind, "}");
            }
            Kind::Bitmask { name, width } => {
                let t = self.fresh("t");
                self.line(ind, "{");
                self.line(ind + 1, &format!("{} {t};", uint(width)));
                self.read_uint(ind + 1, width, &t);
                let bname = self.fl.type_name(&name);
                self.line(ind + 1, &format!("{place} = ({bname}){t};"));
                self.line(ind, "}");
            }
            Kind::Bitset => {
                if self.fl == Flavor::C {
                    self.read_uint(ind, 8, &format!("(({place}).bits)"));
                } else {
                    self.fail("a bitset is not supported by this backend".into());
                }
            }
            Kind::Unknown(n) => self.fail(format!("type `{n}` is not defined in this IDL file")),
        }
    }

    fn dec_seq(
        &mut self,
        ind: usize,
        place: &str,
        ty: &IdlType,
        inner: &IdlType,
        bound: Option<u32>,
    ) {
        let dh = self.has_dheader(ty);
        let count = self.count_place(place);
        let e = self.fresh("e");
        let n = self.fresh("n");
        self.line(ind, "{");
        if dh {
            self.line(ind + 1, &format!("{} {e};", self.fl.pos()));
        }
        self.line(ind + 1, &format!("uint32_t {n};"));
        if dh {
            self.try_(ind + 1, &format!("rbegin(c, &{e})"));
        }
        self.try_(ind + 1, &format!("r32(c, &{n})"));
        if let Some(cap) = self.seq_cap(bound) {
            self.line(
                ind + 1,
                &format!("if ({n} > {cap}u) {{ return {}; }}", self.fl.einv()),
            );
        }
        let min = self.min_wire(inner);
        self.bound_count(ind + 1, &n, min, &format!("sizeof(*({place}.data))"));
        if self.fl == Flavor::C {
            self.alloc(ind + 1, &format!("{place}.data"), &n, inner);
        }
        self.line(ind + 1, &format!("{count} = {n};"));
        let i = self.counter();
        self.line(ind + 1, &format!("for ({i} = 0; {i} < {n}; ++{i}) {{"));
        self.dec(ind + 2, &format!("{place}.data[{i}]"), inner);
        self.line(ind + 1, "}");
        if dh {
            self.try_(ind + 1, &format!("rskip(c, {e})"));
        }
        self.line(ind, "}");
    }

    fn dec_str(&mut self, ind: usize, place: &str, wide: bool, bound: Option<u32>) {
        let call = match (self.fl, wide) {
            // Unbounded: a pointer, the storage from the reader's allocator.
            (Flavor::C, false) => bound.map_or_else(
                || format!("rstra(c, &({place}))"),
                |b| format!("rstr(c, {place}, {}u)", u64::from(b) + 1),
            ),
            (Flavor::C, true) => bound.map_or_else(
                || format!("rwstra(c, &({place}))"),
                |b| format!("rwstr(c, {place}, {}u, {b}u)", u64::from(b) + 1),
            ),
            (Flavor::Micro { max_string, .. }, w) => {
                let cap = bound.map_or_else(|| u64::from(max_string), |b| u64::from(b) + 1);
                if w {
                    format!("rstr16(c, {place}, {cap}u)")
                } else {
                    format!("rstr(c, {place}, {cap}u)")
                }
            }
        };
        self.try_(ind, &call);
    }

    fn dec_prim(&mut self, ind: usize, place: &str, p: &PrimitiveType) {
        let holder = self.fl.prim(p);
        let width = match p {
            PrimitiveType::Boolean => {
                let t = self.fresh("t");
                self.line(ind, "{");
                self.line(ind + 1, &format!("uint8_t {t};"));
                self.try_(ind + 1, &format!("r8(c, &{t})"));
                self.line(
                    ind + 1,
                    &format!("if ({t} > 1u) {{ return {}; }}", self.fl.einv()),
                );
                self.line(ind + 1, &format!("{place} = ({holder})({t} != 0);"));
                self.line(ind, "}");
                return;
            }
            PrimitiveType::Octet
            | PrimitiveType::UInt8
            | PrimitiveType::Int8
            | PrimitiveType::Char => 1,
            PrimitiveType::Short
            | PrimitiveType::Int16
            | PrimitiveType::UnsignedShort
            | PrimitiveType::UInt16 => 2,
            PrimitiveType::WChar => {
                match self.fl {
                    Flavor::C => self.try_(ind, &format!("rwchar(c, &({place}))")),
                    Flavor::Micro { .. } => self.try_(ind, &format!("r16(c, &({place}))")),
                }
                return;
            }
            PrimitiveType::Long
            | PrimitiveType::Int32
            | PrimitiveType::UnsignedLong
            | PrimitiveType::UInt32 => 4,
            PrimitiveType::LongLong
            | PrimitiveType::Int64
            | PrimitiveType::UnsignedLongLong
            | PrimitiveType::UInt64 => 8,
            PrimitiveType::Float => {
                self.try_(ind, &format!("rf32(c, &({place}))"));
                return;
            }
            PrimitiveType::Double | PrimitiveType::LongDouble => {
                if self.v1 {
                    let o = self.origin.clone();
                    self.line(
                        ind,
                        &format!("HDDS_X2C_TRY(hdds_x1c_rf64(c, {o}, &({place})));"),
                    );
                } else {
                    self.try_(ind, &format!("rf64(c, &({place}))"));
                }
                return;
            }
            PrimitiveType::Fixed { .. } => {
                if self.fl != Flavor::C {
                    self.fail("`fixed` is not supported by this backend".into());
                    return;
                }
                let f = self.fresh("fx");
                self.line(ind, "{");
                self.line(ind + 1, &format!("uint8_t {f}[16];"));
                self.try_(ind + 1, "rpad(c, 4)");
                self.try_(ind + 1, &format!("rraw(c, {f}, 16)"));
                self.line(ind + 1, &format!("cdr_fixed128_from_le(&({place}), {f});"));
                self.line(ind, "}");
                return;
            }
            PrimitiveType::String | PrimitiveType::WString | PrimitiveType::Void => return,
        };
        let t = self.fresh("t");
        self.line(ind, "{");
        self.line(ind + 1, &format!("{} {t};", uint(width)));
        self.read_uint(ind + 1, width, &t);
        self.line(ind + 1, &format!("{place} = ({holder}){t};"));
        self.line(ind, "}");
    }
}

/// Names a structure or union's functions are emitted under.
pub struct Names<'n> {
    /// C type name of the value.
    pub ctype: &'n str,
    /// Encoder name.
    pub enc: &'n str,
    /// Decoder name.
    pub dec: &'n str,
}

fn encoder_name(encoder: &str, key_only: bool) -> String {
    if key_only {
        format!(
            "{}__xkey",
            encoder.strip_suffix("__xenc").unwrap_or(encoder)
        )
    } else {
        encoder.to_string()
    }
}

/// Prototypes of the encoder and decoder, `static <inline>`.
#[must_use]
pub fn prototypes(fl: Flavor, inline: &str, n: &Names<'_>) -> String {
    let key = if fl == Flavor::C {
        format!(
            "static {inline} int {}(const {}* v, {}* c);\n",
            encoder_name(n.enc, true),
            n.ctype,
            fl.wcur()
        )
    } else {
        String::new()
    };
    key + &format!(
        "static {inline} {ret} {e}(const {t}* v, {w}* c);\nstatic {inline} {ret} {d}({t}* v, {r}* c);\nstatic {inline} {ret} {f}({t}* v, {r}* c);\n",
        ret = fl.ret(),
        e = n.enc,
        d = n.dec,
        f = fl.def_name(n.ctype),
        t = n.ctype,
        w = fl.wcur(),
        r = fl.rcur()
    )
}

fn function(fl: Flavor, inline: &str, head: &str, g: &Gen<'_>, locals: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "static {inline} {} {head} {{", fl.ret());
    out.push_str(locals);
    out.push_str(&g.counter_decls());
    if matches!(fl, Flavor::Micro { .. }) {
        out.push_str("    if (v == NULL || c == NULL) { return HDDS_CDR_ERR_NULL; }\n");
    }
    out.push_str("    (void)v;\n    (void)c;\n");
    out.push_str(&g.out);
    out.push_str("    return 0;\n}\n\n");
    out
}

/// Place of a chain member read from `v`.
fn member_place(m: &ChainMember, prefix: &str) -> String {
    let mut place = String::from("v->");
    for _ in 0..m.base_depth {
        place.push_str("base.");
    }
    let _ = write!(place, "{prefix}{}", c_ident(&m.field.name));
    place
}

/// The encoder and decoder of a structure.
///
/// # Errors
///
/// Returns why the structure cannot be serialized by this backend (a member
/// type it cannot hold, an unknown base).
pub fn struct_codec(
    idx: &TypeIndex,
    fl: Flavor,
    inline: &str,
    s: &Struct,
    n: &Names<'_>,
) -> Result<String, String> {
    let mut out = struct_codec_impl(idx, fl, inline, s, n, false)?;
    if fl == Flavor::C {
        out.push_str(&struct_codec_impl(idx, fl, inline, s, n, true)?);
    }
    Ok(out)
}

fn struct_codec_impl(
    idx: &TypeIndex,
    fl: Flavor,
    inline: &str,
    s: &Struct,
    n: &Names<'_>,
    key_only: bool,
) -> Result<String, String> {
    let chain = member_chain(idx, s)?;
    let chain = if key_only {
        data_model::key_members(chain)
    } else {
        chain
    };
    let ext = data_model::struct_ext(s);
    let pos = fl.pos();

    // ---- encode ----
    let mut g = Gen::new(idx, fl);
    g.key_only = key_only;
    match ext {
        Ext::Final | Ext::Appendable => {
            if ext == Ext::Appendable {
                g.try_(1, "wbegin(c, &dh)");
            }
            for m in &chain {
                let place = member_place(m, "");
                if m.field.is_optional() {
                    let has = member_place(m, "has_");
                    g.try_(1, &format!("w8(c, (uint8_t)({has} ? 1 : 0))"));
                    g.line(1, &format!("if ({has}) {{"));
                    g.enc(2, &place, &m.field.field_type);
                    g.line(1, "}");
                } else {
                    g.enc(1, &place, &m.field.field_type);
                }
            }
            if ext == Ext::Appendable {
                g.try_(1, "wend(c, dh)");
            }
        }
        Ext::Mutable => {
            g.try_(1, "wbegin(c, &dh)");
            for m in &chain {
                enc_mutable_member(&mut g, m);
            }
            g.try_(1, "wend(c, dh)");
        }
    }
    let locals = if ext == Ext::Final {
        String::new()
    } else {
        format!("    {pos} dh;\n")
    };
    let encoder = encoder_name(n.enc, key_only);
    let enc_head = format!("{encoder}(const {}* v, {}* c)", n.ctype, fl.wcur());
    if let Some(e) = g.error.take() {
        return Err(e);
    }
    let mut out = function(fl, inline, &enc_head, &g, &locals);

    if key_only {
        return Ok(out);
    }

    // ---- decode ----
    let mut g = Gen::new(idx, fl);
    let mut locals = locals;
    match ext {
        Ext::Final | Ext::Appendable => {
            let app = ext == Ext::Appendable;
            if app {
                g.try_(1, "rbegin(c, &dh)");
            }
            // APPENDABLE: a member past the DHEADER's end was not written
            // by the (older) writer's type: it takes its default
            // (7.4.1.2.3; Table 9).
            let ind = if app { 2 } else { 1 };
            for m in &chain {
                let place = member_place(m, "");
                if app {
                    g.line(1, "if (c->pos < dh) {");
                }
                if m.field.is_optional() {
                    let has = member_place(m, "has_");
                    let t = g.fresh("t");
                    g.line(ind, "{");
                    g.line(ind + 1, &format!("uint8_t {t};"));
                    g.try_(ind + 1, &format!("r8(c, &{t})"));
                    g.line(
                        ind + 1,
                        &format!("if ({t} > 1u) {{ return {}; }}", fl.einv()),
                    );
                    g.line(ind + 1, &format!("{has} = {t};"));
                    g.line(ind, "}");
                    g.line(ind, &format!("if ({has}) {{"));
                    g.dec(ind + 1, &place, &m.field.field_type);
                    g.line(ind, "}");
                } else {
                    g.dec(ind, &place, &m.field.field_type);
                }
                if app {
                    g.line(1, "} else {");
                    if m.field.is_optional() {
                        g.line(2, &format!("{} = 0;", member_place(m, "has_")));
                    } else {
                        g.default_value(2, &place, &m.field.field_type);
                    }
                    g.line(1, "}");
                }
            }
            if ext == Ext::Appendable {
                g.try_(1, "rskip(c, dh)");
            }
        }
        Ext::Mutable => {
            let _ = write!(
                locals,
                "    {pos} mend;\n    uint32_t id;\n    int mu;\n    unsigned char seen[{}];\n",
                chain.len().max(1)
            );
            dec_mutable(&mut g, &chain);
        }
    }
    let dec_head = format!("{}({}* v, {}* c)", n.dec, n.ctype, fl.rcur());
    if let Some(e) = g.error.take() {
        return Err(e);
    }
    out.push_str(&function(fl, inline, &dec_head, &g, &locals));

    // ---- default (Table 9: no optional member, the others at their
    // default) ----
    let mut g = Gen::new(idx, fl);
    for m in &chain {
        if m.field.is_optional() {
            g.line(1, &format!("{} = 0;", member_place(m, "has_")));
        } else {
            g.default_value(1, &member_place(m, ""), &m.field.field_type);
        }
    }
    if let Some(e) = g.error.take() {
        return Err(e);
    }
    let default_head = format!("{}({}* v, {}* c)", fl.def_name(n.ctype), n.ctype, fl.rcur());
    out.push_str(&function(fl, inline, &default_head, &g, ""));
    Ok(out)
}

/// One member of a MUTABLE structure: `EMHEADER1`, a NEXTINT when the
/// length code is 4, the value (rule 22); an absent optional member is left
/// out.
fn enc_mutable_member(g: &mut Gen<'_>, m: &ChainMember) {
    let place = member_place(m, "");
    let lc = length_code(g.idx, &m.field.field_type);
    let mu = u32::from(must_understand(&m.field));
    let word = (mu << 31) | (lc << 28) | (m.id & 0x0fff_ffff);
    let ind = if m.field.is_optional() {
        g.line(1, &format!("if ({}) {{", member_place(m, "has_")));
        2
    } else {
        1
    };
    g.try_(ind, &format!("w32(c, 0x{word:08x}u)"));
    if lc == 4 {
        let nx = g.fresh("nx");
        g.line(ind, "{");
        g.line(ind + 1, &format!("{} {nx};", g.fl.pos()));
        g.try_(ind + 1, &format!("wbegin(c, &{nx})"));
        g.enc(ind + 1, &place, &m.field.field_type);
        g.try_(ind + 1, &format!("wend(c, {nx})"));
        g.line(ind, "}");
    } else {
        g.enc(ind, &place, &m.field.field_type);
    }
    if m.field.is_optional() {
        g.line(1, "}");
    }
}

/// The member loop of a MUTABLE structure's decoder: a non-optional member
/// absent from the data takes its default (7.4.1.2.3, spec text lines
/// 6195-6203; Table 9), an absent optional member has no value; an unknown
/// member is skipped unless it must be understood.
fn dec_mutable(g: &mut Gen<'_>, chain: &[ChainMember]) {
    g.line(1, "memset(seen, 0, sizeof(seen));");
    for m in chain.iter().filter(|m| m.field.is_optional()) {
        g.line(1, &format!("{} = 0;", member_place(m, "has_")));
    }
    g.try_(1, "rbegin(c, &dh)");
    g.line(1, "while (c->pos < dh) {");
    g.try_(2, "remh(c, dh, &mu, &id, &mend)");
    g.line(2, "switch (id) {");
    for (k, m) in chain.iter().enumerate() {
        g.line(2, &format!("case 0x{:08x}u:", m.id));
        g.dec(3, &member_place(m, ""), &m.field.field_type);
        g.line(3, &format!("seen[{k}] = 1;"));
        if m.field.is_optional() {
            g.line(3, &format!("{} = 1;", member_place(m, "has_")));
        }
        g.line(3, "break;");
    }
    g.line(2, "default:");
    g.line(3, &format!("if (mu) {{ return {}; }}", g.fl.einv()));
    g.line(3, "break;");
    g.line(2, "}");
    g.try_(2, "rskip(c, mend)");
    g.line(1, "}");
    g.try_(1, "rskip(c, dh)");
    for (k, m) in chain.iter().enumerate() {
        if !m.field.is_optional() {
            g.line(1, &format!("if (!seen[{k}]) {{"));
            g.default_value(2, &member_place(m, ""), &m.field.field_type);
            g.line(1, "}");
        }
    }
}

// -------------------------------------------------------------------
// Unions
// -------------------------------------------------------------------

/// Width and signedness of the discriminator of type `ty`, `None` for a
/// type that cannot discriminate.
fn disc_holder(idx: &TypeIndex, ty: &IdlType) -> Option<(usize, bool)> {
    match idx.resolve(ty) {
        IdlType::Primitive(p) => match p {
            PrimitiveType::Boolean
            | PrimitiveType::Octet
            | PrimitiveType::UInt8
            | PrimitiveType::Char => Some((1, false)),
            PrimitiveType::Int8 => Some((1, true)),
            PrimitiveType::WChar | PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => {
                Some((2, false))
            }
            PrimitiveType::Short | PrimitiveType::Int16 => Some((2, true)),
            PrimitiveType::Long | PrimitiveType::Int32 => Some((4, true)),
            PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => Some((4, false)),
            PrimitiveType::LongLong | PrimitiveType::Int64 => Some((8, true)),
            PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => Some((8, false)),
            _ => None,
        },
        IdlType::Named(n) => match idx.named(&n) {
            Some(Named::Enum { bit_bound, .. }) => Some((enum_width(*bit_bound), true)),
            _ => None,
        },
        _ => None,
    }
}

/// Integer value of a case label, when it can be computed here
/// ([`data_model::union_label_value`]).
#[must_use]
pub fn label_value(idx: &TypeIndex, disc: &IdlType, label: &str) -> Option<i64> {
    data_model::union_label_value(idx, disc, label)
}

/// The discriminator's C type, for the casts.
fn disc_ctype(idx: &TypeIndex, fl: Flavor, ty: &IdlType) -> String {
    match idx.resolve(ty) {
        IdlType::Primitive(p) => fl.prim(&p).to_string(),
        IdlType::Named(n) => fl.type_name(&n),
        other => format!("/* {} */ int32_t", other.to_idl_string()),
    }
}

/// `if (...) { ... } else if ... else { default }` over the cases of `u`;
/// the body gets the index of the selected case. With `none`, a union
/// without `default` gets an `else` branch for a discriminator no label
/// selects (the body gets `None`).
fn case_chain(
    g: &mut Gen<'_>,
    u: &Union,
    dctype: &str,
    none: bool,
    body: &mut dyn FnMut(&mut Gen<'_>, Option<usize>),
) {
    let mut first = true;
    let mut default = None;
    for (i, case) in u.cases.iter().enumerate() {
        let mut conds = Vec::new();
        for label in &case.labels {
            match label {
                UnionLabel::Default => default = Some(i),
                UnionLabel::Value(v) => match label_value(g.idx, &u.discriminator, v) {
                    Some(x) => conds.push(format!("v->_d == ({dctype})({x})")),
                    None => g.fail(format!(
                        "the label `{v}` of case `{}` is not a value hddsgen can compute",
                        case.field.name
                    )),
                },
            }
        }
        if conds.is_empty() {
            continue;
        }
        let kw = if first { "if" } else { "} else if" };
        first = false;
        g.line(1, &format!("{kw} ({}) {{", conds.join(" || ")));
        body(g, Some(i));
    }
    if default.is_some() || none {
        g.line(1, if first { "{" } else { "} else {" });
        body(g, default);
        g.line(1, "}");
    } else if !first {
        g.line(1, "}");
    }
}

/// The encoder and decoder of a union.
///
/// # Errors
///
/// Returns why the union cannot be serialized by this backend.
pub fn union_codec(
    idx: &TypeIndex,
    fl: Flavor,
    inline: &str,
    u: &Union,
    n: &Names<'_>,
) -> Result<String, String> {
    let mut out = union_codec_impl(idx, fl, inline, u, n, false)?;
    if fl == Flavor::C {
        let mut key_union = u.clone();
        if u.discriminator_is_key() {
            key_union.cases.clear();
        }
        let key = union_codec_impl(idx, fl, inline, &key_union, n, true)?;
        let end = key.find("\n}\n").map_or(key.len(), |n| n + 3);
        out.push_str(&key[..end]);
    }
    Ok(out)
}

fn union_codec_impl(
    idx: &TypeIndex,
    fl: Flavor,
    inline: &str,
    u: &Union,
    n: &Names<'_>,
    key_only: bool,
) -> Result<String, String> {
    let ext = union_ext(u);
    if ext == Ext::Mutable {
        return mutable_union_codec(idx, fl, inline, u, n, key_only);
    }
    let (width, signed) = disc_holder(idx, &u.discriminator).ok_or_else(|| {
        format!(
            "discriminator type `{}` is not an integer, char, boolean or enumeration",
            u.discriminator.to_idl_string()
        )
    })?;
    let dctype = disc_ctype(idx, fl, &u.discriminator);
    let locals = if ext == Ext::Appendable {
        format!("    {} dh;\n", fl.pos())
    } else {
        String::new()
    };
    let member = |m: &Field| format!("v->_u.{}", c_ident(&m.name));
    let is_void = |m: &Field| matches!(m.field_type, IdlType::Primitive(PrimitiveType::Void));

    // ---- encode ----
    let mut g = Gen::new(idx, fl);
    g.key_only = key_only;
    if ext == Ext::Appendable {
        g.try_(1, "wbegin(c, &dh)");
    }
    let disc = if signed {
        format!("({})({})(v->_d)", uint(width), sint(width))
    } else {
        format!("({})(v->_d)", uint(width))
    };
    g.try_(1, &format!("w{}(c, {disc})", bits(width)));
    case_chain(&mut g, u, &dctype, false, &mut |g, i| {
        if let Some(m) = i.map(|i| &u.cases[i].field) {
            if !is_void(m) {
                g.enc(2, &member(m), &m.field_type);
            }
        }
    });
    if ext == Ext::Appendable {
        g.try_(1, "wend(c, dh)");
    }
    if let Some(e) = g.error.take() {
        return Err(e);
    }
    let encoder = encoder_name(n.enc, key_only);
    let enc_head = format!("{encoder}(const {}* v, {}* c)", n.ctype, fl.wcur());
    let mut out = function(fl, inline, &enc_head, &g, &locals);

    // ---- decode ----
    let mut g = Gen::new(idx, fl);
    if ext == Ext::Appendable {
        g.try_(1, "rbegin(c, &dh)");
    }
    g.line(1, "{");
    g.line(2, &format!("{} t;", uint(width)));
    g.try_(2, &format!("r{}(c, &t)", bits(width)));
    if signed {
        g.line(2, &format!("v->_d = ({dctype})({})t;", sint(width)));
    } else {
        g.line(2, &format!("v->_d = ({dctype})t;"));
    }
    g.line(1, "}");
    case_chain(&mut g, u, &dctype, false, &mut |g, i| {
        if let Some(m) = i.map(|i| &u.cases[i].field) {
            if !is_void(m) {
                g.dec(2, &member(m), &m.field_type);
            }
        }
    });
    if ext == Ext::Appendable {
        g.try_(1, "rskip(c, dh)");
    }
    if let Some(e) = g.error.take() {
        return Err(e);
    }
    let dec_head = format!("{}({}* v, {}* c)", n.dec, n.ctype, fl.rcur());
    out.push_str(&function(fl, inline, &dec_head, &g, &locals));
    out.push_str(&union_default_fn(idx, fl, inline, u, n, &dctype)?);
    Ok(out)
}

/// The default function of a union (Table 9): the discriminator at the
/// default of its type (0, false, '\0', the first literal of an
/// enumeration), and the member it selects, if any, at its default.
fn union_default_fn(
    idx: &TypeIndex,
    fl: Flavor,
    inline: &str,
    u: &Union,
    n: &Names<'_>,
    dctype: &str,
) -> Result<String, String> {
    let dval = match idx.resolve(&u.discriminator) {
        IdlType::Named(name) => match idx.named(&name) {
            Some(Named::Enum { literals, .. }) => literals.first().map_or(0, |l| l.1),
            _ => 0,
        },
        _ => 0,
    };
    let selects = |l: &UnionLabel| match l {
        UnionLabel::Value(v) => label_value(idx, &u.discriminator, v) == Some(dval),
        UnionLabel::Default => false,
    };
    let sel = u
        .cases
        .iter()
        .position(|c| c.labels.iter().any(selects))
        .or_else(|| {
            u.cases
                .iter()
                .position(|c| c.labels.iter().any(|l| matches!(l, UnionLabel::Default)))
        });
    let mut g = Gen::new(idx, fl);
    g.line(1, &format!("v->_d = ({dctype})({dval});"));
    if let Some(m) = sel.map(|i| &u.cases[i].field) {
        if !matches!(m.field_type, IdlType::Primitive(PrimitiveType::Void)) {
            g.default_value(1, &format!("v->_u.{}", c_ident(&m.name)), &m.field_type);
        }
    }
    if let Some(e) = g.error.take() {
        return Err(e);
    }
    let head = format!("{}({}* v, {}* c)", fl.def_name(n.ctype), n.ctype, fl.rcur());
    Ok(function(fl, inline, &head, &g, ""))
}

/// Width, signedness and C type of a union's discriminator.
fn discriminator(idx: &TypeIndex, fl: Flavor, u: &Union) -> Result<(usize, bool, String), String> {
    let (width, signed) = disc_holder(idx, &u.discriminator).ok_or_else(|| {
        format!(
            "discriminator type `{}` is not an integer, char, boolean or enumeration",
            u.discriminator.to_idl_string()
        )
    })?;
    Ok((width, signed, disc_ctype(idx, fl, &u.discriminator)))
}

/// The encoder and decoder of a MUTABLE union (rule 27): a DHEADER, the
/// discriminator as member 0 (its `EMHEADER1` with the M flag: "the
/// discriminator member shall always have the must understand attribute
/// set to true", 7.2.2.4.4.4.6), then the selected case as a member with
/// its id ([`data_model::union_case_ids`]), length code and M flag, a
/// NEXTINT for length code 4. The decoder reads the members by id (an
/// unknown member is skipped, or refused with the M flag), requires the
/// discriminator, and refuses a case member other than the one the
/// discriminator selects (or any case member when it selects none).
#[allow(clippy::many_single_char_names)]
fn mutable_union_codec(
    idx: &TypeIndex,
    fl: Flavor,
    inline: &str,
    u: &Union,
    n: &Names<'_>,
    key_only: bool,
) -> Result<String, String> {
    let (width, signed, dctype) = discriminator(idx, fl, u)?;
    let ids = data_model::union_case_ids(idx, u);
    if let Some(i) = u.cases.iter().zip(&ids).position(|(c, id)| {
        *id == 0 && !matches!(c.field.field_type, IdlType::Primitive(PrimitiveType::Void))
    }) {
        return Err(format!(
            "case `{}` has member id 0, the id of the discriminator of a @mutable union",
            u.cases[i].field.name
        ));
    }
    let dlc = data_model::discriminator_length_code(idx, &u.discriminator);
    let dword = (1u32 << 31) | (dlc << 28);
    let member = |m: &Field| format!("v->_u.{}", c_ident(&m.name));
    let is_void = |m: &Field| matches!(m.field_type, IdlType::Primitive(PrimitiveType::Void));
    let case_word = |g: &Gen<'_>, i: usize| {
        let m = &u.cases[i].field;
        let lc = length_code(g.idx, &m.field_type);
        let mu = u32::from(must_understand(m));
        ((mu << 31) | (lc << 28) | (ids[i] & 0x0fff_ffff), lc)
    };

    // ---- encode ----
    let mut g = Gen::new(idx, fl);
    g.key_only = key_only;
    g.try_(1, "wbegin(c, &dh)");
    g.try_(1, &format!("w32(c, 0x{dword:08x}u)"));
    let disc = if signed {
        format!("({})({})(v->_d)", uint(width), sint(width))
    } else {
        format!("({})(v->_d)", uint(width))
    };
    g.try_(1, &format!("w{}(c, {disc})", bits(width)));
    case_chain(&mut g, u, &dctype, false, &mut |g, i| {
        let Some(i) = i else { return };
        let m = &u.cases[i].field;
        if is_void(m) {
            return;
        }
        let (word, lc) = case_word(g, i);
        g.try_(2, &format!("w32(c, 0x{word:08x}u)"));
        if lc == 4 {
            g.line(2, "{");
            g.line(3, &format!("{} hx_nx;", g.fl.pos()));
            g.try_(3, "wbegin(c, &hx_nx)");
            g.enc(3, &member(m), &m.field_type);
            g.try_(3, "wend(c, hx_nx)");
            g.line(2, "}");
        } else {
            g.enc(2, &member(m), &m.field_type);
        }
    });
    g.try_(1, "wend(c, dh)");
    if let Some(e) = g.error.take() {
        return Err(e);
    }
    let locals = format!("    {} dh;\n", fl.pos());
    let encoder = encoder_name(n.enc, key_only);
    let enc_head = format!("{encoder}(const {}* v, {}* c)", n.ctype, fl.wcur());
    let mut out = function(fl, inline, &enc_head, &g, &locals);

    // ---- decode ----
    let mut g = Gen::new(idx, fl);
    g.line(1, "dseen = 0;");
    g.line(1, "cseen = 0;");
    g.line(1, "cid = 0;");
    g.try_(1, "rbegin(c, &dh)");
    g.line(1, "while (c->pos < dh) {");
    g.try_(2, "remh(c, dh, &mu, &id, &mend)");
    g.line(2, "switch (id) {");
    g.line(2, "case 0x00000000u:");
    g.line(3, "{");
    g.line(4, &format!("{} t;", uint(width)));
    g.try_(4, &format!("r{}(c, &t)", bits(width)));
    if signed {
        g.line(4, &format!("v->_d = ({dctype})({})t;", sint(width)));
    } else {
        g.line(4, &format!("v->_d = ({dctype})t;"));
    }
    g.line(3, "}");
    g.line(3, "dseen = 1;");
    g.line(3, "break;");
    for (i, case) in u.cases.iter().enumerate() {
        if is_void(&case.field) {
            continue;
        }
        g.line(2, &format!("case 0x{:08x}u:", ids[i]));
        g.line(3, &format!("if (cseen) {{ return {}; }}", fl.einv()));
        g.dec(3, &member(&case.field), &case.field.field_type);
        g.line(3, "cseen = 1;");
        g.line(3, "cid = id;");
        g.line(3, "break;");
    }
    g.line(2, "default:");
    g.line(3, &format!("if (mu) {{ return {}; }}", fl.einv()));
    g.line(3, "break;");
    g.line(2, "}");
    g.try_(2, "rskip(c, mend)");
    g.line(1, "}");
    g.try_(1, "rskip(c, dh)");
    g.line(1, &format!("if (!dseen) {{ return {}; }}", fl.einv()));
    let einv = fl.einv();
    case_chain(&mut g, u, &dctype, true, &mut |g, i| match i {
        Some(i) if !is_void(&u.cases[i].field) => g.line(
            2,
            &format!(
                "if (!cseen || cid != 0x{:08x}u) {{ return {einv}; }}",
                ids[i]
            ),
        ),
        _ => g.line(2, &format!("if (cseen) {{ return {einv}; }}")),
    });
    if let Some(e) = g.error.take() {
        return Err(e);
    }
    let dlocals = format!(
        "    {pos} dh;
    {pos} mend;
    uint32_t id;
    uint32_t cid;
    int mu;
    int dseen;
    int cseen;
",
        pos = fl.pos()
    );
    let dec_head = format!("{}({}* v, {}* c)", n.dec, n.ctype, fl.rcur());
    out.push_str(&function(fl, inline, &dec_head, &g, &dlocals));
    out.push_str(&union_default_fn(idx, fl, inline, u, n, &dctype)?);
    Ok(out)
}

// -------------------------------------------------------------------
// Static upper bound (C-micro `*_ENCODED_SIZE_MAX`)
// -------------------------------------------------------------------

/// An upper bound of the XCDR2 size of a value of `ty` held by the
/// C-micro types (every collection has a capacity): every scalar and
/// length word counted with the at most 3 padding bytes before it. `None`
/// when the type has no bound (an unknown type, a cycle).
#[must_use]
pub fn upper_bound(idx: &TypeIndex, fl: Flavor, ty: &IdlType, depth: usize) -> Option<u64> {
    if depth > 32 {
        return None;
    }
    let g = Gen::new(idx, fl);
    let word = 7u64; // a length word and its padding
    let scalar = |w: usize| (w as u64) + if w > 1 { 3 } else { 0 };
    Some(match g.kind(ty) {
        Kind::Prim(p) => data_model::primitive_size(&p).map_or(19, scalar),
        Kind::Str { wide, bound } => {
            let cap = match (fl, bound) {
                (_, Some(b)) => u64::from(b),
                (Flavor::Micro { max_string, .. }, None) => u64::from(max_string),
                (Flavor::C, None) => return None,
            };
            word + if wide { cap * 2 } else { cap + 1 }
        }
        Kind::Seq { inner, bound } => {
            let cap = g.seq_cap(bound)?;
            let dh = if collection_has_dheader(idx, ty) {
                word
            } else {
                0
            };
            dh + word + u64::from(cap) * upper_bound(idx, fl, &inner, depth + 1)?
        }
        Kind::Array { elem, dims } => {
            let dh = if collection_has_dheader(idx, ty) {
                word
            } else {
                0
            };
            let n: u64 = dims.iter().map(|d| u64::from(*d)).product();
            dh + n * upper_bound(idx, fl, &elem, depth + 1)?
        }
        Kind::Map { .. } | Kind::Unknown(_) => return None,
        Kind::Enum { width, .. } | Kind::Bitmask { width, .. } => scalar(width),
        Kind::Bitset => scalar(8),
        Kind::Aggregate(n) => {
            if let Some(s) = idx.struct_def(&n) {
                struct_bound(idx, fl, s, depth + 1)?
            } else {
                let u = idx.union_def(&n)?;
                let mut max = 0u64;
                for c in &u.cases {
                    max = max.max(upper_bound(idx, fl, &c.field.field_type, depth + 1)?);
                }
                let framing = match union_ext(u) {
                    Ext::Final => 0,
                    Ext::Appendable => word,
                    // DHEADER, two EMHEADER1 and a NEXTINT
                    Ext::Mutable => word + 2 * word + 4,
                };
                framing + 11 + max
            }
        }
    })
}

/// [`upper_bound`] of a structure.
#[must_use]
pub fn struct_bound(idx: &TypeIndex, fl: Flavor, s: &Struct, depth: usize) -> Option<u64> {
    let chain = member_chain(idx, s).ok()?;
    let ext = data_model::struct_ext(s);
    let mut total = if ext == Ext::Final { 0 } else { 7 };
    for m in &chain {
        total += upper_bound(idx, fl, &m.field.field_type, depth)?;
        if ext == Ext::Mutable {
            total += 7 + 4; // EMHEADER1 (and padding), NEXTINT
        } else if m.field.is_optional() {
            total += 1;
        }
    }
    Some(total)
}
