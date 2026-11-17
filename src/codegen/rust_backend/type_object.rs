// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Complete `TypeObject`s of the generated Rust types (DDS-`XTypes` 1.3
//! 7.3.4), as the HDDS runtime announces them in SEDP and serves them over
//! `TypeLookup`.
//!
//! Every structure, union, enumeration and bitmask gets two hidden free
//! functions next to its definition: `__hdds_type_object_<Name>()` builds
//! its `CompleteTypeObject`, `__hdds_type_id_<Name>()` its `EK_COMPLETE`
//! `TypeIdentifier` (the object's equivalence hash, computed once). A
//! member referencing a named type uses that type's identifier; strings,
//! sequences, arrays and maps of anything use the plain identifiers
//! (`TI_STRING8_SMALL`, `TI_PLAIN_SEQUENCE_SMALL`, ...), as Cyclone DDS
//! idlc writes them, so that the objects hash the same.
//!
//! A typedef gets its own alias object (`CompleteAliasType`, 7.3.4.6.1)
//! and a member of a typedef'd type references the alias, as Cyclone DDS
//! idlc writes it.

#![allow(clippy::uninlined_format_args)]

use std::collections::BTreeSet;
use std::fmt::Write as _;

use super::super::data_model::{
    enumerated_ext, own_member_ids, struct_ext, try_construct_flags, union_ext, Ext,
};
use super::super::key_holder::{Named, TypeIndex};
use crate::ast::{Bitmask, Enum, Struct, Typedef, Union, UnionLabel};
use crate::types::{Annotation, IdlType, PrimitiveType};

const X: &str = "::hdds::xtypes";

/// `TypeIdentifier` expression of `ty`, and whether it is fully descriptive
/// (no hash inside: the `EK_BOTH` collections).
fn type_id(idx: &TypeIndex, ty: &IdlType, cyclic: &dyn Fn(&str) -> bool) -> (String, bool) {
    if let IdlType::Named(n) = ty {
        if matches!(idx.named(n), Some(Named::Typedef(_))) {
            return if cyclic(n) {
                (format!("{X}::TypeIdentifier::TK_UINT32"), true)
            } else {
                (format!("{}()", fn_path(n, "__hdds_type_id_")), false)
            };
        }
    }
    match idx.resolve(ty) {
        IdlType::Primitive(p) => (prim_id(p), true),
        IdlType::BoundedString { wide, bound } => (string_id(wide, bound), true),
        IdlType::Sequence { inner, bound } => {
            let (elem, full) = type_id(idx, &inner, cyclic);
            let header = collection_header(full);
            let b = bound.unwrap_or(0);
            let expr = if b <= 255 {
                format!(
                    "{X}::TypeIdentifier::PlainSequenceSmall({X}::PlainSequenceSElemDefn {{ header: {header}, bound: {b}, element_identifier: Box::new({elem}) }})"
                )
            } else {
                format!(
                    "{X}::TypeIdentifier::PlainSequenceLarge({X}::PlainSequenceLElemDefn {{ header: {header}, bound: {b}, element_identifier: Box::new({elem}) }})"
                )
            };
            (expr, full)
        }
        IdlType::Array { inner, size } => {
            // The dimensions of THIS declarator only (`long a[2][3]`): an
            // element that names an alias, even of an array, keeps the
            // alias identifier (DDS-XTypes 1.3 7.3.4.6.1), as idlc does.
            let mut dims = vec![size];
            let mut elem_ty = *inner;
            while let IdlType::Array { inner, size } = elem_ty {
                dims.push(size);
                elem_ty = *inner;
            }
            let (elem, full) = type_id(idx, &elem_ty, cyclic);
            let header = collection_header(full);
            let dims_s: Vec<String> = dims.iter().map(u32::to_string).collect();
            let expr = if dims.iter().all(|d| *d <= 255) {
                format!(
                    "{X}::TypeIdentifier::PlainArraySmall({X}::PlainArraySElemDefn {{ header: {header}, array_bound_seq: vec![{}], element_identifier: Box::new({elem}) }})",
                    dims_s.join(", ")
                )
            } else {
                format!(
                    "{X}::TypeIdentifier::PlainArrayLarge({X}::PlainArrayLElemDefn {{ header: {header}, array_bound_seq: vec![{}], element_identifier: Box::new({elem}) }})",
                    dims_s.join(", ")
                )
            };
            (expr, full)
        }
        IdlType::Map { key, value, bound } => {
            let (k, kfull) = type_id(idx, &key, cyclic);
            let (v, vfull) = type_id(idx, &value, cyclic);
            let full = kfull && vfull;
            let header = collection_header(full);
            let b = bound.unwrap_or(0);
            let flags = format!("{X}::CollectionElementFlag(0x0001)");
            let expr = if b <= 255 {
                format!(
                    "{X}::TypeIdentifier::PlainMapSmall({X}::PlainMapSTypeDefn {{ header: {header}, bound: {b}, element_identifier: Box::new({v}), key_flags: {flags}, key_identifier: Box::new({k}) }})"
                )
            } else {
                format!(
                    "{X}::TypeIdentifier::PlainMapLarge({X}::PlainMapLTypeDefn {{ header: {header}, bound: {b}, element_identifier: Box::new({v}), key_flags: {flags}, key_identifier: Box::new({k}) }})"
                )
            };
            (expr, full)
        }
        IdlType::Named(n) => match idx.named(&n) {
            Some(
                Named::Struct(_) | Named::Union(_) | Named::Enum { .. } | Named::Bitmask { .. },
            ) => {
                if cyclic(&n) {
                    // A type that reaches itself would need a strongly
                    // connected component identifier (7.3.4.9), which is
                    // not generated: an inert placeholder breaks the loop.
                    (format!("{X}::TypeIdentifier::TK_UINT32"), true)
                } else {
                    (format!("{}()", fn_path(&n, "__hdds_type_id_")), false)
                }
            }
            _ => (format!("{X}::TypeIdentifier::TK_UINT64"), true),
        },
    }
}

