// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! XCDR1 / XCDR2 codec of the generated Rust types (DDS-`XTypes` 1.3
//! 7.4.3.5.3).
//!
//! Every structure and union gets two hidden methods, `__hdds_enc` and
//! `__hdds_dec`, that serialize the value through the cursor types of the
//! `__hdds_cdr` module emitted once per generated file. The cursor carries
//! the version (so one body serves XCDR1 and XCDR2), the position, and the
//! origin alignment is measured from: the start of the buffer handed to the
//! public entry point, reset at the value of every XCDR1 parameter (rules
//! 24-25). Alignment is therefore global: a nested type is aligned where it
//! lands in the enclosing stream, never relative to its own first byte.
//!
//! The public methods keep their names: `encode_xcdr1_le`,
//! `encode_xcdr2_le`, their `_at` forms, `decode_xcdr1_le`,
//! `decode_xcdr2_le` and theirs, and `max_xcdr1_size` / `max_xcdr2_size`,
//! which now return the exact serialized size of the value (a measuring run
//! of the encoder).

#![allow(clippy::uninlined_format_args)]

use std::fmt::Write as _;

use super::super::data_model::{
    self, bitmask_width, collection_has_dheader, enum_width, flatten_array, length_code,
    member_chain, must_understand, union_case_disc, union_ext, union_label_value, Ext,
};
use super::super::key_holder::{Named, TypeIndex};
use super::super::keywords::rust_ident;
use super::RustGenerator;
use crate::ast::{Enum, Field, Struct, Union, UnionLabel};
use crate::types::{IdlType, PrimitiveType};

/// Name of the generated cursor module.
pub(super) const CDR_MOD: &str = "__hdds_cdr";

