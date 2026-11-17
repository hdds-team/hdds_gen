// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Least XCDR2 (and XCDR1) size of a value, shared by the decoders that
//! bound a sequence or map count before building its elements (C, C++,
//! Python, TypeScript).
//!
//! The least sizes follow DDS-XTypes 1.3 7.4.3.5.3 (spec text lines
//! 6750-7015): a primitive takes its size, a string or a sequence at least
//! its length word, an APPENDABLE or MUTABLE structure its DHEADER, a FINAL
//! structure the sum of its members (one presence byte for an optional
//! one), a FINAL union its discriminator (at least one byte).
//!
//! Both walks have no depth limit: the figure of a structure or union is
//! computed once per walk and remembered (a type used n times, or a chain
//! of n types each holding two of the one below, costs n visits, not
//! 2^n), and a type reached again while its own figure is being computed
//! (only an ill-formed type contains itself without a sequence, a map or
//! an optional member, which stop the descent) counts as nothing promised.

use std::collections::HashMap;

use crate::codegen::data_model::{
    self, collection_has_dheader, flatten_array, member_chain, union_ext, Ext,
};
use crate::codegen::key_holder::{Named, TypeIndex};
use crate::types::{IdlType, PrimitiveType};

/// The figure of each structure or union of one walk, keyed by the address
/// of its definition in the index: `None` while it is being computed.
type Memo = HashMap<usize, Option<u64>>;

/// The key of a definition in a [`Memo`].
fn key(named: &Named) -> usize {
    std::ptr::from_ref(named) as usize
}

/// A lower bound of the XCDR2 size of any value of `ty`, padding aside
/// (padding only adds bytes): what a decoder can demand of the bytes left
/// for each element of a sequence or map before it builds them. 0 when
/// nothing can be promised (an empty FINAL structure, an unknown type, a
/// type that contains itself): such elements consume no byte, so the bytes
/// left cannot bound their count.
#[must_use]
pub fn min_wire(idx: &TypeIndex, ty: &IdlType) -> u64 {
    min_wire_in(idx, ty, &mut Memo::new(), false)
}

/// [`min_wire`] under XCDR1 (DDS-XTypes 1.3 7.4.1.2): no DHEADER (a
/// collection takes its count, an APPENDABLE structure or union the FINAL
/// form), an optional member of a FINAL or APPENDABLE structure at least a
/// short parameter header (4 bytes, 7.4.1.2.2), a MUTABLE structure or
/// union at least its `PID_LIST_END` (4). An APPENDABLE structure counts
/// all its members: under XCDR1 nothing delimits it, so only the end of
/// the data can end it early, and an element of a collection is followed
/// by the next one.
#[must_use]
pub fn min_wire_xcdr1(idx: &TypeIndex, ty: &IdlType) -> u64 {
    min_wire_in(idx, ty, &mut Memo::new(), true)
}

fn min_wire_in(idx: &TypeIndex, ty: &IdlType, memo: &mut Memo, v1: bool) -> u64 {
    let resolved = idx.resolve(ty);
    match resolved {
        // A length word (an empty string is accepted with length 0).
        IdlType::BoundedString { .. }
        | IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString) => 4,
        IdlType::Primitive(PrimitiveType::Fixed { .. }) => 16,
        IdlType::Primitive(p) => data_model::primitive_size(&p).map_or(0, |w| w as u64),
        IdlType::Sequence { .. } | IdlType::Map { .. } => {
            if !v1 && collection_has_dheader(idx, ty) {
                8
            } else {
                4
            }
        }
        IdlType::Array { .. } => {
            let (elem, dims) =
                flatten_array(idx, &resolved).unwrap_or_else(|| (resolved.clone(), vec![]));
            let n = dims
                .iter()
                .fold(1u64, |acc, d| acc.saturating_mul(u64::from(*d)));
            let dh = if !v1 && collection_has_dheader(idx, ty) {
                4
            } else {
                0
            };
            n.saturating_mul(min_wire_in(idx, &elem, memo, v1))
                .saturating_add(dh)
        }
        IdlType::Named(n) => match idx.named(&n) {
            Some(named @ (Named::Struct(_) | Named::Union(_))) => {
                let k = key(named);
                match memo.get(&k) {
                    Some(Some(v)) => return *v,
                    Some(None) => return 0,
                    None => {}
                }
                memo.insert(k, None);
                let v = aggregate(idx, named, memo, v1);
                memo.insert(k, Some(v));
                v
            }
            Some(Named::Enum { bit_bound, .. }) => data_model::enum_width(*bit_bound) as u64,
            Some(Named::Bitmask { bit_bound }) => data_model::bitmask_width(*bit_bound) as u64,
            Some(Named::Bitset) => 8,
            Some(Named::Typedef(_)) | None => 0,
        },
    }
}