fn collection_header(full: bool) -> String {
    let ek = if full { "Both" } else { "Complete" };
    format!(
        "{X}::PlainCollectionHeader {{ equiv_kind: {X}::EquivalenceKind::{ek}, element_flags: {X}::CollectionElementFlag(0x0001) }}"
    )
}

fn string_id(wide: bool, bound: u32) -> String {
    match (wide, bound <= 255) {
        (false, true) => format!("{X}::TypeIdentifier::StringSmall {{ bound: {bound} }}"),
        (false, false) => format!("{X}::TypeIdentifier::StringLarge {{ bound: {bound} }}"),
        (true, true) => format!("{X}::TypeIdentifier::WStringSmall {{ bound: {bound} }}"),
        (true, false) => format!("{X}::TypeIdentifier::WStringLarge {{ bound: {bound} }}"),
    }
}

fn prim_id(p: PrimitiveType) -> String {
    let kind = match p {
        PrimitiveType::Boolean => "TK_BOOLEAN",
        PrimitiveType::Octet | PrimitiveType::Void => "TK_BYTE",
        PrimitiveType::UInt8 => "TK_UINT8",
        PrimitiveType::Int8 => "TK_INT8",
        PrimitiveType::Char => "TK_CHAR8",
        PrimitiveType::WChar => "TK_CHAR16",
        PrimitiveType::Short | PrimitiveType::Int16 => "TK_INT16",
        PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => "TK_UINT16",
        PrimitiveType::Long | PrimitiveType::Int32 => "TK_INT32",
        PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => "TK_UINT32",
        PrimitiveType::LongLong | PrimitiveType::Int64 | PrimitiveType::Fixed { .. } => "TK_INT64",
        PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => "TK_UINT64",
        PrimitiveType::Float => "TK_FLOAT32",
        PrimitiveType::Double => "TK_FLOAT64",
        PrimitiveType::LongDouble => {
            return format!("{X}::TypeIdentifier::Primitive({X}::TypeKind::TK_FLOAT128)")
        }
        PrimitiveType::String => return string_id(false, 0),
        PrimitiveType::WString => return string_id(true, 0),
    };
    format!("{X}::TypeIdentifier::{kind}")
}

/// Path of a hidden per-type function for the IDL scoped name `name`.
fn fn_path(name: &str, prefix: &str) -> String {
    let name = &crate::codegen::rust_paths::path(name);
    match name.rsplit_once("::") {
        Some((scope, last)) => format!("{scope}::{prefix}{last}"),
        None => format!("{prefix}{name}"),
    }
}

/// Named types `ty` references directly (a typedef is an alias object).
fn direct_refs(idx: &TypeIndex, ty: &IdlType, out: &mut Vec<String>) {
    match ty.clone() {
        IdlType::Sequence { inner, .. } | IdlType::Array { inner, .. } => {
            direct_refs(idx, &inner, out);
        }
        IdlType::Map { key, value, .. } => {
            direct_refs(idx, &key, out);
            direct_refs(idx, &value, out);
        }
        IdlType::Named(n) => {
            if matches!(
                idx.named(&n),
                Some(
                    Named::Struct(_)
                        | Named::Union(_)
                        | Named::Enum { .. }
                        | Named::Bitmask { .. }
                        | Named::Typedef(_)
                )
            ) {
                out.push(n);
            }
        }
        IdlType::Primitive(_) | IdlType::BoundedString { .. } => {}
    }
}

