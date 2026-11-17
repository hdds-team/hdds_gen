// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! The DDS-`XTypes` 1.3 data model the backends serialize a sample by.
//!
//! Everything here is decided at generation time and is the same for every
//! target language: the extensibility kind of a type, the member ids of a
//! structure (base members first), which collections carry a DHEADER under
//! XCDR2, the holder width of an enumeration or a bitmask, and the length
//! code of a mutable member's `EMHEADER1`.
//!
//! The serialization rules are those of `XTypes` 1.3 7.4.3.5.3; where the
//! specification leaves a choice to the writer (the length code of a
//! member, the length word of an XCDR1 parameter), the choice is the one
//! Cyclone DDS 0.11 makes, so that a sample serializes to the same bytes
//! (see `tests/fixtures/xtypes_datapath/`).

use crate::ast::{Field, Struct, Union, UnionLabel};
use crate::codegen::key_holder::{Named, TypeIndex};
use crate::types::{Annotation, ExtensibilityKind, IdlType, PrimitiveType};

/// Extensibility kind of an aggregated type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ext {
    /// FINAL: members in order, nothing else.
    Final,
    /// APPENDABLE: a DHEADER under XCDR2 (rule 30), as FINAL under XCDR1
    /// (rule 29).
    Appendable,
    /// MUTABLE: a DHEADER and an `EMHEADER1` per member under XCDR2 (rules
    /// 21-22), a parameter list under XCDR1 (rules 23-25).
    Mutable,
}

impl Ext {
    /// The extensibility as the hdds C API numbers it
    /// (`HDDS_EXTENSIBILITY_FINAL` 0, `_APPENDABLE` 1, `_MUTABLE` 2, the
    /// argument of `hdds_writer_create_with_extensibility`).
    #[must_use]
    pub const fn hdds_code(self) -> u32 {
        match self {
            Self::Final => 0,
            Self::Appendable => 1,
            Self::Mutable => 2,
        }
    }

    /// The extensibility as the hdds Python and TypeScript SDKs name it
    /// (`"final"`, `"appendable"`, `"mutable"`).
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Final => "final",
            Self::Appendable => "appendable",
            Self::Mutable => "mutable",
        }
    }
}

fn ext_of(explicit: Option<ExtensibilityKind>, annotations: &[Annotation]) -> Ext {
    let kind = explicit.or_else(|| {
        annotations.iter().find_map(|a| match a {
            Annotation::Extensibility(k) => Some(*k),
            Annotation::Final => Some(ExtensibilityKind::Final),
            Annotation::Appendable => Some(ExtensibilityKind::Appendable),
            Annotation::Mutable => Some(ExtensibilityKind::Mutable),
            _ => None,
        })
    });
    match kind {
        Some(ExtensibilityKind::Mutable) => Ext::Mutable,
        Some(ExtensibilityKind::Final) => Ext::Final,
        // A type without an annotation is APPENDABLE: "In the event that the
        // representation of a given type does not indicate the type's
        // extensibility kind, the type shall be considered appendable"
        // (`XTypes` 1.3 7.3.1.2.1.8, spec text lines 3986-3990). A compiler
        // may offer another default: `apply_default_extensibility`.
        Some(ExtensibilityKind::Appendable) | None => Ext::Appendable,
    }
}

/// Extensibility of a structure.
#[must_use]
pub fn struct_ext(s: &Struct) -> Ext {
    ext_of(s.extensibility, &s.annotations)
}

/// Extensibility of an enumeration or a bitmask (FINAL or APPENDABLE,
/// DDS-XTypes 1.3 7.2.3 Table 12), from its annotations.
#[must_use]
pub fn enumerated_ext(annotations: &[Annotation]) -> Ext {
    ext_of(None, annotations)
}

/// Extensibility of a union.
#[must_use]
pub fn union_ext(u: &Union) -> Ext {
    ext_of(None, &u.annotations)
}