/// The cursor module, emitted once at the top of every generated file.
#[allow(clippy::too_many_lines)]
pub(super) fn cdr_module() -> String {
    format!(
        r#"/// XCDR1 / XCDR2 little-endian cursors of the generated codec (generated
/// by hddsgen; DDS-XTypes 1.3 7.4.3). Alignment is measured from `origin`,
/// capped at 4 under XCDR2 and at 8 under XCDR1; padding is zero.
#[doc(hidden)]
#[allow(dead_code, clippy::all, clippy::pedantic)]
pub mod {m} {{
    use ::hdds::CdrError;

    /// Encoding cursor. `measure` counts the bytes without writing them.
    pub struct Enc<'a> {{
        buf: &'a mut [u8],
        pub pos: usize,
        origin: usize,
        pub xcdr2: bool,
        measure: bool,
    }}

    impl<'a> Enc<'a> {{
        pub fn new(buf: &'a mut [u8], pos: usize, xcdr2: bool) -> Self {{
            Self {{ buf, pos, origin: 0, xcdr2, measure: false }}
        }}

        pub fn measure(xcdr2: bool) -> Enc<'static> {{
            Enc {{ buf: &mut [], pos: 0, origin: 0, xcdr2, measure: true }}
        }}

        fn reserve(&mut self, n: usize) -> Result<usize, CdrError> {{
            let at = self.pos;
            let end = at.checked_add(n).ok_or(CdrError::DataTooLarge)?;
            if !self.measure && end > self.buf.len() {{
                return Err(CdrError::BufferTooSmall);
            }}
            self.pos = end;
            Ok(at)
        }}

        /// Bytes, no alignment.
        pub fn raw(&mut self, b: &[u8]) -> Result<(), CdrError> {{
            let at = self.reserve(b.len())?;
            if !self.measure {{
                self.buf[at..at + b.len()].copy_from_slice(b);
            }}
            Ok(())
        }}

        /// Zero padding up to a multiple of `size` (capped) from the origin.
        pub fn align(&mut self, size: usize) -> Result<(), CdrError> {{
            let a = size.min(if self.xcdr2 {{ 4 }} else {{ 8 }});
            if a > 1 {{
                let pad = (a - (self.pos - self.origin) % a) % a;
                if pad > 0 {{
                    let at = self.reserve(pad)?;
                    if !self.measure {{
                        self.buf[at..at + pad].fill(0);
                    }}
                }}
            }}
            Ok(())
        }}

        /// A primitive: aligned to its size, then its bytes.
        pub fn put(&mut self, b: &[u8]) -> Result<(), CdrError> {{
            self.align(b.len())?;
            self.raw(b)
        }}

        /// A length or element count.
        pub fn count(&mut self, n: usize) -> Result<(), CdrError> {{
            let n = u32::try_from(n).map_err(|_| CdrError::DataTooLarge)?;
            self.put(&n.to_le_bytes())
        }}

        /// Refuse a collection longer than its bound (0: unbounded).
        pub fn bounded(n: usize, bound: usize) -> Result<(), CdrError> {{
            if bound != 0 && n > bound {{
                return Err(CdrError::Other(::std::format!(
                    "{{n}} elements exceed the bound {{bound}}"
                )));
            }}
            Ok(())
        }}

        /// `string`: length counting the NUL, the bytes, the NUL.
        pub fn string(&mut self, s: &str, bound: usize) -> Result<(), CdrError> {{
            Self::bounded(s.len(), bound)?;
            self.count(s.len() + 1)?;
            self.raw(s.as_bytes())?;
            self.raw(&[0])
        }}

        /// `wstring`: length in bytes, UTF-16 code units, no NUL.
        pub fn wstring(&mut self, s: &str, bound: usize) -> Result<(), CdrError> {{
            let units = s.encode_utf16().count();
            Self::bounded(units, bound)?;
            self.count(units * 2)?;
            for u in s.encode_utf16() {{
                self.raw(&u.to_le_bytes())?;
            }}
            Ok(())
        }}

        /// `wchar`: one UTF-16 code unit.
        pub fn wchar(&mut self, c: char) -> Result<(), CdrError> {{
            let mut b = [0u16; 2];
            let units = c.encode_utf16(&mut b);
            if units.len() != 1 {{
                return Err(CdrError::InvalidEncoding);
            }}
            self.put(&units[0].to_le_bytes())
        }}

        /// `char`: one byte.
        pub fn char8(&mut self, c: char) -> Result<(), CdrError> {{
            let b = u8::try_from(u32::from(c)).map_err(|_| CdrError::InvalidEncoding)?;
            self.raw(&[b])
        }}

        /// Reserve a length word (DHEADER, NEXTINT), aligned at 4.
        pub fn begin_len(&mut self) -> Result<usize, CdrError> {{
            self.align(4)?;
            self.reserve(4)
        }}

        /// Fill a reserved length word with the bytes written after it.
        pub fn end_len(&mut self, at: usize) -> Result<(), CdrError> {{
            let n = u32::try_from(self.pos - at - 4).map_err(|_| CdrError::DataTooLarge)?;
            if !self.measure {{
                self.buf[at..at + 4].copy_from_slice(&n.to_le_bytes());
            }}
            Ok(())
        }}

        /// XCDR2 `EMHEADER1` (7.4.3.4.2).
        pub fn emheader(&mut self, must_understand: bool, lc: u32, id: u32) -> Result<(), CdrError> {{
            let word = (u32::from(must_understand) << 31) | (lc << 28) | (id & 0x0fff_ffff);
            self.put(&word.to_le_bytes())
        }}

        /// XCDR1 parameter header, long form (rule 25, 7.4.1.2.2):
        /// PID_EXTENDED with FLAG_M (0x7F01), 8, the member header (FLAG_2
        /// for a member that must be understood, the member id), the member
        /// length (patched by `pl1_end`). The value's alignment origin is
        /// its first byte.
        pub fn pl1_begin(&mut self, id: u32, must_understand: bool) -> Result<(usize, usize), CdrError> {{
            self.align(4)?;
            self.raw(&0x7f01u16.to_le_bytes())?;
            self.raw(&8u16.to_le_bytes())?;
            let header = (u32::from(must_understand) << 30) | (id & 0x0fff_ffff);
            self.raw(&header.to_le_bytes())?;
            let at = self.reserve(4)?;
            let previous = self.origin;
            self.origin = self.pos;
            Ok((at, previous))
        }}

        /// End an XCDR1 parameter: its length is the exact length of the
        /// member, without the padding the next header brings (7.4.1.2.2).
        pub fn pl1_end(&mut self, header: (usize, usize)) -> Result<(), CdrError> {{
            self.origin = header.1;
            self.end_len(header.0)
        }}

        /// PID_LIST_END with FLAG_M (0x7F02), length 0, closing the list.
        pub fn pl1_sentinel(&mut self) -> Result<(), CdrError> {{
            self.align(4)?;
            self.raw(&0x7f02u16.to_le_bytes())?;
            self.raw(&0u16.to_le_bytes())
        }}
    }}

    /// Storage the sequences and maps of one decode may hold, all together,
    /// in bytes per byte of the body (and a single collection, per byte
    /// left when its count is read): a count asking for more is refused
    /// before any storage is reserved. An element can take far more memory
    /// than wire bytes: an absent `@optional` member (1 byte, or 4 under
    /// XCDR1, for a whole value), a union whose selected branch is small
    /// while another is large, an APPENDABLE element from an older writer
    /// (its missing trailing members take their default). Edit to accept
    /// denser data.
    pub const MAX_ALLOC_RATIO: usize = 64;

    /// Most values of types that can take no byte on the wire (an empty
    /// structure, an array of them) one decode may read through sequences
    /// and maps, all collections together: nothing else bounds their
    /// counts. Edit to accept more.
    pub const MAX_ZERO_WIRE_ELEMENTS: usize = 1024;

    /// Decoding cursor.
    pub struct Dec<'a> {{
        buf: &'a [u8],
        pub pos: usize,
        origin: usize,
        pub xcdr2: bool,
        /// Collection storage this decode may still request, in bytes.
        storage_left: usize,
        /// Values of zero-wire-size types this decode may still read.
        zero_wire_left: usize,
    }}

    impl<'a> Dec<'a> {{
        pub fn new(buf: &'a [u8], pos: usize, xcdr2: bool) -> Self {{
            let body = buf.len().saturating_sub(pos);
            Self {{
                buf,
                pos,
                origin: 0,
                xcdr2,
                storage_left: MAX_ALLOC_RATIO.saturating_mul(body),
                zero_wire_left: MAX_ZERO_WIRE_ELEMENTS,
            }}
        }}

        /// Bytes left in the buffer.
        pub fn remaining(&self) -> usize {{
            self.buf.len().saturating_sub(self.pos)
        }}

        /// Refuse a collection of `n` elements of `T` before reserving or
        /// filling storage for it. The budgets are per decode, shared by
        /// every collection however nested: the storage of all collections
        /// together may not exceed [`MAX_ALLOC_RATIO`] bytes per byte of
        /// the body (nor this one the ratio times the bytes left), and the
        /// values of types that take no byte on the wire, `zero_wire` per
        /// element (nested ones included; 0 for any other type), may not
        /// exceed [`MAX_ZERO_WIRE_ELEMENTS`] in all (a collection of a
        /// single element is always read, its values still counted).
        pub fn storage<T>(&mut self, n: usize, zero_wire: usize) -> Result<(), CdrError> {{
            if zero_wire != 0 {{
                let values = n.saturating_mul(zero_wire);
                if n > 1 && values > self.zero_wire_left {{
                    return Err(CdrError::InvalidEncoding);
                }}
                self.zero_wire_left = self.zero_wire_left.saturating_sub(values);
            }}
            let bytes = n.saturating_mul(::core::mem::size_of::<T>());
            if bytes > MAX_ALLOC_RATIO.saturating_mul(self.remaining()) || bytes > self.storage_left {{
                return Err(CdrError::InvalidEncoding);
            }}
            self.storage_left -= bytes;
            Ok(())
        }}

        pub fn take(&mut self, n: usize) -> Result<&'a [u8], CdrError> {{
            let end = self.pos.checked_add(n).ok_or(CdrError::UnexpectedEof)?;
            if end > self.buf.len() {{
                return Err(CdrError::UnexpectedEof);
            }}
            let s = &self.buf[self.pos..end];
            self.pos = end;
            Ok(s)
        }}

        pub fn align(&mut self, size: usize) -> Result<(), CdrError> {{
            let a = size.min(if self.xcdr2 {{ 4 }} else {{ 8 }});
            if a > 1 {{
                let pad = (a - (self.pos - self.origin) % a) % a;
                self.take(pad)?;
            }}
            Ok(())
        }}

        /// `N` bytes, no alignment.
        pub fn raw<const N: usize>(&mut self) -> Result<[u8; N], CdrError> {{
            let s = self.take(N)?;
            let mut a = [0u8; N];
            a.copy_from_slice(s);
            Ok(a)
        }}

        /// A primitive of `N` bytes, aligned to `N`.
        pub fn get<const N: usize>(&mut self) -> Result<[u8; N], CdrError> {{
            self.align(N)?;
            self.raw::<N>()
        }}

        pub fn count(&mut self) -> Result<usize, CdrError> {{
            Ok(u32::from_le_bytes(self.get::<4>()?) as usize)
        }}

        /// Refuse a collection longer than its bound (0: unbounded).
        pub fn bounded(n: usize, bound: usize) -> Result<(), CdrError> {{
            if bound != 0 && n > bound {{
                return Err(CdrError::InvalidEncoding);
            }}
            Ok(())
        }}

        pub fn string(&mut self, bound: usize) -> Result<::std::string::String, CdrError> {{
            let n = self.count()?;
            if n == 0 {{
                return Ok(::std::string::String::new());
            }}
            let b = self.take(n)?;
            if b[n - 1] != 0 {{
                return Err(CdrError::InvalidEncoding);
            }}
            Self::bounded(n - 1, bound)?;
            ::std::string::String::from_utf8(b[..n - 1].to_vec()).map_err(|_| CdrError::InvalidEncoding)
        }}

        pub fn wstring(&mut self, bound: usize) -> Result<::std::string::String, CdrError> {{
            let n = self.count()?;
            if n % 2 != 0 {{
                return Err(CdrError::InvalidEncoding);
            }}
            let b = self.take(n)?;
            Self::bounded(n / 2, bound)?;
            let units: ::std::vec::Vec<u16> =
                b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            ::std::string::String::from_utf16(&units).map_err(|_| CdrError::InvalidEncoding)
        }}

        pub fn wchar(&mut self) -> Result<char, CdrError> {{
            let u = u16::from_le_bytes(self.get::<2>()?);
            char::from_u32(u32::from(u)).ok_or(CdrError::InvalidEncoding)
        }}

        pub fn char8(&mut self) -> Result<char, CdrError> {{
            Ok(char::from(self.raw::<1>()?[0]))
        }}

        pub fn boolean(&mut self) -> Result<bool, CdrError> {{
            match self.raw::<1>()?[0] {{
                0 => Ok(false),
                1 => Ok(true),
                _ => Err(CdrError::InvalidEncoding),
            }}
        }}

        /// A length word (DHEADER): the end of the bytes it delimits.
        pub fn begin_len(&mut self) -> Result<usize, CdrError> {{
            let n = self.count()?;
            let end = self.pos.checked_add(n).ok_or(CdrError::UnexpectedEof)?;
            if end > self.buf.len() {{
                return Err(CdrError::UnexpectedEof);
            }}
            Ok(end)
        }}

        /// Move to `end`, skipping what a newer writer appended; an `end`
        /// behind the cursor means the value overran its delimiter.
        pub fn skip_to(&mut self, end: usize) -> Result<(), CdrError> {{
            if end < self.pos {{
                return Err(CdrError::InvalidEncoding);
            }}
            self.pos = end;
            Ok(())
        }}

        /// XCDR2 `EMHEADER1` and its NEXTINT (7.4.3.4.2): the
        /// must-understand flag, the member id and the end of the member,
        /// the cursor at the value (on the NEXTINT for LC 5 to 7).
        pub fn emheader(&mut self, end: usize) -> Result<(bool, u32, usize), CdrError> {{
            let h = u32::from_le_bytes(self.get::<4>()?);
            let mu = h >> 31 == 1;
            let id = h & 0x0fff_ffff;
            let size = match (h >> 28) & 7 {{
                0 => 1,
                1 => 2,
                2 => 4,
                3 => 8,
                lc => {{
                    let n = u32::from_le_bytes(self.raw::<4>()?) as usize;
                    match lc {{
                        4 => n,
                        _ => {{
                            self.pos -= 4;
                            let unit = match lc {{ 5 => 1, 6 => 4, _ => 8 }};
                            n.checked_mul(unit).and_then(|v| v.checked_add(4)).ok_or(CdrError::InvalidEncoding)?
                        }}
                    }}
                }}
            }};
            let member_end = self.pos.checked_add(size).ok_or(CdrError::InvalidEncoding)?;
            if member_end > end {{
                return Err(CdrError::InvalidEncoding);
            }}
            Ok((mu, id, member_end))
        }}

        /// Next XCDR1 parameter (rules 24-25, 7.4.1.2.2): its member id,
        /// its must-understand flag (FLAG_2 of an extended header, FLAG_M of
        /// a short one), its end and the origin to restore, the cursor (and
        /// the origin) at the value; `None` at PID_LIST_END.
        pub fn pl1_member(&mut self) -> Result<Option<(u32, bool, usize, usize)>, CdrError> {{
            loop {{
                self.align(4)?;
                let raw_pid = u16::from_le_bytes(self.raw::<2>()?);
                let pid = raw_pid & 0x3fff;
                let len = usize::from(u16::from_le_bytes(self.raw::<2>()?));
                if pid == 0x3f02 {{
                    return Ok(None);
                }}
                let (id, mu, size) = if pid == 0x3f01 {{
                    let header = u32::from_le_bytes(self.raw::<4>()?);
                    let size = u32::from_le_bytes(self.raw::<4>()?) as usize;
                    (header & 0x0fff_ffff, header & 0x4000_0000 != 0, size)
                }} else {{
                    (u32::from(pid), raw_pid & 0x4000 != 0, len)
                }};
                let end = self.pos.checked_add(size).ok_or(CdrError::UnexpectedEof)?;
                if end > self.buf.len() {{
                    return Err(CdrError::UnexpectedEof);
                }}
                if pid == 0x3f03 {{
                    self.pos = end;
                    continue;
                }}
                let previous = self.origin;
                self.origin = self.pos;
                return Ok(Some((id, mu, end, previous)));
            }}
        }}

        /// Leave an XCDR1 parameter: move to its end, restore the origin.
        pub fn pl1_done(&mut self, end: usize, previous: usize) -> Result<(), CdrError> {{
            self.skip_to(end)?;
            self.origin = previous;
            Ok(())
        }}
    }}
}}