/// Named types the object of `name` references directly (members, base,
/// discriminator).
fn refs_of(idx: &TypeIndex, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    match idx.named(name) {
        Some(Named::Struct(s)) => {
            if let Some(b) = &s.base_struct {
                out.push(b.clone());
            }
            for f in &s.fields {
                direct_refs(idx, &f.field_type, &mut out);
            }
        }
        Some(Named::Union(u)) => {
            direct_refs(idx, &u.discriminator, &mut out);
            for c in &u.cases {
                direct_refs(idx, &c.field.field_type, &mut out);
            }
        }
        Some(Named::Typedef(base)) => direct_refs(idx, base, &mut out),
        _ => {}
    }
    out
}

/// The global name of the type `name` names (`a::b::T`), or `name` itself
/// when it names no indexed type.
fn global(idx: &TypeIndex, name: &str) -> String {
    idx.global(name).unwrap_or_else(|| name.to_string())
}

/// Every named type reachable from `name`, by global name.
fn reachable(idx: &TypeIndex, name: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack: Vec<String> = refs_of(idx, name);
    while let Some(n) = stack.pop() {
        let key = global(idx, &n);
        if seen.insert(key.clone()) {
            stack.extend(refs_of(idx, &format!("::{key}")));
        }
    }
    seen
}

/// Whether the object of `from` would reach the object of `to` through
/// member references (a cycle when `to` is the referencing type).
fn reaches(idx: &TypeIndex, from: &str, to: &str) -> bool {
    let to = global(idx, to);
    global(idx, from) == to || reachable(idx, from).contains(&to)
}

fn member_detail(name: &str, annotations: &[Annotation]) -> String {
    let hashid = annotations.iter().find_map(|a| match a {
        Annotation::Custom { name: n, params } if n == "hashid" => Some(
            params
                .first()
                .map(|p| p.value.trim_matches('"').to_string())
                .unwrap_or_default(),
        ),
        _ => None,
    });
    hashid.map_or_else(
        || format!("{X}::CompleteMemberDetail::new({name:?})"),
        |h| {
            format!(
                "{X}::CompleteMemberDetail {{ name: {name:?}.to_string(), ann_builtin: Some({X}::AppliedBuiltinMemberAnnotations {{ hash_id: Some({h:?}.to_string()), ..Default::default() }}), ann_custom: None }}"
            )
        },
    )
}

/// Header of the hidden per-type functions, `fqn` the type's name.
fn emit_fns(out: &mut String, name: &str, object: &str) {
    let _ = write!(
        out,
        "#[doc(hidden)]\n#[allow(non_snake_case, clippy::all, clippy::pedantic)]\n\
         pub fn __hdds_type_object_{name}() -> {X}::CompleteTypeObject {{\n    {object}\n}}\n\n\
         #[doc(hidden)]\n#[allow(non_snake_case, clippy::all, clippy::pedantic)]\n\
         pub fn __hdds_type_id_{name}() -> {X}::TypeIdentifier {{\n    \
         static HASH: ::std::sync::OnceLock<{X}::EquivalenceHash> = ::std::sync::OnceLock::new();\n    \
         {X}::TypeIdentifier::Complete(*HASH.get_or_init(|| {{\n        \
         __hdds_type_object_{name}()\n            .compute_equivalence_hash()\n            \
         .expect(\"CDR2 encoding of a generated CompleteTypeObject\")\n    }}))\n}}\n\n"
    );
}

