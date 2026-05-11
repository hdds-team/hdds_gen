// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! CDR2 codec generation for C.
//!
//! Note: `uninlined_format_args` allowed in this module due to extensive `format!()`
//! usage in code generation that would require significant refactoring.
//!
//! Generates `encode`, `decode`, and `max_size` functions for C structs.

#![allow(clippy::uninlined_format_args)]

use super::index::DefinitionIndex;
use super::CStandard;
use crate::ast::Field;
use crate::types::{IdlType, PrimitiveType};
use std::collections::HashSet;

mod decode;
mod encode;
mod size;

#[derive(Copy, Clone)]
struct PrimitiveScalar {
    align: usize,
    width: usize,
}

const fn primitive_scalar_layout(prim: &PrimitiveType) -> Option<PrimitiveScalar> {
    match prim {
        PrimitiveType::Octet
        | PrimitiveType::UInt8
        | PrimitiveType::Int8
        | PrimitiveType::Boolean
        | PrimitiveType::Char => Some(PrimitiveScalar { align: 1, width: 1 }),
        PrimitiveType::Short
        | PrimitiveType::Int16
        | PrimitiveType::UnsignedShort
        | PrimitiveType::UInt16 => Some(PrimitiveScalar { align: 2, width: 2 }),
        PrimitiveType::Long
        | PrimitiveType::Int32
        | PrimitiveType::UnsignedLong
        | PrimitiveType::UInt32
        | PrimitiveType::Float
        | PrimitiveType::WChar => Some(PrimitiveScalar { align: 4, width: 4 }),
        // XCDR2 §7.4.3.4.1 Tab.15: 8-byte primitives align on 4 (cap-4),
        // not 8. Mirrors `impl_cdr2_primitive!` (1.6.1a-impls-macro) on the
        // Rust side. Pre-1.6.1e value was `align: 8` (F01 systemic bug).
        PrimitiveType::LongLong
        | PrimitiveType::Int64
        | PrimitiveType::UnsignedLongLong
        | PrimitiveType::UInt64
        | PrimitiveType::Double
        | PrimitiveType::LongDouble => Some(PrimitiveScalar { align: 4, width: 8 }),
        PrimitiveType::Fixed { .. } => Some(PrimitiveScalar {
            align: 4,
            width: 16,
        }),
        PrimitiveType::String | PrimitiveType::WString | PrimitiveType::Void => None,
    }
}

fn max_scalar(indent: &str, align: usize, size: usize) -> String {
    format!("{indent}offset = cdr_align(offset, {align}) + {size};\n")
}

pub(super) fn emit_encode_field(
    f: &Field,
    idx: &DefinitionIndex,
    parent: &str,
    c_std: CStandard,
) -> String {
    encode::emit_encode_field(f, idx, parent, c_std)
}

pub(super) fn emit_decode_field(
    f: &Field,
    idx: &DefinitionIndex,
    parent: &str,
    c_std: CStandard,
) -> String {
    decode::emit_decode_field(f, idx, parent, c_std)
}

pub(super) fn emit_max_field(f: &Field, idx: &DefinitionIndex, parent: &str) -> String {
    size::emit_max_field(f, idx, parent)
}

pub(super) fn label_to_c(discr: &IdlType, label: &str) -> String {
    size::label_to_c(discr, label)
}

/// Collects variable declarations needed for C89 compatibility.
/// Returns a set of declaration strings to emit at function start.
pub(super) fn collect_c89_declarations(fields: &[Field], idx: &DefinitionIndex) -> HashSet<String> {
    let mut decls = HashSet::new();

    for field in fields {
        collect_type_decls(&field.field_type, &field.name, idx, &mut decls);
    }

    decls
}

fn collect_type_decls(
    ty: &IdlType,
    field_name: &str,
    idx: &DefinitionIndex,
    decls: &mut HashSet<String>,
) {
    match ty {
        IdlType::Primitive(PrimitiveType::String) => {
            decls.insert(format!("uint32_t {}_len;", field_name));
        }
        IdlType::Primitive(PrimitiveType::WString) => {
            decls.insert(format!("uint32_t {}_bytes;", field_name));
        }
        IdlType::Primitive(PrimitiveType::WChar) => {
            decls.insert("uint32_t scalar;".to_string());
        }
        IdlType::Primitive(PrimitiveType::Fixed { .. }) => {
            decls.insert("uint8_t raw[CDR_SIZE_FIXED128];".to_string());
        }
        IdlType::Array { inner, .. } | IdlType::Sequence { inner, .. } => {
            decls.insert("uint32_t i;".to_string());
            collect_type_decls(inner, &format!("{}_elem", field_name), idx, decls);
        }
        IdlType::Map { key, value, .. } => {
            decls.insert("uint32_t i;".to_string());
            collect_type_decls(key, &format!("{}_key", field_name), idx, decls);
            collect_type_decls(value, &format!("{}_value", field_name), idx, decls);
        }
        IdlType::Named(nm) => {
            let type_ident = super::helpers::last_ident_owned(nm);
            if let Some(td) = idx.typedefs.get(&type_ident) {
                collect_type_decls(&td.base_type, field_name, idx, decls);
            }
        }
        IdlType::Primitive(_) => {} // Primitives need no forward declarations
    }
}