"#,
        m = CDR_MOD
    )
}

/// Code emitter: the type index, the output and a counter for unique local
/// names.
struct Gen<'a> {
    idx: &'a TypeIndex,
    out: String,
    var: usize,
    /// `zero_wire` of each named type already computed.
    zero_wire_of: ::std::cell::RefCell<::std::collections::HashMap<String, u64>>,
}

impl<'a> Gen<'a> {
    fn new(idx: &'a TypeIndex) -> Self {
        Self {
            idx,
            out: String::new(),
            var: 0,
            zero_wire_of: ::std::cell::RefCell::default(),
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
}

/// How a resolved type serializes, as far as the Rust codec cares.
enum Kind {
    Prim(PrimitiveType),
    Str {
        wide: bool,
        bound: usize,
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
    Aggregate(String),
    Enum {
        name: String,
        width: usize,
    },
    Bitmask {
        width: usize,
    },
    Bitset(String),
    Unknown(String),
}

fn bound_of(b: Option<u32>) -> usize {
    b.map_or(0, |n| n as usize)
}

impl Gen<'_> {
    fn kind(&self, ty: &IdlType) -> Kind {
        let resolved = self.idx.resolve(ty);
        match resolved {
            IdlType::Primitive(PrimitiveType::String) => Kind::Str {
                wide: false,
                bound: 0,
            },
            IdlType::Primitive(PrimitiveType::WString) => Kind::Str {
                wide: true,
                bound: 0,
            },
            IdlType::Primitive(p) => Kind::Prim(p),
            IdlType::BoundedString { wide, bound } => Kind::Str {
                wide,
                bound: bound as usize,
            },
            IdlType::Sequence { ref inner, bound } => Kind::Seq {
                inner: (**inner).clone(),
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
                Some(Named::Struct(_) | Named::Union(_)) => Kind::Aggregate(n),
                Some(Named::Enum { bit_bound, .. }) => Kind::Enum {
                    name: n.clone(),
                    width: enum_width(*bit_bound),
                },
                Some(Named::Bitmask { bit_bound }) => Kind::Bitmask {
                    width: bitmask_width(*bit_bound),
                },
                Some(Named::Bitset) => Kind::Bitset(n),
                Some(Named::Typedef(_)) | None => Kind::Unknown(n),
            },
        }
    }

    // -----------------------------------------------------------------------
    // Encoding: statements writing the value at `place` (a place expression
    // of the member's Rust type) through the cursor `__e`.
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_lines)]
    fn enc(&mut self, ind: usize, place: &str, ty: &IdlType) {
        match self.kind(ty) {
            Kind::Prim(p) => self.enc_prim(ind, place, p),
            Kind::Str { wide: false, bound } => {
                self.line(ind, &format!("__e.string({}, {bound})?;", borrow_of(place)));
            }
            Kind::Str { wide: true, bound } => {
                self.line(
                    ind,
                    &format!("__e.wstring({}, {bound})?;", borrow_of(place)),
                );
            }
            Kind::Seq { inner, bound } => {
                let dh = collection_has_dheader(self.idx, ty);
                let d = self.fresh("d");
                if dh {
                    self.line(
                        ind,
                        &format!(
                            "let {d} = if __e.xcdr2 {{ Some(__e.begin_len()?) }} else {{ None }};"
                        ),
                    );
                }
                self.line(
                    ind,
                    &format!("{CDR_MOD}::Enc::bounded({place}.len(), {bound})?;"),
                );
                self.line(ind, &format!("__e.count({place}.len())?;"));
                if self.is_octet(&inner) {
                    self.line(ind, &format!("__e.raw({})?;", borrow_of(place)));
                } else {
                    let x = self.fresh("x");
                    self.line(ind, &format!("for {x} in {place}.iter() {{"));
                    self.enc(ind + 1, &format!("(*{x})"), &inner);
                    self.line(ind, "}");
                }
                if dh {
                    self.line(
                        ind,
                        &format!("if let Some({d}) = {d} {{ __e.end_len({d})?; }}"),
                    );
                }
            }
            Kind::Array { elem, dims } => {
                let dh = collection_has_dheader(self.idx, ty);
                let d = self.fresh("d");
                if dh {
                    self.line(
                        ind,
                        &format!(
                            "let {d} = if __e.xcdr2 {{ Some(__e.begin_len()?) }} else {{ None }};"
                        ),
                    );
                }
                if dims.len() == 1 && self.is_octet(&elem) {
                    self.line(ind, &format!("__e.raw({})?;", borrow_of(place)));
                } else {
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
                if dh {
                    self.line(
                        ind,
                        &format!("if let Some({d}) = {d} {{ __e.end_len({d})?; }}"),
                    );
                }
            }
            Kind::Map { key, value, bound } => {
                let dh = collection_has_dheader(self.idx, ty);
                let d = self.fresh("d");
                if dh {
                    self.line(
                        ind,
                        &format!(
                            "let {d} = if __e.xcdr2 {{ Some(__e.begin_len()?) }} else {{ None }};"
                        ),
                    );
                }
                self.line(
                    ind,
                    &format!("{CDR_MOD}::Enc::bounded({place}.len(), {bound})?;"),
                );
                self.line(ind, &format!("__e.count({place}.len())?;"));
                let k = self.fresh("k");
                let v = self.fresh("v");
                self.line(ind, &format!("for ({k}, {v}) in {place}.iter() {{"));
                self.enc(ind + 1, &format!("(*{k})"), &key);
                self.enc(ind + 1, &format!("(*{v})"), &value);
                self.line(ind, "}");
                if dh {
                    self.line(
                        ind,
                        &format!("if let Some({d}) = {d} {{ __e.end_len({d})?; }}"),
                    );
                }
            }
            Kind::Aggregate(_) => self.line(ind, &format!("{place}.__hdds_enc(__e)?;")),
            Kind::Enum { width, .. } => {
                let int = match width {
                    1 => "i8",
                    2 => "i16",
                    _ => "i32",
                };
                self.line(
                    ind,
                    &format!("__e.put(&(({place} as i64) as {int}).to_le_bytes())?;"),
                );
            }
            Kind::Bitmask { width } => {
                let int = uint(width);
                // The value is a `u64`: a cast to `u64` would be an identity.
                let stmt = if width == 8 {
                    format!("__e.put(&{place}.to_le_bytes())?;")
                } else {
                    format!("__e.put(&({} as {int}).to_le_bytes())?;", as_value(place))
                };
                self.line(ind, &stmt);
            }
            Kind::Bitset(_) => {
                self.line(ind, &format!("__e.put(&{place}.bits().to_le_bytes())?;"));
            }
            Kind::Unknown(n) => {
                self.line(
                    ind,
                    &format!(
                        "compile_error!(\"hddsgen: type `{n}` is not defined in this IDL file\");"
                    ),
                );
            }
        }
    }

    fn is_octet(&self, ty: &IdlType) -> bool {
        matches!(
            self.idx.resolve(ty),
            IdlType::Primitive(PrimitiveType::Octet | PrimitiveType::UInt8)
        )
    }

    fn enc_prim(&mut self, ind: usize, place: &str, p: PrimitiveType) {
        let value = as_value(place);
        let stmt = match p {
            PrimitiveType::Boolean => format!("__e.raw(&[u8::from({value})])?;"),
            PrimitiveType::Octet | PrimitiveType::UInt8 => format!("__e.raw(&[{value}])?;"),
            PrimitiveType::Int8 => format!("__e.raw(&{place}.to_le_bytes())?;"),
            PrimitiveType::Char => format!("__e.char8({value})?;"),
            PrimitiveType::WChar => format!("__e.wchar({value})?;"),
            PrimitiveType::Fixed { .. } => {
                format!(
                    "__e.align(4)?;\n{}__e.raw(&{place}.raw().to_le_bytes())?;",
                    "    ".repeat(ind)
                )
            }
            PrimitiveType::Void => return,
            PrimitiveType::String | PrimitiveType::WString => {
                format!("__e.string({}, 0)?;", borrow_of(place))
            }
            _ => format!("__e.put(&{place}.to_le_bytes())?;"),
        };
        self.line(ind, &stmt);
    }

    // -----------------------------------------------------------------------
    // Decoding: an expression (often a block) of the member's Rust type,
    // read through the cursor `__d`.
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_lines)]
    fn dec(&mut self, ty: &IdlType) -> String {
        match self.kind(ty) {
            Kind::Prim(p) => dec_prim(p),
            Kind::Str { wide: false, bound } => format!("__d.string({bound})?"),
            Kind::Str { wide: true, bound } => format!("__d.wstring({bound})?"),
            Kind::Seq { inner, bound } => {
                let dh = collection_has_dheader(self.idx, ty);
                let n = self.fresh("n");
                let v = self.fresh("s");
                let mut b = String::from("{\n");
                if dh {
                    b.push_str(
                        "let __end = if __d.xcdr2 { Some(__d.begin_len()?) } else { None };\n",
                    );
                }
                let _ = writeln!(b, "let {n} = __d.count()?;");
                let _ = writeln!(b, "{CDR_MOD}::Dec::bounded({n}, {bound})?;");
                if self.is_octet(&inner) {
                    let _ = writeln!(b, "let {v} = __d.take({n})?.to_vec();");
                } else {
                    let elem = self.dec(&inner);
                    let _ = writeln!(
                        b,
                        "__d.storage::<{}>({n}, {}usize)?;",
                        RustGenerator::type_to_rust(&inner),
                        self.zero_wire(&inner, 0).min(u64::from(u32::MAX))
                    );
                    let _ = writeln!(
                        b,
                        "let mut {v} = ::std::vec::Vec::with_capacity({n}.min(__d.remaining()));"
                    );
                    let _ = writeln!(b, "for _ in 0..{n} {{ {v}.push({elem}); }}");
                }
                if dh {
                    b.push_str("if let Some(__end) = __end { __d.skip_to(__end)?; }\n");
                }
                let _ = write!(b, "{v}\n}}");
                b
            }
            Kind::Array { elem, dims } => {
                let dh = collection_has_dheader(self.idx, ty);
                let value = if dims.len() == 1 && self.is_octet(&elem) {
                    let n = dims[0];
                    format!(
                        "<[u8; {n}]>::try_from(__d.take({n})?).map_err(|_| ::hdds::CdrError::InvalidEncoding)?"
                    )
                } else {
                    self.dec_array_dims(&elem, &dims)
                };
                if !dh {
                    return value;
                }
                let v = self.fresh("a");
                let mut b = String::from("{\n");
                b.push_str("let __end = if __d.xcdr2 { Some(__d.begin_len()?) } else { None };\n");
                let _ = writeln!(b, "let {v} = {value};");
                b.push_str("if let Some(__end) = __end { __d.skip_to(__end)?; }\n");
                let _ = write!(b, "{v}\n}}");
                b
            }
            Kind::Map { key, value, bound } => {
                let dh = collection_has_dheader(self.idx, ty);
                let n = self.fresh("n");
                let m = self.fresh("m");
                let kexpr = self.dec(&key);
                let vexpr = self.dec(&value);
                let mut b = String::from("{\n");
                if dh {
                    b.push_str(
                        "let __end = if __d.xcdr2 { Some(__d.begin_len()?) } else { None };\n",
                    );
                }
                let _ = writeln!(b, "let {n} = __d.count()?;");
                let _ = writeln!(b, "{CDR_MOD}::Dec::bounded({n}, {bound})?;");
                let _ = writeln!(
                    b,
                    "__d.storage::<({}, {})>({n}, 0usize)?;",
                    RustGenerator::type_to_rust(&key),
                    RustGenerator::type_to_rust(&value),
                );
                let _ = writeln!(
                    b,
                    "let mut {m} = ::std::collections::HashMap::with_capacity({n}.min(__d.remaining()));"
                );
                let _ = writeln!(
                    b,
                    "for _ in 0..{n} {{ let __k = {kexpr}; let __v = {vexpr}; {m}.insert(__k, __v); }}"
                );
                if dh {
                    b.push_str("if let Some(__end) = __end { __d.skip_to(__end)?; }\n");
                }
                let _ = write!(b, "{m}\n}}");
                b
            }
            Kind::Aggregate(n) => format!("{}::__hdds_dec(__d)?", rust_path(&n)),
            Kind::Enum { name, width } => {
                let read = match width {
                    1 => "i32::from(i8::from_le_bytes(__d.get::<1>()?))",
                    2 => "i32::from(i16::from_le_bytes(__d.get::<2>()?))",
                    _ => "i32::from_le_bytes(__d.get::<4>()?)",
                };
                format!("{}::__hdds_from_i32({read})?", rust_path(&name))
            }
            Kind::Bitmask { width } => {
                let int = uint(width);
                if width == 8 {
                    format!("{int}::from_le_bytes(__d.get::<{width}>()?)")
                } else {
                    format!("u64::from({int}::from_le_bytes(__d.get::<{width}>()?))")
                }
            }
            Kind::Bitset(n) => format!(
                "{}::from_bits(u64::from_le_bytes(__d.get::<8>()?))",
                rust_path(&n)
            ),
            Kind::Unknown(n) => {
                format!("compile_error!(\"hddsgen: type `{n}` is not defined in this IDL file\")")
            }
        }
    }

    /// The default value of `ty` (DDS-XTypes 1.3 7.2.2.4.4.4.7: zero, empty,
    /// the first enumerator, a nested type's default), the value a member
    /// takes when the data has none for it (type evolution, 7.4.1.2.3).
    fn default_expr(&self, ty: &IdlType) -> String {
        match self.kind(ty) {
            Kind::Prim(p) => match p {
                PrimitiveType::Boolean => "false".into(),
                PrimitiveType::Char | PrimitiveType::WChar => "'\\0'".into(),
                PrimitiveType::Float | PrimitiveType::Double | PrimitiveType::LongDouble => {
                    "0.0".into()
                }
                PrimitiveType::Fixed { digits, scale } => {
                    format!("Fixed::<{digits}, {scale}>::from_raw(0)")
                }
                PrimitiveType::String | PrimitiveType::WString => {
                    "::std::string::String::new()".into()
                }
                PrimitiveType::Void => "()".into(),
                _ => "0".into(),
            },
            Kind::Str { .. } => "::std::string::String::new()".into(),
            Kind::Seq { .. } => "::std::vec::Vec::new()".into(),
            Kind::Map { .. } => "::std::collections::HashMap::new()".into(),
            Kind::Array { elem, dims } => {
                let mut v = self.default_expr(&elem);
                for _ in &dims {
                    v = format!("::core::array::from_fn(|_| {v})");
                }
                v
            }
            Kind::Aggregate(n) => format!("{}::__hdds_default()", rust_path(&n)),
            Kind::Enum { name, .. } | Kind::Bitset(name) => {
                format!(
                    "<{} as ::core::default::Default>::default()",
                    rust_path(&name)
                )
            }
            Kind::Bitmask { .. } => "0".into(),
            Kind::Unknown(n) => {
                format!("compile_error!(\"hddsgen: type `{n}` is not defined in this IDL file\")")
            }
        }
    }

    /// For a type whose value can take no byte on the wire under some
    /// encoding (a FINAL or APPENDABLE structure, no DHEADER under XCDR1,
    /// whose members all can; an array of such elements or with a 0
    /// dimension), the number of values decoding one reads, nested ones
    /// included (saturating); 0 for any other type. Such a count in a
    /// collection is not bounded by the bytes left. Computed once per
    /// named type; a type that reaches itself counts as taking bytes.
    fn zero_wire(&self, ty: &IdlType, depth: usize) -> u64 {
        match self.kind(ty) {
            Kind::Array { elem, dims } => {
                if dims.contains(&0) {
                    return 1;
                }
                let w = self.zero_wire(&elem, depth);
                dims.iter()
                    .fold(w, |acc, d| acc.saturating_mul(u64::from(*d)))
            }
            Kind::Aggregate(n) => {
                if let Some(w) = self.zero_wire_of.borrow().get(&n) {
                    return *w;
                }
                // Provisional entry: a cycle through `n` takes bytes.
                self.zero_wire_of.borrow_mut().insert(n.clone(), 0);
                let w = self.struct_zero_wire(&n, depth);
                self.zero_wire_of.borrow_mut().insert(n, w);
                w
            }
            _ => 0,
        }
    }

    fn struct_zero_wire(&self, name: &str, depth: usize) -> u64 {
        let Some(s) = self.idx.struct_def(name) else {
            return 0;
        };
        if data_model::struct_ext(s) == Ext::Mutable {
            return 0;
        }
        let Ok(chain) = data_model::member_chain(self.idx, s) else {
            return 0;
        };
        let mut total: u64 = 1;
        for m in &chain {
            let w = if m.field.is_optional() {
                0
            } else {
                self.zero_wire(&m.field.field_type, depth + 1)
            };
            if w == 0 {
                return 0;
            }
            total = total.saturating_add(w);
        }
        total
    }

    /// `[T; N]...` read element by element (row-major), any element type.
    fn dec_array_dims(&mut self, elem: &IdlType, dims: &[u32]) -> String {
        let Some((&first, rest)) = dims.split_first() else {
            return self.dec(elem);
        };
        let inner = self.dec_array_dims(elem, rest);
        let v = self.fresh("v");
        format!(
            "{{ let mut {v} = ::std::vec::Vec::with_capacity({first}); for _ in 0..{first} {{ {v}.push({inner}); }} \
             <[_; {first}]>::try_from({v}).map_err(|_| ::hdds::CdrError::InvalidEncoding)? }}"
        )
    }
}