fn aggregate(idx: &TypeIndex, named: &Named, memo: &mut Memo, v1: bool) -> u64 {
    match named {
        Named::Struct(s) => {
            let ext = data_model::struct_ext(s);
            if ext == Ext::Mutable || (!v1 && ext == Ext::Appendable) {
                // DHEADER (XCDR2) or PID_LIST_END (XCDR1); the members may
                // be absent (7.4.3.5.3, 7.4.1.2.2).
                return 4;
            }
            member_chain(idx, s).map_or(0, |chain| {
                chain.iter().fold(0u64, |acc, m| {
                    let w = if m.field.is_optional() {
                        if v1 {
                            4
                        } else {
                            1
                        }
                    } else {
                        min_wire_in(idx, &m.field.field_type, memo, v1)
                    };
                    acc.saturating_add(w)
                })
            })
        }
        // A discriminator of at least one byte; the selected member may be
        // none.
        Named::Union(u) => match (union_ext(u), v1) {
            (Ext::Mutable, _) | (Ext::Appendable, false) => 4,
            _ => 1,
        },
        _ => 0,
    }
}

/// An upper bound of the number of values a decoder builds for one value
/// of `ty`, whatever its wire form: what the C and C++ decoders bound
/// through `sizeof`, for the decoders of languages without one (Python,
/// TypeScript), which build an absent member's default (a whole array)
/// from a few bytes. A structure counts its members (at least 1), whatever
/// its extensibility; a union its largest branch plus its discriminator;
/// an array its elements; a string, sequence or map 1 (their contents are
/// bounded by their own counts); anything else 1. A type that contains
/// itself counts 1 where it recurs.
#[must_use]
pub fn value_weight(idx: &TypeIndex, ty: &IdlType) -> u64 {
    weight_in(idx, ty, &mut Memo::new())
}