/// The hidden functions of a structure.
pub(super) fn emit_struct(idx: &TypeIndex, s: &Struct, fqn: &str) -> String {
    let own = format!("::{fqn}");
    let cyclic = |n: &str| reaches(idx, n, &own);
    let mut flags: u16 = match struct_ext(s) {
        Ext::Final => 0x0001,
        Ext::Appendable => 0x0002,
        Ext::Mutable => 0x0004,
    };
    if s.annotations
        .iter()
        .any(|a| matches!(a, Annotation::Nested))
    {
        flags |= 0x0008;
    }
    if idx.hashes_member_ids(s) {
        flags |= 0x0010;
    }
    let base = s.base_struct.as_ref().map_or_else(
        || "None".to_string(),
        |b| format!("Some({}())", fn_path(b, "__hdds_type_id_")),
    );
    let ids = own_member_ids(idx, s);
    let mut members = String::new();
    for (i, f) in s.fields.iter().enumerate() {
        let Some(mut mflags) = try_construct_flags(&f.annotations) else {
            return format!(
                "compile_error!(\"hddsgen: `{}::{}`: @try_construct takes DISCARD, USE_DEFAULT or TRIM\");\n",
                s.name, f.name
            );
        };
        if f.is_external() {
            mflags |= 0x0004;
        }
        if f.is_optional() {
            mflags |= 0x0008;
        }
        if f.is_must_understand() {
            mflags |= 0x0010;
        }
        if f.is_key() {
            mflags |= 0x0020;
        }
        let (tid, _) = type_id(idx, &f.field_type, &cyclic);
        let id = ids.get(i).copied().unwrap_or(0);
        let _ = write!(
            members,
            "{X}::CompleteStructMember {{ common: {X}::CommonStructMember {{ member_id: {id}, member_flags: {X}::MemberFlag({mflags:#06x}), member_type_id: {tid} }}, detail: {} }}, ",
            member_detail(&f.name, &f.annotations)
        );
    }
    let object = format!(
        "{X}::CompleteTypeObject::Struct({X}::CompleteStructType {{ struct_flags: {X}::StructTypeFlag({flags:#06x}), header: {X}::CompleteStructHeader {{ base_type: {base}, detail: {X}::CompleteTypeDetail::new({fqn:?}) }}, member_seq: vec![{members}] }})"
    );
    let mut out = String::new();
    emit_fns(&mut out, &s.name, &object);
    out
}

/// The hidden functions of a union.
pub(super) fn emit_union(idx: &TypeIndex, u: &Union, fqn: &str) -> String {
    let own = format!("::{fqn}");
    let cyclic = |n: &str| reaches(idx, n, &own);
    let flags: u16 = match union_ext(u) {
        Ext::Final => 0x0001,
        Ext::Appendable => 0x0002,
        Ext::Mutable => 0x0004,
    };
    let (disc, _) = type_id(idx, &u.discriminator, &cyclic);
    let mut disc_flags: u16 = 0x0011;
    if u.discriminator_is_key() {
        disc_flags |= 0x0020;
    }
    let mut members = String::new();
    let ids = super::super::data_model::union_case_ids(idx, u);
    for (case, id) in u.cases.iter().zip(ids) {
        let Some(mut mflags) = try_construct_flags(&case.field.annotations) else {
            return format!(
                "compile_error!(\"hddsgen: `{}::{}`: @try_construct takes DISCARD, USE_DEFAULT or TRIM\");\n",
                u.name, case.field.name
            );
        };
        let mut labels = Vec::new();
        for l in &case.labels {
            match l {
                UnionLabel::Default => mflags |= 0x0040,
                UnionLabel::Value(v) => {
                    if let Some(n) =
                        super::super::data_model::union_label_value(idx, &u.discriminator, v)
                    {
                        labels.push(format!("{}", low_i32(n)));
                    }
                }
            }
        }
        let (tid, _) = type_id(idx, &case.field.field_type, &cyclic);
        let _ = write!(
            members,
            "{X}::CompleteUnionMember {{ common: {X}::CommonUnionMember {{ member_id: {id}, member_flags: {X}::MemberFlag({mflags:#06x}), member_type_id: {tid}, label_seq: vec![{}] }}, detail: {} }}, ",
            labels.join(", "),
            member_detail(&case.field.name, &case.field.annotations)
        );
    }
    let object = format!(
        "{X}::CompleteTypeObject::Union({X}::CompleteUnionType {{ union_flags: {X}::UnionTypeFlag({flags:#06x}), header: {X}::CompleteUnionHeader {{ discriminator: {disc}, discriminator_flags: {X}::MemberFlag({disc_flags:#06x}), detail: {X}::CompleteTypeDetail::new({fqn:?}) }}, member_seq: vec![{members}] }})"
    );
    let mut out = String::new();
    emit_fns(&mut out, &u.name, &object);
    out
}