/// A place used as a whole value: `(*x)` becomes `*x` (no `unused_parens`
/// in the generated code).
fn as_value(place: &str) -> String {
    place
        .strip_prefix('(')
        .and_then(|r| r.strip_suffix(')'))
        .filter(|inner| inner.starts_with('*') && !inner.contains(['(', ')']))
        .map_or_else(|| place.to_string(), str::to_string)
}

/// The method call that replaces a missing value by `default`, in the
/// form clippy asks for: a constant is passed as is, a no-argument
/// constructor as a function path, a `Default` value through
/// `unwrap_or_default`; anything else stays lazy.
fn unwrap_or(default: &str) -> String {
    const CHEAP: [&str; 5] = ["false", "0", "0.0", "'\\0'", "()"];
    const DEFAULTS: [&str; 3] = [
        "::std::string::String::new()",
        "::std::vec::Vec::new()",
        "::std::collections::HashMap::new()",
    ];
    if CHEAP.contains(&default) {
        return format!(".unwrap_or({default})");
    }
    if DEFAULTS.contains(&default)
        || (default.starts_with('<')
            && default.ends_with(" as ::core::default::Default>::default()"))
    {
        return ".unwrap_or_default()".to_string();
    }
    match default.strip_suffix("()") {
        Some(path)
            if !path.is_empty()
                && path
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':') =>
        {
            format!(".unwrap_or_else({path})")
        }
        _ => format!(".unwrap_or_else(|| {default})"),
    }
}