/// The `TRY_CONSTRUCT1` / `TRY_CONSTRUCT2` bits of a member's `MemberFlag`.
///
/// From its `@try_construct` (DDS-XTypes 1.3 7.3.1.2.1.12, spec text lines
/// 4078-4100; Annex B lines 12325-12326): `DISCARD` 01, also without the
/// annotation; `USE_DEFAULT` 10, also without a value; `TRIM` 11. `None`
/// for a value the annotation does not define.
#[must_use]
pub fn try_construct_flags(annotations: &[Annotation]) -> Option<u16> {
    let Some(params) = annotations.iter().find_map(|a| match a {
        Annotation::Custom { name, params } if name == "try_construct" => Some(params),
        _ => None,
    }) else {
        return Some(0x0001);
    };
    let value = params.first().map_or("", |p| p.value.as_str()).trim();
    match value.rsplit("::").next().unwrap_or(value) {
        "DISCARD" => Some(0x0001),
        "" | "USE_DEFAULT" => Some(0x0002),
        "TRIM" => Some(0x0003),
        _ => None,
    }
}

fn has_ext(explicit: Option<ExtensibilityKind>, annotations: &[Annotation]) -> bool {
    explicit.is_some()
        || annotations.iter().any(|a| {
            matches!(
                a,
                Annotation::Extensibility(_)
                    | Annotation::Final
                    | Annotation::Appendable
                    | Annotation::Mutable
            )
        })
}