fn weight_in(idx: &TypeIndex, ty: &IdlType, memo: &mut Memo) -> u64 {
    let resolved = idx.resolve(ty);
    match resolved {
        IdlType::Array { .. } => {
            let (elem, dims) =
                flatten_array(idx, &resolved).unwrap_or_else(|| (resolved.clone(), vec![]));
            dims.iter()
                .fold(1u64, |acc, d| acc.saturating_mul(u64::from(*d)))
                .saturating_mul(weight_in(idx, &elem, memo))
        }
        IdlType::Named(n) => {
            let Some(named @ (Named::Struct(_) | Named::Union(_))) = idx.named(&n) else {
                return 1;
            };
            let k = key(named);
            match memo.get(&k) {
                Some(Some(v)) => return *v,
                Some(None) => return 1,
                None => {}
            }
            memo.insert(k, None);
            let v = match named {
                Named::Struct(s) => member_chain(idx, s).map_or(1, |chain| {
                    chain
                        .iter()
                        .fold(0u64, |acc, m| {
                            acc.saturating_add(weight_in(idx, &m.field.field_type, memo))
                        })
                        .max(1)
                }),
                Named::Union(u) => u
                    .cases
                    .iter()
                    .map(|c| weight_in(idx, &c.field.field_type, memo))
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1),
                _ => 1,
            };
            memo.insert(k, Some(v));
            v
        }
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::parser::Parser;
    use std::fmt::Write as _;

    fn least(idl: &str, ty: &str) -> u64 {
        let mut p = Parser::try_new(idl).expect("lexer");
        let ast = p.parse().expect("parse");
        let idx = TypeIndex::new(&ast);
        min_wire(&idx, &IdlType::Named(ty.to_string()))
    }

    fn least1(idl: &str, ty: &str) -> u64 {
        let mut p = Parser::try_new(idl).expect("lexer");
        let ast = p.parse().expect("parse");
        let idx = TypeIndex::new(&ast);
        min_wire_xcdr1(&idx, &IdlType::Named(ty.to_string()))
    }

    fn weight(idl: &str, ty: &str) -> u64 {
        let mut p = Parser::try_new(idl).expect("lexer");
        let ast = p.parse().expect("parse");
        let idx = TypeIndex::new(&ast);
        value_weight(&idx, &IdlType::Named(ty.to_string()))
    }

    #[test]
    fn least_sizes_of_the_wire_forms() {
        let idl = "
            @final struct Empty {};
            @final struct Two { Empty a; Empty b; };
            @final struct Opt { @optional long a[16]; octet b; };
            @appendable struct App { long a[16]; };
            @final union U switch (long) { case 1: long a; };
            @mutable union M switch (long) { case 1: long a; };
            @final struct Arr { Empty e[8]; };
            @final struct Prim { long a[3]; string s; sequence<long> q; sequence<Empty> qe; };
        ";
        assert_eq!(least(idl, "Empty"), 0);
        assert_eq!(least(idl, "Two"), 0);
        assert_eq!(least(idl, "Opt"), 2);
        assert_eq!(least(idl, "App"), 4);
        assert_eq!(least(idl, "U"), 1);
        assert_eq!(least(idl, "M"), 4);
        // An array of structures carries a DHEADER (7.4.3.5.3 rule 9, spec text line 6809).
        assert_eq!(least(idl, "Arr"), 4);
        // 12 + 4 + 4 + (DHEADER + count) 8.
        assert_eq!(least(idl, "Prim"), 28);
    }

    #[test]
    fn least_xcdr1_sizes_have_no_dheader() {
        let idl = "
            @final struct Empty {};
            @appendable struct AppEmpty {};
            @final struct Opt { @optional long a[16]; octet b; };
            @appendable struct App { long a[16]; octet b; };
            @mutable struct Mut { long a; };
            @final union U switch (long) { case 1: long a; };
            @appendable union UA switch (long) { case 1: long a; };
            @mutable union M switch (long) { case 1: long a; };
            @final struct Arr { App e[2]; };
            @final struct Prim { long a[3]; string s; sequence<long> q; sequence<App> qa; };
        ";
        assert_eq!(least1(idl, "Empty"), 0);
        assert_eq!(least1(idl, "AppEmpty"), 0);
        // A short parameter header for the optional member, then the octet.
        assert_eq!(least1(idl, "Opt"), 5);
        assert_eq!(least1(idl, "App"), 65);
        assert_eq!(least1(idl, "Mut"), 4);
        assert_eq!(least1(idl, "U"), 1);
        assert_eq!(least1(idl, "UA"), 1);
        assert_eq!(least1(idl, "M"), 4);
        assert_eq!(least1(idl, "Arr"), 130);
        // 12 + 4 + 4 + 4, no DHEADER.
        assert_eq!(least1(idl, "Prim"), 24);
        // XCDR2 promises the DHEADER alone.
        assert_eq!(least(idl, "AppEmpty"), 4);
    }

    #[test]
    fn value_weights_count_the_values_built() {
        let mut idl = String::from("@final struct Empty {};\n");
        idl.push_str("@final struct E1 { Empty a; Empty b; };\n");
        for i in 2..=12 {
            let _ = writeln!(idl, "@final struct E{i} {{ E{} a; E{} b; }};", i - 1, i - 1);
        }
        idl.push_str(
            "@final struct Arr { long a[3][4]; };\n@appendable struct App { long a[8]; };\n",
        );
        idl.push_str("@final union U switch (long) { case 1: long a; case 2: long b[16]; };\n");
        idl.push_str("@mutable struct Q { sequence<long> q; string s; };\n");
        assert_eq!(least(&idl, "Empty"), 0);
        assert_eq!(weight(&idl, "Empty"), 1);
        assert_eq!(weight(&idl, "E1"), 2);
        assert_eq!(least(&idl, "E12"), 0);
        assert_eq!(weight(&idl, "E12"), 1 << 12);
        assert_eq!(weight(&idl, "Arr"), 12);
        // An absent member of an APPENDABLE or MUTABLE structure takes its
        // default: the whole array.
        assert_eq!(weight(&idl, "App"), 8);
        assert_eq!(weight(&idl, "U"), 17);
        assert_eq!(weight(&idl, "Q"), 2);
    }

    #[test]
    fn deep_nesting_keeps_its_least_size() {
        let mut idl = String::from("@final struct S0 { long a; };\n");
        for i in 1..=100 {
            let _ = writeln!(idl, "@final struct S{i} {{ S{} x; }};", i - 1);
        }
        // No depth limit: 34 (or 100) FINAL structures around a `long`
        // still take its 4 bytes.
        assert_eq!(least(&idl, "S31"), 4);
        assert_eq!(least(&idl, "S34"), 4);
        assert_eq!(least(&idl, "S100"), 4);
        assert_eq!(weight(&idl, "S100"), 1);
    }

    #[test]
    fn doubling_chains_are_walked_once_per_type() {
        // F40 holds two F39 ... F1 two F0: 2^40 visits of F0 without the
        // memo of each type's figure; 41 with it.
        let mut idl = String::from("@final struct F0 { long a; };\n@final struct Z0 {};\n");
        for i in 1..=40 {
            let _ = writeln!(idl, "@final struct F{i} {{ F{} a; F{} b; }};", i - 1, i - 1);
            let _ = writeln!(idl, "@final struct Z{i} {{ Z{} a; Z{} b; }};", i - 1, i - 1);
        }
        let start = std::time::Instant::now();
        assert_eq!(least(&idl, "F40"), 4 << 40);
        assert_eq!(weight(&idl, "F40"), 1 << 40);
        assert_eq!(least(&idl, "Z40"), 0);
        assert_eq!(weight(&idl, "Z40"), 1 << 40);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn a_type_that_contains_itself_terminates() {
        // Ill-formed (an infinite value); the recurrence adds nothing.
        let idl = "@final struct R { long a; R r; };";
        assert_eq!(least(idl, "R"), 4);
        assert_eq!(weight(idl, "R"), 2);
    }
}