/// `&place` for a borrowed argument, without the reborrow clippy flags:
/// `&(*x)` on a reference binding `x` is `x`, `&(**x)` through a boxed
/// binding is `x` (deref coercion), `&(*self.f)` on a boxed field is
/// `&self.f`.
fn borrow_of(place: &str) -> String {
    let inner = place.strip_prefix('(').and_then(|r| r.strip_suffix(')'));
    let Some(inner) = inner.and_then(|i| i.strip_prefix('*')) else {
        return format!("&{place}");
    };
    let path = inner.strip_prefix('*').unwrap_or(inner);
    let plain = !path.is_empty()
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
    if !plain {
        return format!("&{place}");
    }
    if path.starts_with("__") && !path.contains('.') {
        path.to_string()
    } else if inner.starts_with('*') {
        format!("&{place}")
    } else {
        format!("&{path}")
    }
}

const fn uint(width: usize) -> &'static str {
    match width {
        1 => "u8",
        2 => "u16",
        4 => "u32",
        _ => "u64",
    }
}

fn dec_prim(p: PrimitiveType) -> String {
    match p {
        PrimitiveType::Boolean => "__d.boolean()?".into(),
        PrimitiveType::Octet | PrimitiveType::UInt8 => "__d.raw::<1>()?[0]".into(),
        PrimitiveType::Int8 => "i8::from_le_bytes(__d.raw::<1>()?)".into(),
        PrimitiveType::Char => "__d.char8()?".into(),
        PrimitiveType::WChar => "__d.wchar()?".into(),
        PrimitiveType::Short | PrimitiveType::Int16 => "i16::from_le_bytes(__d.get::<2>()?)".into(),
        PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => {
            "u16::from_le_bytes(__d.get::<2>()?)".into()
        }
        PrimitiveType::Long | PrimitiveType::Int32 => "i32::from_le_bytes(__d.get::<4>()?)".into(),
        PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => {
            "u32::from_le_bytes(__d.get::<4>()?)".into()
        }
        PrimitiveType::LongLong | PrimitiveType::Int64 => {
            "i64::from_le_bytes(__d.get::<8>()?)".into()
        }
        PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => {
            "u64::from_le_bytes(__d.get::<8>()?)".into()
        }
        PrimitiveType::Float => "f32::from_le_bytes(__d.get::<4>()?)".into(),
        PrimitiveType::Double | PrimitiveType::LongDouble => {
            "f64::from_le_bytes(__d.get::<8>()?)".into()
        }
        PrimitiveType::Fixed { digits, scale } => format!(
            "{{ __d.align(4)?; Fixed::<{digits}, {scale}>::from_raw(i128::from_le_bytes(__d.raw::<16>()?)) }}"
        ),
        PrimitiveType::String => "__d.string(0)?".into(),
        PrimitiveType::WString => "__d.wstring(0)?".into(),
        PrimitiveType::Void => "()".into(),
    }
}

/// Rust path of an IDL scoped name (`A::B` stays `A::B`).
fn rust_path(name: &str) -> String {
    RustGenerator::type_to_rust(&IdlType::Named(name.to_string()))
}

/// The public entry points of a structure or union, all routed to its
/// hidden `__hdds_enc` / `__hdds_dec`.
fn entry_points() -> String {
    let mut out = String::new();
    for (suffix, v2) in [("xcdr1", false), ("xcdr2", true)] {
        let _ = write!(
            out,
            "    pub fn encode_{suffix}_le(&self, dst: &mut [u8]) -> Result<usize, CdrError> {{\n\
             \x20       let mut __e = {CDR_MOD}::Enc::new(dst, 0, {v2});\n\
             \x20       self.__hdds_enc(&mut __e)?;\n\
             \x20       Ok(__e.pos)\n\
             \x20   }}\n\n\
             \x20   pub fn encode_{suffix}_le_at(&self, dst: &mut [u8], offset: &mut usize) -> Result<(), CdrError> {{\n\
             \x20       let mut __e = {CDR_MOD}::Enc::new(dst, *offset, {v2});\n\
             \x20       self.__hdds_enc(&mut __e)?;\n\
             \x20       *offset = __e.pos;\n\
             \x20       Ok(())\n\
             \x20   }}\n\n\
             \x20   /// Exact serialized size of this value, or the error that\n\
             \x20   /// makes it unencodable (a bound exceeded, a size overflow).\n\
             \x20   pub fn try_{suffix}_size(&self) -> Result<usize, CdrError> {{\n\
             \x20       let mut __e = {CDR_MOD}::Enc::measure({v2});\n\
             \x20       self.__hdds_enc(&mut __e)?;\n\
             \x20       Ok(__e.pos)\n\
             \x20   }}\n\n\
             \x20   /// Exact serialized size of this value. For a value that\n\
             \x20   /// cannot be encoded, the size up to the failure:\n\
             \x20   /// encoding into that many bytes fails with the error\n\
             \x20   /// `try_{suffix}_size` reports.\n\
             \x20   pub fn max_{suffix}_size(&self) -> usize {{\n\
             \x20       let mut __e = {CDR_MOD}::Enc::measure({v2});\n\
             \x20       match self.__hdds_enc(&mut __e) {{\n\
             \x20           Ok(()) | Err(_) => __e.pos,\n\
             \x20       }}\n\
             \x20   }}\n\n\
             \x20   pub fn decode_{suffix}_le(src: &[u8]) -> Result<(Self, usize), CdrError> {{\n\
             \x20       let mut __d = {CDR_MOD}::Dec::new(src, 0, {v2});\n\
             \x20       let v = Self::__hdds_dec(&mut __d)?;\n\
             \x20       Ok((v, __d.pos))\n\
             \x20   }}\n\n\
             \x20   pub fn decode_{suffix}_le_at(src: &[u8], offset: &mut usize) -> Result<Self, CdrError> {{\n\
             \x20       let mut __d = {CDR_MOD}::Dec::new(src, *offset, {v2});\n\
             \x20       let v = Self::__hdds_dec(&mut __d)?;\n\
             \x20       *offset = __d.pos;\n\
             \x20       Ok(v)\n\
             \x20   }}\n\n"
        );
    }
    out
}

/// Place of a chain member read from `self`.
fn member_place(m: &data_model::ChainMember) -> String {
    let mut place = String::from("self");
    for _ in 0..m.base_depth {
        place.push_str(".base");
    }
    let _ = write!(place, ".{}", rust_ident(&m.field.name));
    if m.field.is_external() && !m.field.is_optional() {
        format!("(*{place})")
    } else {
        place
    }
}