/// Gives the un-annotated structures, unions, enumerations and bitmasks of
/// `ast` the extensibility `kind`; returns their scoped names (`m::T`).
///
/// DDS-XTypes 1.3 7.3.1.2.1.8 (spec text lines 3986-3990) makes APPENDABLE
/// the default, which is what an un-annotated type means to hddsgen, and
/// lets a compiler offer another one. An enumeration or a bitmask is FINAL
/// or APPENDABLE only (7.2.3 Table 12): `kind` MUTABLE leaves it APPENDABLE.
/// Every named type is annotated, so the AST says explicitly what it means
/// whatever the default.
pub fn apply_default_extensibility(
    ast: &mut crate::ast::IdlFile,
    kind: ExtensibilityKind,
) -> Vec<String> {
    fn walk(
        defs: &mut [crate::ast::Definition],
        scope: &str,
        kind: ExtensibilityKind,
        out: &mut Vec<String>,
    ) {
        use crate::ast::Definition;
        let enumerated = if kind == ExtensibilityKind::Final {
            ExtensibilityKind::Final
        } else {
            ExtensibilityKind::Appendable
        };
        for d in defs {
            match d {
                Definition::Module(m) => {
                    let inner = format!("{scope}{}::", m.name);
                    walk(&mut m.definitions, &inner, kind, out);
                }
                Definition::Struct(s) if !has_ext(s.extensibility, &s.annotations) => {
                    out.push(format!("{scope}{}", s.name));
                    s.extensibility = Some(kind);
                }
                Definition::Union(u) if !has_ext(None, &u.annotations) => {
                    out.push(format!("{scope}{}", u.name));
                    u.annotations.push(Annotation::Extensibility(kind));
                }
                Definition::Enum(e) if !has_ext(None, &e.annotations) => {
                    out.push(format!("{scope}{}", e.name));
                    e.annotations.push(Annotation::Extensibility(enumerated));
                }
                Definition::Bitmask(b) if !has_ext(None, &b.annotations) => {
                    out.push(format!("{scope}{}", b.name));
                    b.annotations.push(Annotation::Extensibility(enumerated));
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&mut ast.definitions, "", kind, &mut out);
    out
}

/// One member of a structure, its base members included.
#[derive(Clone, Debug)]
pub struct ChainMember {
    /// The IDL member.
    pub field: Field,
    /// Name of the structure that declares it.
    pub owner: String,
    /// How many `base` hops lead from the structure to `owner`.
    pub base_depth: usize,
    /// Member id (`XTypes` 1.3 7.3.1.2.1.1).
    pub id: u32,
}

/// The members of `s` in serialization order: the root base's first, then
/// each derived type's, with their member ids. Members that are
/// `@non_serialized` are left out (their ids are still counted).
///
/// # Errors
///
/// Returns the reason when a base is unknown or the chain is cyclic.
pub fn member_chain(index: &TypeIndex, s: &Struct) -> Result<Vec<ChainMember>, String> {
    let chain = index.base_chain(s)?;
    let modes: Vec<(&Struct, bool)> = chain
        .iter()
        .map(|c| (*c, index.hashes_member_ids(c)))
        .collect();
    let ids = crate::codegen::key_holder::spec_member_ids_with(&modes);
    let hops = chain.len() - 1;
    let mut ids = ids.into_iter();
    let mut out = Vec::new();
    for (level, owner) in chain.iter().enumerate() {
        for field in &owner.fields {
            let id = ids.next().unwrap_or(0);
            if field.is_non_serialized() {
                continue;
            }
            out.push(ChainMember {
                field: field.clone(),
                owner: owner.name.clone(),
                base_depth: hops - level,
                id,
            });
        }
    }
    Ok(out)
}

/// `KeyHolder` members in declaration order, preserving original member ids.
/// A nested structure without explicit keys contributes all its members
/// (DDS-XTypes 1.3 7.2.2.4.7).
pub(crate) fn key_members(mut chain: Vec<ChainMember>) -> Vec<ChainMember> {
    if chain.iter().any(|m| m.field.is_key()) {
        chain.retain(|m| m.field.is_key());
    }
    chain
}

/// A keyed union's `KeyHolder` is its discriminator alone. Adapt that scalar
/// to the existing hash emitters without changing its native member access.
pub(crate) fn discriminator_holder(u: &Union, member: &str) -> Struct {
    let mut s = Struct::new(&u.name);
    if u.discriminator_is_key() {
        let mut field = Field::new(member, u.discriminator.clone());
        field.annotations.push(Annotation::Key);
        s.fields.push(field);
    }
    s
}

/// The member ids of `s`'s own members (its bases' members counted first),
/// in declaration order, `@non_serialized` members included.
#[must_use]
pub fn own_member_ids(index: &TypeIndex, s: &Struct) -> Vec<u32> {
    let Ok(chain) = index.base_chain(s) else {
        return Vec::new();
    };
    let modes: Vec<(&Struct, bool)> = chain
        .iter()
        .map(|c| (*c, index.hashes_member_ids(c)))
        .collect();
    let ids = crate::codegen::key_holder::spec_member_ids_with(&modes);
    let skip: usize = chain[..chain.len() - 1]
        .iter()
        .map(|c| c.fields.len())
        .sum();
    ids.into_iter().skip(skip).collect()
}

/// Width in bytes of the integer an enumeration is serialized as
/// (`XTypes` 1.3 7.3.1.2.1.6: `@bit_bound` 1-8 -> int8, 9-16 -> int16,
/// otherwise int32; the default bound is 32).
#[must_use]
pub fn enum_width(bit_bound: Option<u32>) -> usize {
    match bit_bound.unwrap_or(32) {
        1..=8 => 1,
        9..=16 => 2,
        _ => 4,
    }
}

/// Width in bytes of the integer a bitmask is serialized as (`@bit_bound`
/// 1-8 -> uint8, 9-16 -> uint16, 17-32 -> uint32, otherwise uint64; the
/// default bound is 32).
#[must_use]
pub fn bitmask_width(bit_bound: Option<u32>) -> usize {
    match bit_bound.unwrap_or(32) {
        1..=8 => 1,
        9..=16 => 2,
        17..=32 => 4,
        _ => 8,
    }
}

/// Serialized size of an `XTypes` 1.3 primitive type.
///
/// 7.2.2.2: boolean, byte, the integers, the floats and the characters;
/// `None` otherwise (strings, `fixed`, `void`). `long double` is reported
/// as the 8 bytes hddsgen maps it to.
#[must_use]
pub const fn primitive_size(p: &PrimitiveType) -> Option<usize> {
    match p {
        PrimitiveType::Boolean
        | PrimitiveType::Octet
        | PrimitiveType::UInt8
        | PrimitiveType::Int8
        | PrimitiveType::Char => Some(1),
        PrimitiveType::Short
        | PrimitiveType::UnsignedShort
        | PrimitiveType::Int16
        | PrimitiveType::UInt16
        | PrimitiveType::WChar => Some(2),
        PrimitiveType::Long
        | PrimitiveType::UnsignedLong
        | PrimitiveType::Int32
        | PrimitiveType::UInt32
        | PrimitiveType::Float => Some(4),
        PrimitiveType::LongLong
        | PrimitiveType::UnsignedLongLong
        | PrimitiveType::Int64
        | PrimitiveType::UInt64
        | PrimitiveType::Double
        | PrimitiveType::LongDouble => Some(8),
        PrimitiveType::String
        | PrimitiveType::WString
        | PrimitiveType::Fixed { .. }
        | PrimitiveType::Void => None,
    }
}

/// Size of the element when `ty` (resolved) is an `XTypes` primitive.
///
/// That element kind serializes a sequence or an array with no DHEADER
/// under XCDR2 (rules 8, 11). Enumerations, bitmasks and strings are not
/// primitive: their collections carry a DHEADER (rules 9, 12).
#[must_use]
pub fn primitive_element(index: &TypeIndex, ty: &IdlType) -> Option<usize> {
    match index.resolve(ty) {
        IdlType::Primitive(p) => primitive_size(&p),
        _ => None,
    }
}

/// Whether `ty` is a string or a bounded string (`string<N>`,
/// `wstring<N>`). A `sequence<char>` is not: it serializes its element
/// count and no NUL (DDS-XTypes 1.3 7.4.3.5.3 rule 11 versus rules 3-4).
#[must_use]
pub fn is_string_like(index: &TypeIndex, ty: &IdlType) -> bool {
    matches!(
        index.resolve(ty),
        IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString)
            | IdlType::BoundedString { .. }
    )
}

/// Element type and total element count of a (possibly multi-dimensional)
/// array, the innermost element resolved.
#[must_use]
pub fn flatten_array(index: &TypeIndex, ty: &IdlType) -> Option<(IdlType, Vec<u32>)> {
    let IdlType::Array { inner, size } = index.resolve(ty) else {
        return None;
    };
    let mut dims = vec![size];
    let mut elem = index.resolve(&inner);
    while let IdlType::Array { inner, size } = elem {
        dims.push(size);
        elem = index.resolve(&inner);
    }
    Some((elem, dims))
}

/// Serialized size of a scalar (primitive, enumeration, bitmask), `None`
/// for anything else.
#[must_use]
pub fn scalar_size(index: &TypeIndex, ty: &IdlType) -> Option<usize> {
    match index.resolve(ty) {
        IdlType::Primitive(p) => primitive_size(&p),
        IdlType::Named(n) => match index.named(&n) {
            Some(Named::Enum { bit_bound, .. }) => Some(enum_width(*bit_bound)),
            Some(Named::Bitmask { bit_bound }) => Some(bitmask_width(*bit_bound)),
            _ => None,
        },
        _ => None,
    }
}

/// Whether the XCDR2 serialization of `ty` starts with a DHEADER: a
/// sequence, array or map of non-primitive elements (rules 9, 12, 15).
#[must_use]
pub fn collection_has_dheader(index: &TypeIndex, ty: &IdlType) -> bool {
    match index.resolve(ty) {
        IdlType::Sequence { inner, .. } => {
            !is_string_like(index, ty) && primitive_element(index, &inner).is_none()
        }
        IdlType::Array { .. } => flatten_array(index, ty)
            .is_some_and(|(elem, _)| primitive_element(index, &elem).is_none()),
        IdlType::Map { key, value, .. } => {
            primitive_element(index, &key).is_none() || primitive_element(index, &value).is_none()
        }
        _ => false,
    }
}

/// Length code of the `EMHEADER1` of a mutable member of type `ty` under
/// XCDR2 (`XTypes` 1.3 7.4.3.4.2), as Cyclone DDS 0.11 chooses it:
///
/// - a 1/2/4/8-byte scalar: 0/1/2/3, no NEXTINT;
/// - a string, a collection with a DHEADER, a sequence of 1-byte
///   primitives: 5, the value's own leading length word is the NEXTINT;
/// - a sequence of 4-byte (8-byte) primitives: 6 (7), its element count is
///   the NEXTINT;
/// - anything else (structures, unions, arrays of primitives, sequences of
///   2-byte primitives): 4, a NEXTINT holding the member length precedes
///   the value.
#[must_use]
pub fn length_code(index: &TypeIndex, ty: &IdlType) -> u32 {
    if let Some(size) = scalar_size(index, ty) {
        return match size {
            1 => 0,
            2 => 1,
            4 => 2,
            _ => 3,
        };
    }
    if is_string_like(index, ty) || collection_has_dheader(index, ty) {
        return 5;
    }
    if let IdlType::Sequence { inner, .. } = index.resolve(ty) {
        return match primitive_element(index, &inner) {
            Some(1) => 5,
            Some(4) => 6,
            Some(8) => 7,
            _ => 4,
        };
    }
    4
}

/// Whether the member header of a key member carries the must-understand
/// flag: the XCDR2 `EMHEADER1` M flag (7.4.3.4.2) and the XCDR1 extended
/// parameter header `FLAG_2` (7.4.1.2.2).
///
/// `XTypes` 1.3 7.2.2.4.4.4.8: key members "shall always have their must
/// understand attribute set to true". Cyclone DDS 0.11 sets the M flag
/// under XCDR2 and never sets `FLAG_2` under XCDR1; a reader is robust to
/// either setting for a member it knows (7.4.1.2.1, spec text lines
/// 6085-6087), which `spec_plprobe.txt` exercises. Kept as the one switch
/// of that policy.
pub const KEY_MEMBERS_MUST_UNDERSTAND: bool = true;

/// Whether a member's header carries the must-understand flag: an
/// `@must_understand` member, and a key member (see
/// [`KEY_MEMBERS_MUST_UNDERSTAND`]).
#[must_use]
pub fn must_understand(field: &Field) -> bool {
    field.is_must_understand() || (KEY_MEMBERS_MUST_UNDERSTAND && field.is_key())
}

/// Integer value of a union case label: an integer (decimal or hex), a
/// character, TRUE / FALSE, or a literal of the enumeration discriminator;
/// `None` for anything else (a constant name).
#[must_use]
pub fn union_label_value(idx: &TypeIndex, disc: &IdlType, label: &str) -> Option<i64> {
    let label = label.trim();
    if let Ok(v) = label.parse::<i64>() {
        return Some(v);
    }
    if let Some(hex) = label
        .strip_prefix("0x")
        .or_else(|| label.strip_prefix("0X"))
    {
        return i64::from_str_radix(hex, 16).ok();
    }
    match label {
        "TRUE" | "true" => return Some(1),
        "FALSE" | "false" => return Some(0),
        _ => {}
    }
    if label.len() >= 3 && label.starts_with('\'') && label.ends_with('\'') {
        let inner = &label[1..label.len() - 1];
        let mut chars = inner.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return Some(i64::from(u32::from(c)));
        }
    }
    if let IdlType::Named(n) = idx.resolve(disc) {
        if let Some(Named::Enum { literals, .. }) = idx.named(&n) {
            let bare = label.rsplit("::").next().unwrap_or(label);
            return literals
                .iter()
                .find(|(name, _)| name == bare)
                .map(|(_, v)| *v);
        }
    }
    None
}

/// The discriminator value a case writes: its first label, or for the
/// `default` case the smallest non-negative value no label uses.
#[must_use]
pub fn union_case_disc(idx: &TypeIndex, u: &Union, case_index: usize) -> Option<i64> {
    let case = &u.cases[case_index];
    for label in &case.labels {
        if let UnionLabel::Value(v) = label {
            return union_label_value(idx, &u.discriminator, v);
        }
    }
    let used: Vec<i64> = u
        .cases
        .iter()
        .flat_map(|c| c.labels.iter())
        .filter_map(|l| match l {
            UnionLabel::Value(v) => union_label_value(idx, &u.discriminator, v),
            UnionLabel::Default => None,
        })
        .collect();
    (0..=i64::from(u32::MAX)).find(|v| !used.contains(v))
}

/// Member ids of the cases of a union, in declaration order.
///
/// The discriminator is member 0 and the cases follow it, whatever the
/// union's extensibility (DDS-XTypes 1.3 7.3.1.2.1.1, spec text lines
/// 3815-3826): an explicit `@id`;
/// else, under `@autoid(HASH)` (the union's or an enclosing module's) or
/// with `@hashid`, the MD5 name hash (lines 3866-3873); else the previous
/// id plus one, the first case 1. Cyclone DDS 0.11 numbers the cases of
/// its `TypeObject`s from 0, which this does not follow.
#[must_use]
pub fn union_case_ids(index: &TypeIndex, u: &Union) -> Vec<u32> {
    let hash_mode = index.union_hashes_member_ids(u);
    let mut next: u32 = 1;
    u.cases
        .iter()
        .map(|c| {
            let explicit = c.field.annotations.iter().find_map(|a| match a {
                Annotation::Id(n) => Some(*n),
                _ => None,
            });
            let hashid = c.field.annotations.iter().find_map(|a| match a {
                Annotation::Custom { name, params } if name == "hashid" => Some(
                    params
                        .first()
                        .map(|p| p.value.trim_matches('"').to_string())
                        .filter(|v| !v.is_empty())
                        .unwrap_or_else(|| c.field.name.clone()),
                ),
                _ => None,
            });
            let id = explicit.unwrap_or_else(|| {
                hashid.map_or_else(
                    || {
                        if hash_mode {
                            crate::codegen::key_holder::name_hash_id(&c.field.name)
                        } else {
                            next
                        }
                    },
                    |name| crate::codegen::key_holder::name_hash_id(&name),
                )
            });
            next = id.wrapping_add(1);
            id
        })
        .collect()
}

/// Length code of a union discriminator's `EMHEADER1` (a 1, 2, 4 or
/// 8-byte integer).
#[must_use]
pub fn discriminator_length_code(index: &TypeIndex, disc: &IdlType) -> u32 {
    length_code(index, disc)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::parser::Parser;

    fn index(idl: &str) -> (TypeIndex, crate::ast::IdlFile) {
        let mut p = Parser::try_new(idl).expect("lexer");
        let ast = p.parse().expect("parse");
        (TypeIndex::new(&ast), ast)
    }

    /// `@try_construct` gives the member flag bits of Annex B (spec text
    /// lines 12325-12326); without it, DISCARD.
    #[test]
    fn try_construct_flags_follow_annex_b() {
        let (_, ast) = index(
            "struct T { long a; @try_construct(DISCARD) long d; \
             @try_construct(USE_DEFAULT) long u; @try_construct long n; \
             @try_construct(TRIM) long t; @try_construct(TryConstructFailAction::TRIM) long q; \
             @try_construct(SOMETIMES) long x; };",
        );
        let Some(crate::ast::Definition::Struct(s)) = ast.definitions.first() else {
            unreachable!("one struct")
        };
        let flags: Vec<Option<u16>> = s
            .fields
            .iter()
            .map(|f| try_construct_flags(&f.annotations))
            .collect();
        assert_eq!(
            flags,
            [
                Some(0x0001),
                Some(0x0001),
                Some(0x0002),
                Some(0x0002),
                Some(0x0003),
                Some(0x0003),
                None
            ]
        );
    }

    /// An un-annotated structure, union, enumeration or bitmask is
    /// APPENDABLE (DDS-XTypes 1.3 7.3.1.2.1.8, spec text lines 3986-3990).
    /// `--default-extensibility`: only the un-annotated ones change, at
    /// every module depth; an enumeration or a bitmask never becomes MUTABLE.
    #[test]
    fn default_extensibility_reaches_only_unannotated_types() {
        let idl = "struct A { long a; }; @final struct B { long b; }; \
                   @extensibility(MUTABLE) struct C { long c; }; \
                   enum E { E0, E1 }; @final enum F { F0 }; \
                   module m { module n { union U switch (long) { case 1: long x; }; \
                   @appendable union V switch (long) { case 1: long y; }; \
                   bitmask M { M0 }; }; };";
        let exts = |ast: &crate::ast::IdlFile| {
            let idx = TypeIndex::new(ast);
            let enumerated = |name: &str| {
                fn find<'a>(
                    defs: &'a [crate::ast::Definition],
                    name: &str,
                ) -> Option<&'a [Annotation]> {
                    defs.iter().find_map(|d| match d {
                        crate::ast::Definition::Enum(e) if e.name == name => {
                            Some(e.annotations.as_slice())
                        }
                        crate::ast::Definition::Bitmask(b) if b.name == name => {
                            Some(b.annotations.as_slice())
                        }
                        crate::ast::Definition::Module(m) => find(&m.definitions, name),
                        _ => None,
                    })
                }
                find(&ast.definitions, name).map(enumerated_ext)
            };
            [
                ["A", "B", "C", "m::n::U", "m::n::V"].map(|n| match idx.named(n) {
                    Some(Named::Struct(s)) => Some(struct_ext(s)),
                    Some(Named::Union(u)) => Some(union_ext(u)),
                    _ => None,
                }),
                [
                    enumerated("E"),
                    enumerated("F"),
                    enumerated("M"),
                    None,
                    None,
                ],
            ]
        };
        let (_, ast) = index(idl);
        assert_eq!(
            exts(&ast),
            [
                [
                    Ext::Appendable,
                    Ext::Final,
                    Ext::Mutable,
                    Ext::Appendable,
                    Ext::Appendable
                ]
                .map(Some),
                [
                    Some(Ext::Appendable),
                    Some(Ext::Final),
                    Some(Ext::Appendable),
                    None,
                    None
                ],
            ]
        );
        for (kind, want, want_enum) in [
            (ExtensibilityKind::Final, Ext::Final, Ext::Final),
            (
                ExtensibilityKind::Appendable,
                Ext::Appendable,
                Ext::Appendable,
            ),
            (ExtensibilityKind::Mutable, Ext::Mutable, Ext::Appendable),
        ] {
            let (_, mut ast) = index(idl);
            let names = apply_default_extensibility(&mut ast, kind);
            assert_eq!(names, ["A", "E", "m::n::U", "m::n::M"]);
            assert_eq!(
                exts(&ast),
                [
                    [want, Ext::Final, Ext::Mutable, want, Ext::Appendable].map(Some),
                    [
                        Some(want_enum),
                        Some(Ext::Final),
                        Some(want_enum),
                        None,
                        None
                    ],
                ]
            );
        }
    }

    fn field_type(ast: &crate::ast::IdlFile, s: &str, f: &str) -> IdlType {
        ast.definitions
            .iter()
            .find_map(|d| match d {
                crate::ast::Definition::Struct(st) if st.name == s => {
                    st.fields.iter().find(|x| x.name == f)
                }
                _ => None,
            })
            .expect("field")
            .field_type
            .clone()
    }

    /// The length codes Cyclone DDS 0.11 writes for the members of `MutLC`
    /// and `MutLC2` of the corpus (`cyclone_vectors.txt`).
    #[test]
    fn length_codes_are_those_cyclone_writes() {
        let (idx, ast) = index(
            "@final struct Inner8 { long long x; long y; };
             @bit_bound(8) enum E8 { A, B };
             enum E32 { C, D };
             @bit_bound(8) bitmask BM8 { F };
             bitmask BM32 { G };
             typedef sequence<long> LSeq;
             @mutable struct M { sequence<octet> s1; sequence<short> s2; sequence<long> s4;
               sequence<long long> s8; sequence<double> sd; sequence<boolean> sb; octet a1[3];
               short a2[2]; long a4[2]; string s; sequence<Inner8> si; E32 e; BM32 m;
               long long a8[1]; boolean b; Inner8 ai[1]; E8 ae[2]; string sa[1]; BM8 bm;
               sequence<BM8> sbm; LSeq ls; Inner8 st; string<4> bs; };",
        );
        let expect = [
            ("s1", 5),
            ("s2", 4),
            ("s4", 6),
            ("s8", 7),
            ("sd", 7),
            ("sb", 5),
            ("a1", 4),
            ("a2", 4),
            ("a4", 4),
            ("s", 5),
            ("si", 5),
            ("e", 2),
            ("m", 2),
            ("a8", 4),
            ("b", 0),
            ("ai", 5),
            ("ae", 5),
            ("sa", 5),
            ("bm", 0),
            ("sbm", 5),
            ("ls", 6),
            ("st", 4),
            ("bs", 5),
        ];
        for (f, lc) in expect {
            assert_eq!(
                length_code(&idx, &field_type(&ast, "M", f)),
                lc,
                "member {f}"
            );
        }
    }

    /// The cases of every union start at 1 (the discriminator is member 0);
    /// `@autoid(HASH)` of the union or of its module, and `@hashid`, give
    /// the MD5 name hash (the ids of `color` and `shapesize` in the spec's
    /// Example 2, asserted below).
    #[test]
    fn union_case_ids_start_at_one_and_follow_autoid() {
        let (idx, ast) = index(
            "@final union UF switch (long) { case 1: long a; case 2: @id(7) short b; case 3: long c; };
             @mutable @autoid(HASH) union UMh switch (long) { case 1: long color; case 2: short shapesize; };
             @autoid(HASH) module MH { union UMm switch (long) { case 1: long color; }; };
             union UHi switch (long) { case 1: @hashid(\"shapesize\") long size; case 2: long z; };",
        );
        let unions: Vec<crate::ast::Union> = ast
            .definitions
            .iter()
            .flat_map(|d| match d {
                crate::ast::Definition::Union(u) => vec![u.clone()],
                crate::ast::Definition::Module(m) => m
                    .definitions
                    .iter()
                    .filter_map(|d| match d {
                        crate::ast::Definition::Union(u) => Some(u.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => vec![],
            })
            .collect();
        let ids: Vec<Vec<u32>> = unions.iter().map(|u| union_case_ids(&idx, u)).collect();
        assert_eq!(ids[0], vec![1, 7, 8]);
        assert_eq!(ids[1], vec![0x0FA5_DD70, 0x0477_90DA]);
        assert_eq!(ids[2], vec![0x0FA5_DD70]);
        assert_eq!(ids[3], vec![0x0477_90DA, 0x0477_90DB]);
    }

    #[test]
    fn enum_and_bitmask_widths_follow_bit_bound() {
        assert_eq!(enum_width(Some(8)), 1);
        assert_eq!(enum_width(Some(9)), 2);
        assert_eq!(enum_width(Some(16)), 2);
        assert_eq!(enum_width(Some(17)), 4);
        assert_eq!(enum_width(None), 4);
        assert_eq!(bitmask_width(Some(8)), 1);
        assert_eq!(bitmask_width(Some(16)), 2);
        assert_eq!(bitmask_width(None), 4);
        assert_eq!(bitmask_width(Some(33)), 8);
    }

    #[test]
    fn member_ids_count_the_base_first_and_continue_after_an_explicit_id() {
        let (idx, ast) = index(
            "@mutable struct B { long id; string tag; };
             @mutable struct D : B { long extra; @id(50) short s; long after; };",
        );
        let d = ast
            .definitions
            .iter()
            .find_map(|x| match x {
                crate::ast::Definition::Struct(s) if s.name == "D" => Some(s.clone()),
                _ => None,
            })
            .expect("D");
        let chain = member_chain(&idx, &d).expect("chain");
        let ids: Vec<(String, u32, usize)> = chain
            .iter()
            .map(|m| (m.field.name.clone(), m.id, m.base_depth))
            .collect();
        assert_eq!(
            ids,
            vec![
                ("id".into(), 0, 1),
                ("tag".into(), 1, 1),
                ("extra".into(), 2, 0),
                ("s".into(), 50, 0),
                ("after".into(), 51, 0)
            ]
        );
        assert_eq!(own_member_ids(&idx, &d), vec![2, 50, 51]);
    }

    #[test]
    fn dheader_follows_the_element_kind() {
        let (idx, ast) = index(
            "enum E { A }; @final struct I { long x; };
             struct S { sequence<long> a; sequence<E> b; sequence<string> c; long d[2];
                        I e[2]; string<5> f; sequence<string<3>> g; };",
        );
        let has = |f: &str| collection_has_dheader(&idx, &field_type(&ast, "S", f));
        assert!(!has("a"));
        assert!(has("b"));
        assert!(has("c"));
        assert!(!has("d"));
        assert!(has("e"));
        assert!(!has("f"));
        assert!(has("g"));
    }
}
