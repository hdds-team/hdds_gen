// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Union code generation for C++.
//!
//! Generates C++ class representations of IDL discriminated unions.

use super::helpers::{last_ident, push_fmt, type_to_cpp};
use super::index::DefinitionIndex;
use super::CppGenerator;
use crate::ast::{Union, UnionCase};
use crate::types::{IdlType, PrimitiveType};

/// Returns true if any case field has a non-trivial C++ type that cannot
/// live inside an anonymous union without explicit constructors/destructors.
pub(super) fn has_nontrivial_case(u: &Union, idx: &DefinitionIndex) -> bool {
    u.cases
        .iter()
        .any(|c| is_nontrivial_type(&c.field.field_type, idx))
}

fn is_nontrivial_type(ty: &IdlType, idx: &DefinitionIndex) -> bool {
    // Through typedefs: `typedef sequence<P> PList;` is a `std::vector`
    // whatever module declares it.
    match idx.keys.resolve(ty) {
        // Every sequence is a `std::vector`; a bounded string is a
        // `std::array` of characters (trivial).
        IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString)
        | IdlType::Map { .. }
        | IdlType::Sequence { .. } => true,
        IdlType::Array { inner, .. } => is_nontrivial_type(&inner, idx),
        IdlType::Named(nm) => {
            idx.keys.struct_def(&nm).is_some() || idx.keys.union_def(&nm).is_some()
        }
        _ => false,
    }
}

pub(super) fn generate_union(generator: &CppGenerator, u: &Union, idx: &DefinitionIndex) -> String {
    let mut output = String::new();
    let nontrivial = has_nontrivial_case(u, idx);
    write_union_prologue(generator, &mut output, u, nontrivial);
    if !nontrivial {
        write_union_cases(generator, &mut output, u);
    }
    write_union_codec(generator, &mut output, u, idx, nontrivial);
    write_union_epilogue(generator, &mut output);
    output
}

fn write_union_prologue(generator: &CppGenerator, out: &mut String, u: &Union, nontrivial: bool) {
    let indent = generator.indent();
    let name = &u.name;
    let disc = &u.discriminator;
    let disc_cpp = type_to_cpp(disc);
    push_fmt(
        out,
        format_args!("{indent}// Union: {name} (discriminator: {disc:?})\n"),
    );
    push_fmt(out, format_args!("{indent}struct {name} {{\n"));

    if nontrivial {
        // Non-trivial members: emit flat fields with default values (no anonymous union)
        push_fmt(
            out,
            format_args!("{indent}    {disc_cpp} _d = {{}};  // discriminator\n"),
        );
        for case in &u.cases {
            let field_type = type_to_cpp(&case.field.field_type);
            let field_name = &case.field.name;
            push_fmt(
                out,
                format_args!("{indent}    {field_type} {field_name}{{}};\n"),
            );
        }
        push_fmt(out, format_args!("\n"));
    } else {
        push_fmt(
            out,
            format_args!("{indent}    {disc_cpp} _d;  // discriminator\n"),
        );
        push_fmt(out, format_args!("{indent}    union {{\n"));
    }
}

fn write_union_cases(generator: &CppGenerator, out: &mut String, u: &Union) {
    for case in &u.cases {
        write_union_case(generator, out, case);
    }
}

fn write_union_case(generator: &CppGenerator, out: &mut String, case: &UnionCase) {
    let indent = generator.indent();
    let field_type = type_to_cpp(&case.field.field_type);
    let field_name = &case.field.name;
    let needs_comment = matches!(
        case.field.field_type,
        IdlType::Array { .. } | IdlType::Sequence { .. } | IdlType::Map { .. }
    );

    if needs_comment {
        let idl_type = case.field.field_type.to_idl_string();
        push_fmt(
            out,
            format_args!(
                "{indent}        {field_type} {field_name};  // was: {idl_type} {field_name}\n"
            ),
        );
    } else {
        push_fmt(
            out,
            format_args!("{indent}        {field_type} {field_name};\n"),
        );
    }
}

fn write_union_epilogue(generator: &CppGenerator, out: &mut String) {
    let indent = generator.indent();
    // Note: _u is closed by write_union_codec, just close the struct
    push_fmt(out, format_args!("{indent}}};\n\n"));
}

fn write_union_codec(
    generator: &CppGenerator,
    out: &mut String,
    u: &Union,
    idx: &DefinitionIndex,
    nontrivial: bool,
) {
    let indent = generator.indent();
    let member_indent = format!("{indent}    ");

    if !nontrivial {
        // Close the anonymous union
        push_fmt(out, format_args!("{indent}    }} _u;\n\n"));
    }
    let holder = crate::codegen::data_model::discriminator_holder(u, "_d");
    out.push_str(&super::key_hash::instance_key_methods(&holder, idx, false));
    out.push_str(&super::codec::xcdr2::union_codec(
        idx,
        u,
        &member_indent,
        nontrivial,
        &generator.type_name(&u.name),
    ));
}

/// A union case label as a C++ constant of the discriminator's type. The
/// value is `data_model::union_label_value` (integers, characters, TRUE /
/// FALSE, enumerators, scoped or not); a label it cannot evaluate (a
/// constant name) is written as it is, an enumerator qualified by its
/// enumeration.
pub(super) fn label_to_cpp(idx: &DefinitionIndex, disc: &IdlType, label: &str) -> String {
    let trimmed = label.trim();
    if let Some(v) = crate::codegen::data_model::union_label_value(&idx.keys, disc, trimmed) {
        let lit = if v == i64::MIN {
            "(-9223372036854775807LL - 1)".to_string()
        } else {
            format!("{v}LL")
        };
        return format!("static_cast<{}>({lit})", type_to_cpp(disc));
    }
    match disc {
        IdlType::Named(enum_name) if !trimmed.contains("::") => {
            format!("{}::{trimmed}", last_ident(enum_name))
        }
        _ => trimmed.to_string(),
    }
}