/// Codec of a structure: the public entry points and the hidden methods.
/// A structure whose base chain cannot be resolved gets a `compile_error!`
/// naming the reason.
#[allow(clippy::too_many_lines)]
pub(super) fn emit_struct_codec(idx: &TypeIndex, s: &Struct) -> String {
    let chain = match member_chain(idx, s) {
        Ok(c) => c,
        Err(reason) => {
            return format!(
                "compile_error!(\"hddsgen: {}: {}\");\n\n",
                s.name,
                reason.replace('"', "'")
            );
        }
    };
    let ext = data_model::struct_ext(s);
    let mut g = Gen::new(idx);

    // ---- encode ----
    g.line(1, "#[doc(hidden)]");
    g.line(
        1,
        &format!(
            "pub fn __hdds_enc(&self, __e: &mut {CDR_MOD}::Enc<'_>) -> Result<(), CdrError> {{"
        ),
    );
    match ext {
        Ext::Final | Ext::Appendable => {
            if ext == Ext::Appendable {
                g.line(
                    2,
                    "let __dh = if __e.xcdr2 { Some(__e.begin_len()?) } else { None };",
                );
            }
            for m in &chain {
                let place = member_place(m);
                if m.field.is_optional() {
                    // Rule 20 (XCDR2): a presence flag, then the value;
                    // rule 19 (XCDR1): the member as a parameter, empty
                    // when absent.
                    let inner = opt_inner(&m.field);
                    g.line(2, "if __e.xcdr2 {");
                    g.line(3, &format!("__e.raw(&[u8::from({place}.is_some())])?;"));
                    g.line(3, &format!("if let Some(__o) = &{place} {{"));
                    g.enc(4, inner, &m.field.field_type);
                    g.line(3, "}");
                    g.line(2, "} else {");
                    g.line(
                        3,
                        &format!(
                            "let __h = __e.pl1_begin({:#x}, {})?;",
                            m.id,
                            must_understand(&m.field)
                        ),
                    );
                    g.line(3, &format!("if let Some(__o) = &{place} {{"));
                    g.enc(4, inner, &m.field.field_type);
                    g.line(3, "}");
                    g.line(3, "__e.pl1_end(__h)?;");
                    g.line(2, "}");
                } else {
                    g.enc(2, &place, &m.field.field_type);
                }
            }
            if ext == Ext::Appendable {
                g.line(2, "if let Some(__dh) = __dh { __e.end_len(__dh)?; }");
            }
        }
        Ext::Mutable => {
            g.line(2, "if __e.xcdr2 {");
            g.line(3, "let __dh = __e.begin_len()?;");
            for m in &chain {
                emit_mutable_member_v2(&mut g, 3, m);
            }
            g.line(3, "__e.end_len(__dh)?;");
            g.line(2, "} else {");
            for m in &chain {
                emit_mutable_member_v1(&mut g, 3, m);
            }
            g.line(3, "__e.pl1_sentinel()?;");
            g.line(2, "}");
        }
    }
    g.line(2, "Ok(())");
    g.line(1, "}");
    g.line(0, "");

    // ---- decode ----
    g.line(1, "#[doc(hidden)]");
    g.line(
        1,
        &format!("pub fn __hdds_dec(__d: &mut {CDR_MOD}::Dec<'_>) -> Result<Self, CdrError> {{"),
    );
    let locals: Vec<String> = (0..chain.len()).map(|i| format!("__f{i}")).collect();
    match ext {
        Ext::Final | Ext::Appendable => {
            if ext == Ext::Appendable {
                g.line(
                    2,
                    "let __dh = if __d.xcdr2 { Some(__d.begin_len()?) } else { None };",
                );
            }
            for (m, local) in chain.iter().zip(&locals) {
                let value = wrap_external(&m.field, g.dec(&m.field.field_type));
                if ext == Ext::Appendable {
                    // A writer whose type has fewer members ends its DHEADER
                    // (XCDR2) or its data (XCDR1) before this one: default.
                    let default = if m.field.is_optional() {
                        "None".to_string()
                    } else {
                        wrap_external(&m.field, g.default_expr(&m.field.field_type))
                    };
                    // An optional member continues as `else if __d.xcdr2`
                    // below, so no block wraps a lone `let` and its name.
                    let tail = if m.field.is_optional() {
                        "else if __d.xcdr2 {"
                    } else {
                        "else {"
                    };
                    g.line(
                        2,
                        &format!(
                            "let {local} = if __dh.map_or(__d.remaining() == 0, |__e| __d.pos >= __e) {{ {default} }} {tail}"
                        ),
                    );
                } else if m.field.is_optional() {
                    g.line(2, &format!("let {local} = if __d.xcdr2 {{"));
                } else {
                    g.line(2, &format!("let {local} = {{"));
                }
                if m.field.is_optional() {
                    g.line(
                        3,
                        &format!("if __d.boolean()? {{ Some({value}) }} else {{ None }}"),
                    );
                    g.line(2, "} else {");
                    g.line(
                        3,
                        "let (_, _, __end, __prev) = __d.pl1_member()?.ok_or(::hdds::CdrError::InvalidEncoding)?;",
                    );
                    g.line(
                        3,
                        &format!(
                            "let __o = if __end > __d.pos {{ Some({value}) }} else {{ None }};"
                        ),
                    );
                    g.line(3, "__d.pl1_done(__end, __prev)?;");
                    g.line(3, "__o");
                    g.line(2, "};");
                } else {
                    g.line(2, &format!("{value} }};"));
                }
            }
            if ext == Ext::Appendable {
                g.line(2, "if let Some(__dh) = __dh { __d.skip_to(__dh)?; }");
            }
        }
        Ext::Mutable => {
            for (m, local) in chain.iter().zip(&locals) {
                let ty = RustGenerator::type_to_rust(&m.field.field_type);
                let ty = if m.field.is_external() {
                    format!("Box<{ty}>")
                } else {
                    ty
                };
                g.line(
                    2,
                    &format!("let mut {local}: ::core::option::Option<{ty}> = None;"),
                );
            }
            g.line(2, "if __d.xcdr2 {");
            g.line(3, "let __dh = __d.begin_len()?;");
            g.line(3, "while __d.pos < __dh {");
            g.line(4, "let (__mu, __id, __end) = __d.emheader(__dh)?;");
            g.line(4, "match __id {");
            for (m, local) in chain.iter().zip(&locals) {
                let value = wrap_external(&m.field, g.dec(&m.field.field_type));
                g.line(5, &format!("{:#x} => {{ {local} = Some({value}); }}", m.id));
            }
            g.line(
                5,
                "_ => { if __mu { return Err(::hdds::CdrError::InvalidEncoding); } }",
            );
            g.line(4, "}");
            g.line(4, "__d.skip_to(__end)?;");
            g.line(3, "}");
            g.line(3, "__d.skip_to(__dh)?;");
            g.line(2, "} else {");
            g.line(
                3,
                "while let Some((__id, __mu, __end, __prev)) = __d.pl1_member()? {",
            );
            g.line(4, "match __id {");
            for (m, local) in chain.iter().zip(&locals) {
                let value = wrap_external(&m.field, g.dec(&m.field.field_type));
                g.line(5, &format!("{:#x} => {{ {local} = Some({value}); }}", m.id));
            }
            g.line(
                5,
                "_ => { if __mu { return Err(::hdds::CdrError::InvalidEncoding); } }",
            );
            g.line(4, "}");
            g.line(4, "__d.pl1_done(__end, __prev)?;");
            g.line(3, "}");
            g.line(2, "}");
            // A member the writer's type does not have takes its default
            // (DDS-XTypes 1.3 7.4.1.2.3, spec text lines 6195-6203).
            for (m, local) in chain.iter().zip(&locals) {
                if !m.field.is_optional() {
                    let default = wrap_external(&m.field, g.default_expr(&m.field.field_type));
                    g.line(2, &format!("let {local} = {local}{};", unwrap_or(&default)));
                }
            }
        }
    }
    let literal = struct_literal(idx, s, &s.name, &chain, &locals, 0);
    g.line(2, &format!("Ok({literal})"));
    g.line(1, "}");
    g.line(0, "");

    // ---- default ----
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
                wrap_external(&m.field, g.default_expr(&m.field.field_type))
            }
        })
        .collect();
    let literal = struct_literal(idx, s, &s.name, &chain, &defaults, 0);
    g.line(2, &literal);
    g.line(1, "}");

    let body = g.out;
    let mut out = String::new();
    let _ = writeln!(out, "impl {} {{", s.name);
    out.push_str(&entry_points());
    out.push_str(&body);
    out.push_str("}\n\n");
    out
}

/// The place of the value of a present optional member bound as `__o`.
fn opt_inner(field: &Field) -> &'static str {
    if field.is_external() {
        "(**__o)"
    } else {
        "(*__o)"
    }
}

/// `Box::new(..)` around the value of an `@external` member.
fn wrap_external(field: &Field, value: String) -> String {
    if field.is_external() {
        format!("Box::new({value})")
    } else {
        value
    }
}