/// The low 32 bits of `v` as the `int32` a `TypeObject` carries (a label or
/// a literal of an unsigned 32-bit type keeps its bit pattern).
const fn low_i32(v: i64) -> i32 {
    let b = v.to_le_bytes();
    i32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// `enum_flags` / `bitmask_flags`: the declared extensibility, `IS_FINAL` or
/// `IS_APPENDABLE` (DDS-XTypes 1.3 7.2.3 Table 12), `IS_APPENDABLE` without
/// an annotation (7.3.1.2.1.8); serialized, so part of the hash.
fn enumerated_flags(annotations: &[Annotation]) -> u16 {
    match enumerated_ext(annotations) {
        Ext::Appendable => 0x0002,
        Ext::Final | Ext::Mutable => 0x0001,
    }
}

/// The hidden functions of a typedef: its alias object, whose related type
/// is the identifier of the type it names (DDS-XTypes 1.3 7.3.4.6.1).
pub(super) fn emit_alias(idx: &TypeIndex, t: &Typedef, fqn: &str) -> String {
    let own = format!("::{fqn}");
    let cyclic = |n: &str| reaches(idx, n, &own);
    let (related, _) = type_id(idx, &t.base_type, &cyclic);
    let object = format!(
        "{X}::CompleteTypeObject::Alias({X}::CompleteAliasType {{ alias_flags: {X}::AliasTypeFlag(0), header: {X}::CompleteAliasHeader {{ detail: {X}::CompleteTypeDetail::new({fqn:?}) }}, body: {X}::CompleteAliasBody {{ common: {X}::CommonAliasBody {{ related_flags: {X}::TypeRelationFlag(0), related_type: {related} }}, ann_builtin: None, ann_custom: None }} }})"
    );
    let mut out = String::new();
    emit_fns(&mut out, &t.name, &object);
    out
}

fn bit_bound(annotations: &[Annotation]) -> Option<u32> {
    annotations.iter().find_map(|a| match a {
        Annotation::BitBound(n) => Some(*n),
        _ => None,
    })
}

/// The hidden functions of an enumeration.
pub(super) fn emit_enum(e: &Enum, fqn: &str) -> String {
    let bb = bit_bound(&e.annotations).unwrap_or(32);
    let mut lits = String::new();
    for (i, (v, value)) in e.variants.iter().zip(e.resolved_values()).enumerate() {
        let flags: u16 = if i == 0 { 0x0040 } else { 0 };
        let _ = write!(
            lits,
            "{X}::CompleteEnumeratedLiteral {{ common: {X}::CommonEnumeratedLiteral {{ value: {}, flags: {X}::EnumeratedLiteralFlag({flags:#06x}) }}, detail: {X}::CompleteMemberDetail::new({:?}) }}, ",
            low_i32(value), v.name
        );
    }
    let ef = enumerated_flags(&e.annotations);
    let object = format!(
        "{X}::CompleteTypeObject::Enumerated({X}::CompleteEnumeratedType {{ enum_flags: {X}::StructTypeFlag({ef:#06x}), header: {X}::CompleteEnumeratedHeader {{ bit_bound: {bb}, detail: {X}::CompleteTypeDetail::new({fqn:?}) }}, literal_seq: vec![{lits}] }})"
    );
    let mut out = String::new();
    emit_fns(&mut out, &e.name, &object);
    out
}

/// The hidden functions of a bitmask.
pub(super) fn emit_bitmask(m: &Bitmask, fqn: &str) -> String {
    let bb = bit_bound(&m.annotations).unwrap_or(32);
    let mut flags = String::new();
    let mut next: u32 = 0;
    for f in &m.flags {
        let pos = f
            .annotations
            .iter()
            .find_map(|a| match a {
                Annotation::Position(p) => Some(*p),
                _ => None,
            })
            .unwrap_or(next);
        next = pos + 1;
        let _ = write!(
            flags,
            "{X}::CompleteBitflag {{ common: {X}::CommonBitflag {{ position: {pos}, flags: {X}::BitflagFlag(0) }}, detail: {X}::CompleteMemberDetail::new({:?}) }}, ",
            f.name
        );
    }
    let bf = enumerated_flags(&m.annotations);
    let object = format!(
        "{X}::CompleteTypeObject::Bitmask({X}::CompleteBitmaskType {{ bitmask_flags: {X}::StructTypeFlag({bf:#06x}), header: {X}::CompleteBitmaskHeader {{ bit_bound: {bb}, detail: {X}::CompleteTypeDetail::new({fqn:?}) }}, flag_seq: vec![{flags}] }})"
    );
    let mut out = String::new();
    emit_fns(&mut out, &m.name, &object);
    out
}

/// The `nested_type_objects()` body of a type: the objects of every named
/// type it reaches.
pub(super) fn nested_objects(idx: &TypeIndex, name: &str) -> String {
    let all = reachable(idx, name);
    let own = global(idx, name);
    // By absolute path: a type reached through another type's member is
    // spelled relative to THAT type's module, not to this one.
    let calls: Vec<String> = all
        .iter()
        .filter(|key| **key != own)
        .map(|key| format!("{}()", fn_path(&format!("::{key}"), "__hdds_type_object_")))
        .collect();
    format!("vec![{}]", calls.join(", "))
}