/// The literal of `s` (at `depth` base hops from the decoded type) from the
/// decoded locals; `@non_serialized` members take their default.
fn struct_literal(
    idx: &TypeIndex,
    s: &Struct,
    name: &str,
    chain: &[data_model::ChainMember],
    locals: &[String],
    depth: usize,
) -> String {
    let mut lit = format!("{} {{ ", rust_path(name));
    if let Some(base) = &s.base_struct {
        if let Some(b) = idx.struct_def(base) {
            let b = b.clone();
            let _ = write!(
                lit,
                "base: {}, ",
                struct_literal(idx, &b, base, chain, locals, depth + 1)
            );
        }
    }
    for field in &s.fields {
        let ident = rust_ident(&field.name);
        if field.is_non_serialized() {
            let _ = write!(lit, "{ident}: Default::default(), ");
            continue;
        }
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

/// One member of a MUTABLE structure under XCDR2: `EMHEADER1`, NEXTINT
/// when the length code asks for one, the value (rule 22).
fn emit_mutable_member_v2(g: &mut Gen<'_>, ind: usize, m: &data_model::ChainMember) {
    let place = member_place(m);
    let lc = length_code(g.idx, &m.field.field_type);
    let mu = must_understand(&m.field);
    let (open, value_place) = if m.field.is_optional() {
        (
            Some(format!("if let Some(__o) = &{place} {{")),
            opt_inner(&m.field).to_string(),
        )
    } else {
        (None, place)
    };
    let mut ind = ind;
    if let Some(open) = &open {
        g.line(ind, open);
        ind += 1;
    }
    g.line(ind, &format!("__e.emheader({mu}, {lc}, {:#x})?;", m.id));
    if lc == 4 {
        let n = g.fresh("n");
        g.line(ind, &format!("let {n} = __e.begin_len()?;"));
        g.enc(ind, &value_place, &m.field.field_type);
        g.line(ind, &format!("__e.end_len({n})?;"));
    } else {
        g.enc(ind, &value_place, &m.field.field_type);
    }
    if open.is_some() {
        g.line(ind - 1, "}");
    }
}

/// One member of a MUTABLE structure under XCDR1: a parameter (rule 25),
/// left out when an optional member is absent.
fn emit_mutable_member_v1(g: &mut Gen<'_>, ind: usize, m: &data_model::ChainMember) {
    let place = member_place(m);
    let (open, value_place) = if m.field.is_optional() {
        (
            Some(format!("if let Some(__o) = &{place} {{")),
            opt_inner(&m.field).to_string(),
        )
    } else {
        (None, place)
    };
    let mut ind = ind;
    if let Some(open) = &open {
        g.line(ind, open);
        ind += 1;
    }
    let h = g.fresh("h");
    g.line(
        ind,
        &format!(
            "let {h} = __e.pl1_begin({:#x}, {})?;",
            m.id,
            must_understand(&m.field)
        ),
    );
    g.enc(ind, &value_place, &m.field.field_type);
    g.line(ind, &format!("__e.pl1_end({h})?;"));
    if open.is_some() {
        g.line(ind - 1, "}");
    }
}

// ---------------------------------------------------------------------------
// Unions
// ---------------------------------------------------------------------------

/// The discriminator's Rust holder type and serialized width.
fn disc_holder(idx: &TypeIndex, ty: &IdlType) -> (&'static str, usize) {
    match idx.resolve(ty) {
        IdlType::Primitive(p) => match p {
            PrimitiveType::Boolean
            | PrimitiveType::Octet
            | PrimitiveType::UInt8
            | PrimitiveType::Char => ("u8", 1),
            PrimitiveType::Int8 => ("i8", 1),
            PrimitiveType::WChar | PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => {
                ("u16", 2)
            }
            PrimitiveType::Short | PrimitiveType::Int16 => ("i16", 2),
            PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => ("u32", 4),
            PrimitiveType::LongLong | PrimitiveType::Int64 => ("i64", 8),
            PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => ("u64", 8),
            _ => ("i32", 4),
        },
        IdlType::Named(n) => match idx.named(&n) {
            Some(Named::Enum { bit_bound, .. }) => match enum_width(*bit_bound) {
                1 => ("i8", 1),
                2 => ("i16", 2),
                _ => ("i32", 4),
            },
            _ => ("i32", 4),
        },
        _ => ("i32", 4),
    }
}

/// Codec of a union: the public entry points and the hidden methods.
/// FINAL and APPENDABLE (DHEADER under XCDR2) per rules 26, 29 and 30.
#[allow(clippy::too_many_lines)]
pub(super) fn emit_union_codec(idx: &TypeIndex, u: &Union) -> String {
    let ext = union_ext(u);
    if ext == Ext::Mutable {
        return emit_mutable_union_codec(idx, u);
    }
    let (holder, width) = disc_holder(idx, &u.discriminator);
    let mut g = Gen::new(idx);
    let mut problems = Vec::new();

    // ---- encode ----
    g.line(1, "#[doc(hidden)]");
    g.line(
        1,
        &format!(
            "pub fn __hdds_enc(&self, __e: &mut {CDR_MOD}::Enc<'_>) -> Result<(), CdrError> {{"
        ),
    );
    if ext == Ext::Appendable {
        g.line(
            2,
            "let __dh = if __e.xcdr2 { Some(__e.begin_len()?) } else { None };",
        );
    }
    g.line(2, "match self {");
    for (i, case) in u.cases.iter().enumerate() {
        let variant = RustGenerator::union_variant_name(&case.field.name, i);
        let Some(disc) = union_case_disc(idx, u, i) else {
            problems.push(format!(
                "the label of case `{}` is not a value hddsgen can compute",
                case.field.name
            ));
            continue;
        };
        let is_void = matches!(
            case.field.field_type,
            IdlType::Primitive(PrimitiveType::Void)
        );
        let binding = if is_void { "_" } else { "__v" };
        g.line(3, &format!("Self::{variant}({binding}) => {{"));
        g.line(
            4,
            &format!("__e.put(&({disc}i64 as {holder}).to_le_bytes())?;"),
        );
        if !is_void {
            g.enc(4, "(*__v)", &case.field.field_type);
        }
        g.line(3, "}");
    }
    g.line(2, "}");
    if ext == Ext::Appendable {
        g.line(2, "if let Some(__dh) = __dh { __e.end_len(__dh)?; }");
    }
    g.line(2, "Ok(())");
    g.line(1, "}");
    g.line(0, "");

    // ---- decode ----
    g.line(1, "#[doc(hidden)]");
    g.line(
        1,
        &format!("pub fn __hdds_dec(__d: &mut {CDR_MOD}::Dec<'_>) -> Result<Self, CdrError> {{"),
    );
    if ext == Ext::Appendable {
        g.line(
            2,
            "let __dh = if __d.xcdr2 { Some(__d.begin_len()?) } else { None };",
        );
    }
    g.line(
        2,
        &format!("let __disc = {holder}::from_le_bytes(__d.get::<{width}>()?) as i64;"),
    );
    g.line(2, "let __value = match __disc {");
    let mut default_arm = None;
    for (i, case) in u.cases.iter().enumerate() {
        let variant = RustGenerator::union_variant_name(&case.field.name, i);
        let is_void = matches!(
            case.field.field_type,
            IdlType::Primitive(PrimitiveType::Void)
        );
        let value = if is_void {
            "()".to_string()
        } else {
            g.dec(&case.field.field_type)
        };
        let arm = format!("Self::{variant}({value})");
        let mut pats = Vec::new();
        for label in &case.labels {
            match label {
                UnionLabel::Default => default_arm = Some(arm.clone()),
                UnionLabel::Value(v) => {
                    if let Some(n) = union_label_value(idx, &u.discriminator, v) {
                        pats.push(format!("{n}"));
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
        None => g.line(3, "_ => return Err(::hdds::CdrError::InvalidEncoding),"),
    }
    g.line(2, "};");
    if ext == Ext::Appendable {
        g.line(2, "if let Some(__dh) = __dh { __d.skip_to(__dh)?; }");
    }
    g.line(2, "Ok(__value)");
    g.line(1, "}");

    let mut out = String::new();
    for p in &problems {
        let _ = writeln!(
            out,
            "compile_error!(\"hddsgen: union {}: {}\");",
            u.name,
            p.replace('"', "'")
        );
    }
    let _ = writeln!(out, "impl {} {{", u.name);
    out.push_str(&entry_points());
    out.push_str(&g.out);
    out.push_str("}\n\n");
    out
}

/// Codec of a MUTABLE union (rules 27-28): under XCDR2 a DHEADER, the
/// discriminator as member 0 (`EMHEADER1` with the must-understand flag,
/// 7.2.2.4.4.4.6: the discriminator always must be understood), then the
/// selected case as a member with its id; under XCDR1 the same two
/// members as parameters, closed by `PID_LIST_END`. The case ids are
/// `data_model::union_case_ids` (from 1, DDS-XTypes 1.3 7.3.1.2.1.1).
#[allow(clippy::too_many_lines)]
fn emit_mutable_union_codec(idx: &TypeIndex, u: &Union) -> String {
    let (holder, width) = disc_holder(idx, &u.discriminator);
    let disc_lc = data_model::discriminator_length_code(idx, &u.discriminator);
    let ids = data_model::union_case_ids(idx, u);
    let mut g = Gen::new(idx);
    let mut problems = Vec::new();

    // ---- encode ----
    g.line(1, "#[doc(hidden)]");
    g.line(
        1,
        &format!(
            "pub fn __hdds_enc(&self, __e: &mut {CDR_MOD}::Enc<'_>) -> Result<(), CdrError> {{"
        ),
    );
    g.line(
        2,
        "let __dh = if __e.xcdr2 { Some(__e.begin_len()?) } else { None };",
    );
    g.line(2, "match self {");
    for (i, case) in u.cases.iter().enumerate() {
        let variant = RustGenerator::union_variant_name(&case.field.name, i);
        let Some(disc) = union_case_disc(idx, u, i) else {
            problems.push(format!(
                "the label of case `{}` is not a value hddsgen can compute",
                case.field.name
            ));
            continue;
        };
        let is_void = matches!(
            case.field.field_type,
            IdlType::Primitive(PrimitiveType::Void)
        );
        let id = ids[i];
        let mu = must_understand(&case.field);
        let lc = length_code(idx, &case.field.field_type);
        g.line(
            3,
            &format!(
                "Self::{variant}({}) => {{",
                if is_void { "_" } else { "__v" }
            ),
        );
        g.line(4, "if __e.xcdr2 {");
        g.line(5, &format!("__e.emheader(true, {disc_lc}, 0)?;"));
        g.line(
            5,
            &format!("__e.put(&({disc}i64 as {holder}).to_le_bytes())?;"),
        );
        if !is_void {
            g.line(5, &format!("__e.emheader({mu}, {lc}, {id:#x})?;"));
            if lc == 4 {
                g.line(5, "let __n = __e.begin_len()?;");
                g.enc(5, "(*__v)", &case.field.field_type);
                g.line(5, "__e.end_len(__n)?;");
            } else {
                g.enc(5, "(*__v)", &case.field.field_type);
            }
        }
        g.line(4, "} else {");
        g.line(5, "let __h = __e.pl1_begin(0, true)?;");
        g.line(
            5,
            &format!("__e.put(&({disc}i64 as {holder}).to_le_bytes())?;"),
        );
        g.line(5, "__e.pl1_end(__h)?;");
        if !is_void {
            g.line(5, &format!("let __h = __e.pl1_begin({id:#x}, {mu})?;"));
            g.enc(5, "(*__v)", &case.field.field_type);
            g.line(5, "__e.pl1_end(__h)?;");
        }
        g.line(5, "__e.pl1_sentinel()?;");
        g.line(4, "}");
        g.line(3, "}");
    }
    g.line(2, "}");
    g.line(2, "if let Some(__dh) = __dh { __e.end_len(__dh)?; }");
    g.line(2, "Ok(())");
    g.line(1, "}");
    g.line(0, "");

    // ---- decode ----
    g.line(1, "#[doc(hidden)]");
    g.line(
        1,
        &format!("pub fn __hdds_dec(__d: &mut {CDR_MOD}::Dec<'_>) -> Result<Self, CdrError> {{"),
    );
    g.line(2, "let mut __disc: Option<i64> = None;");
    g.line(2, "let mut __value: Option<Self> = None;");
    // The case member: decoded by id, then checked against the
    // discriminator once both are read.
    let mut arms = String::new();
    for (i, case) in u.cases.iter().enumerate() {
        let variant = RustGenerator::union_variant_name(&case.field.name, i);
        if matches!(
            case.field.field_type,
            IdlType::Primitive(PrimitiveType::Void)
        ) {
            continue;
        }
        let value = g.dec(&case.field.field_type);
        let _ = writeln!(
            arms,
            "{:#x} => {{ __value = Some(Self::{variant}({value})); }}",
            ids[i]
        );
    }
    let disc_read = format!("{holder}::from_le_bytes(__d.get::<{width}>()?) as i64");
    g.line(2, "if __d.xcdr2 {");
    g.line(3, "let __dh = __d.begin_len()?;");
    g.line(3, "while __d.pos < __dh {");
    g.line(4, "let (__mu, __id, __end) = __d.emheader(__dh)?;");
    g.line(4, "match __id {");
    g.line(5, &format!("0 => {{ __disc = Some({disc_read}); }}"));
    for l in arms.lines() {
        g.line(5, l);
    }
    g.line(
        5,
        "_ => { if __mu { return Err(::hdds::CdrError::InvalidEncoding); } }",
    );
    g.line(4, "}");
    g.line(4, "__d.skip_to(__end)?;");
    g.line(3, "}");
    g.line(3, "__d.skip_to(__dh)?;");
    g.line(2, "} else {");
    g.line(
        3,
        "while let Some((__id, __mu, __end, __prev)) = __d.pl1_member()? {",
    );
    g.line(4, "match __id {");
    g.line(5, &format!("0 => {{ __disc = Some({disc_read}); }}"));
    for l in arms.lines() {
        g.line(5, l);
    }
    g.line(
        5,
        "_ => { if __mu { return Err(::hdds::CdrError::InvalidEncoding); } }",
    );
    g.line(4, "}");
    g.line(4, "__d.pl1_done(__end, __prev)?;");
    g.line(3, "}");
    g.line(2, "}");
    // A void case has no member: the discriminator selects it alone.
    g.line(
        2,
        "let __disc = __disc.ok_or(::hdds::CdrError::InvalidEncoding)?;",
    );
    g.line(2, "match (__disc, __value) {");
    let mut default_variant = None;
    for (i, case) in u.cases.iter().enumerate() {
        let variant = RustGenerator::union_variant_name(&case.field.name, i);
        let is_void = matches!(
            case.field.field_type,
            IdlType::Primitive(PrimitiveType::Void)
        );
        let mut pats = Vec::new();
        for label in &case.labels {
            match label {
                UnionLabel::Default => default_variant = Some((variant.clone(), is_void)),
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
            g.line(3, &format!("({pat}, None) => Ok(Self::{variant}(())),"));
        } else {
            g.line(
                3,
                &format!("({pat}, Some(__v @ Self::{variant}(_))) => Ok(__v),"),
            );
        }
    }
    if let Some((variant, is_void)) = default_variant {
        if is_void {
            g.line(3, &format!("(_, None) => Ok(Self::{variant}(())),"));
        } else {
            g.line(
                3,
                &format!("(_, Some(__v @ Self::{variant}(_))) => Ok(__v),"),
            );
        }
    }
    g.line(3, "_ => Err(::hdds::CdrError::InvalidEncoding),");
    g.line(2, "}");
    g.line(1, "}");

    let mut out = String::new();
    for p in &problems {
        let _ = writeln!(
            out,
            "compile_error!(\"hddsgen: union {}: {}\");",
            u.name,
            p.replace('"', "'")
        );
    }
    let _ = writeln!(out, "impl {} {{", u.name);
    out.push_str(&entry_points());
    out.push_str(&g.out);
    out.push_str("}\n\n");
    out
}

/// `__hdds_default()` of a union: the case the discriminator's default
/// value selects (0, FALSE or the first enumerator, DDS-XTypes 1.3
/// 7.2.2.4.4.4.7), else the `default` case, with its value's default.
/// When that value selects no case the spec's union holds the
/// discriminator alone; the Rust enum has no such state and takes its
/// first case instead.
pub(super) fn emit_union_default(idx: &TypeIndex, u: &Union) -> String {
    let g = Gen::new(idx);
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
    let mut out = String::new();
    let _ = writeln!(out, "impl {} {{", u.name);
    out.push_str("    /// The default value of this union (DDS-XTypes 1.3 7.2.2.4.4.4.7).\n");
    out.push_str(
        "    #[doc(hidden)]\n    #[must_use]\n    #[allow(unreachable_code, clippy::all)]\n",
    );
    out.push_str("    pub fn __hdds_default() -> Self {\n");
    match u.cases.get(pick) {
        Some(case) => {
            let variant = RustGenerator::union_variant_name(&case.field.name, pick);
            let value = if matches!(
                case.field.field_type,
                IdlType::Primitive(PrimitiveType::Void)
            ) {
                "()".to_string()
            } else {
                g.default_expr(&case.field.field_type)
            };
            let _ = writeln!(out, "        Self::{variant}({value})");
        }
        None => out.push_str("        compile_error!(\"hddsgen: a union without cases\")\n"),
    }
    out.push_str("    }\n}\n\n");
    out
}

/// The standalone enum decode: the discriminant read as an `i32` and the
/// too-short-input test. `i32::from` only widens an `i8` / `i16`
/// discriminant (on an `i32` it would be an identity conversion), and an
/// input of one byte is tested with `is_empty`.
fn enum_decode_parts(int: &str, width: usize) -> (String, String) {
    let wide = if int == "i32" {
        format!("{int}::from_le_bytes(b)")
    } else {
        format!("i32::from({int}::from_le_bytes(b))")
    };
    let short = if width == 1 {
        "src.is_empty()".to_string()
    } else {
        format!("src.len() < {width}")
    };
    (wide, short)
}

/// Codec of an enumeration: `__hdds_from_i32` and the standalone entry
/// points (the value alone, as an integer of the `@bit_bound` width).
pub(super) fn emit_enum_codec(e: &Enum, indent: &str) -> String {
    let width = enum_width(e.annotations.iter().find_map(|a| match a {
        crate::types::Annotation::BitBound(n) => Some(*n),
        _ => None,
    }));
    let int = match width {
        1 => "i8",
        2 => "i16",
        _ => "i32",
    };
    let (wide, short) = enum_decode_parts(int, width);
    let mut out = String::new();
    let _ = writeln!(out, "{indent}impl {} {{", e.name);
    let _ = writeln!(out, "{indent}    #[doc(hidden)]");
    let _ = writeln!(
        out,
        "{indent}    pub fn __hdds_from_i32(v: i32) -> Result<Self, CdrError> {{"
    );
    let _ = writeln!(out, "{indent}        match v {{");
    for (variant, value) in e.variants.iter().zip(e.resolved_values()) {
        let _ = writeln!(
            out,
            "{indent}            {value} => Ok(Self::{}),",
            variant.name
        );
    }
    let _ = writeln!(
        out,
        "{indent}            _ => Err(CdrError::InvalidEncoding),"
    );
    let _ = writeln!(out, "{indent}        }}");
    let _ = writeln!(out, "{indent}    }}");
    for suffix in ["xcdr1", "xcdr2"] {
        let _ = write!(
            out,
            "{indent}    pub fn encode_{suffix}_le(&self, dst: &mut [u8]) -> Result<usize, CdrError> {{\n\
             {indent}        let b = ((*self as i64) as {int}).to_le_bytes();\n\
             {indent}        if dst.len() < b.len() {{ return Err(CdrError::BufferTooSmall); }}\n\
             {indent}        dst[..b.len()].copy_from_slice(&b);\n\
             {indent}        Ok(b.len())\n\
             {indent}    }}\n\
             {indent}    pub fn max_{suffix}_size(&self) -> usize {{ {width} }}\n\
             {indent}    pub fn decode_{suffix}_le(src: &[u8]) -> Result<(Self, usize), CdrError> {{\n\
             {indent}        if {short} {{ return Err(CdrError::UnexpectedEof); }}\n\
             {indent}        let mut b = [0u8; {width}];\n\
             {indent}        b.copy_from_slice(&src[..{width}]);\n\
             {indent}        Ok((Self::__hdds_from_i32({wide})?, {width}))\n\
             {indent}    }}\n"
        );
        out.push_str(&RustGenerator::emit_encode_at_wrapper(suffix, indent));
        out.push_str(&RustGenerator::emit_decode_at_wrapper(suffix, indent));
    }
    let _ = writeln!(out, "{indent}}}\n");
    out
}
