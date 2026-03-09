// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Union code generation for C++.
//!
//! Generates C++ class representations of IDL discriminated unions.

use super::helpers::{last_ident, push_fmt, type_to_cpp};
use super::index::DefinitionIndex;
use super::CppGenerator;
use crate::ast::{Union, UnionCase, UnionLabel};
use crate::types::{IdlType, PrimitiveType};
use std::fmt::Write;

/// Returns a loop variable name for the given nesting depth.
fn loop_var(depth: u32) -> &'static str {
    const VARS: &[&str] = &["i", "j", "k", "l", "m", "n"];
    VARS.get(depth as usize).unwrap_or(&"n")
}

/// Returns true if any case field has a non-trivial C++ type that cannot
/// live inside an anonymous union without explicit constructors/destructors.
fn has_nontrivial_case(u: &Union, idx: &DefinitionIndex) -> bool {
    u.cases.iter().any(|c| is_nontrivial_type(&c.field.field_type, idx))
}

fn is_nontrivial_type(ty: &IdlType, idx: &DefinitionIndex) -> bool {
    match ty {
        IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString)
        | IdlType::Sequence { bound: None, .. }
        | IdlType::Map { .. } => true,
        IdlType::Named(nm) => {
            let ident = last_ident(nm);
            idx.structs.contains_key(ident) || idx.unions.contains_key(ident)
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
    let body_indent = format!("{indent}        ");

    if !nontrivial {
        // Close the anonymous union
        push_fmt(out, format_args!("{indent}    }} _u;\n\n"));
    }

    // encode_cdr2_le method
    push_fmt(
        out,
        format_args!("{member_indent}/// Encode this union to CDR2 little-endian format\n"),
    );
    push_fmt(
        out,
        format_args!("{member_indent}/// Returns the number of bytes written, or -1 on error\n"),
    );
    push_fmt(
        out,
        format_args!(
            "{member_indent}[[nodiscard]] int encode_cdr2_le(std::uint8_t* dst, std::size_t len) const noexcept {{\n"
        ),
    );
    push_fmt(out, format_args!("{body_indent}std::size_t offset = 0;\n"));

    // Encode discriminator
    out.push_str(&emit_encode_discriminator(&u.discriminator, &body_indent));

    // Switch on discriminator to encode the appropriate field
    out.push_str(&emit_encode_switch(u, idx, &body_indent, nontrivial));

    push_fmt(
        out,
        format_args!("{body_indent}return static_cast<int>(offset);\n"),
    );
    push_fmt(out, format_args!("{member_indent}}}\n\n"));

    // decode_cdr2_le method
    push_fmt(
        out,
        format_args!("{member_indent}/// Decode this union from CDR2 little-endian format\n"),
    );
    push_fmt(
        out,
        format_args!("{member_indent}/// Returns the number of bytes read, or -1 on error\n"),
    );
    push_fmt(
        out,
        format_args!(
            "{member_indent}[[nodiscard]] int decode_cdr2_le(const std::uint8_t* src, std::size_t len) noexcept {{\n"
        ),
    );
    push_fmt(out, format_args!("{body_indent}std::size_t offset = 0;\n"));

    // Decode discriminator
    out.push_str(&emit_decode_discriminator(&u.discriminator, &body_indent));

    // Switch on discriminator to decode the appropriate field
    out.push_str(&emit_decode_switch(u, idx, &body_indent, nontrivial));

    push_fmt(
        out,
        format_args!("{body_indent}return static_cast<int>(offset);\n"),
    );
    push_fmt(out, format_args!("{member_indent}}}\n"));
}

fn emit_encode_discriminator(disc: &IdlType, indent: &str) -> String {
    let (align, size) = discriminator_layout(disc);
    format!(
        "{indent}offset = cdr2::align_offset(offset, {align});\n\
         {indent}if (!cdr2::can_write(len, offset, {size})) return -1;\n\
         {indent}std::memcpy(dst + offset, &(this->_d), {size});\n\
         {indent}offset += {size};\n"
    )
}

fn emit_decode_discriminator(disc: &IdlType, indent: &str) -> String {
    let (align, size) = discriminator_layout(disc);
    format!(
        "{indent}offset = cdr2::align_offset(offset, {align});\n\
         {indent}if (!cdr2::can_read(len, offset, {size})) return -1;\n\
         {indent}std::memcpy(&(this->_d), src + offset, {size});\n\
         {indent}offset += {size};\n"
    )
}

/// Returns (alignment, size) for a discriminator type
#[allow(clippy::missing_const_for_fn)]
fn discriminator_layout(disc: &IdlType) -> (usize, usize) {
    use crate::types::PrimitiveType;
    #[allow(clippy::match_same_arms)]
    match disc {
        IdlType::Primitive(p) => match p {
            PrimitiveType::Octet
            | PrimitiveType::UInt8
            | PrimitiveType::Int8
            | PrimitiveType::Boolean
            | PrimitiveType::Char => (1, 1),
            PrimitiveType::Short
            | PrimitiveType::Int16
            | PrimitiveType::UnsignedShort
            | PrimitiveType::UInt16 => (2, 2),
            PrimitiveType::Long
            | PrimitiveType::Int32
            | PrimitiveType::UnsignedLong
            | PrimitiveType::UInt32 => (4, 4),
            PrimitiveType::LongLong
            | PrimitiveType::Int64
            | PrimitiveType::UnsignedLongLong
            | PrimitiveType::UInt64 => (8, 8),
            _ => (4, 4), // Default for enums and other types
        },
        IdlType::Named(_)
        | IdlType::Sequence { .. }
        | IdlType::Array { .. }
        | IdlType::Map { .. } => (4, 4), // Enums and other types are 4-byte aligned int32
    }
}

fn emit_encode_switch(u: &Union, idx: &DefinitionIndex, indent: &str, nontrivial: bool) -> String {
    let mut out = String::new();
    let switch_indent = format!("{indent}    ");

    let _ = writeln!(out, "{indent}switch (this->_d) {{");

    for case in &u.cases {
        let has_default = case.labels.iter().any(|l| matches!(l, UnionLabel::Default));
        let value_labels: Vec<_> = case
            .labels
            .iter()
            .filter_map(|l| {
                if let UnionLabel::Value(v) = l {
                    Some(v.as_str())
                } else {
                    None
                }
            })
            .collect();

        // Emit case labels
        for label in &value_labels {
            let cpp_label = label_to_cpp(&u.discriminator, label);
            let _ = writeln!(out, "{indent}case {cpp_label}:");
        }
        if has_default {
            let _ = writeln!(out, "{indent}default:");
        }

        // Emit field encoding
        let field_expr = if nontrivial {
            format!("this->{}", case.field.name)
        } else {
            format!("this->_u.{}", case.field.name)
        };
        out.push_str(&emit_encode_union_field(
            &case.field,
            idx,
            &switch_indent,
            &field_expr,
        ));
        let _ = writeln!(out, "{switch_indent}break;");
    }

    let _ = writeln!(out, "{indent}}}");
    out
}

fn emit_decode_switch(u: &Union, idx: &DefinitionIndex, indent: &str, nontrivial: bool) -> String {
    let mut out = String::new();
    let switch_indent = format!("{indent}    ");

    let _ = writeln!(out, "{indent}switch (this->_d) {{");

    for case in &u.cases {
        let has_default = case.labels.iter().any(|l| matches!(l, UnionLabel::Default));
        let value_labels: Vec<_> = case
            .labels
            .iter()
            .filter_map(|l| {
                if let UnionLabel::Value(v) = l {
                    Some(v.as_str())
                } else {
                    None
                }
            })
            .collect();

        // Emit case labels
        for label in &value_labels {
            let cpp_label = label_to_cpp(&u.discriminator, label);
            let _ = writeln!(out, "{indent}case {cpp_label}:");
        }
        if has_default {
            let _ = writeln!(out, "{indent}default:");
        }

        // Emit field decoding
        let field_expr = if nontrivial {
            format!("this->{}", case.field.name)
        } else {
            format!("this->_u.{}", case.field.name)
        };
        out.push_str(&emit_decode_union_field(
            &case.field,
            idx,
            &switch_indent,
            &field_expr,
        ));
        let _ = writeln!(out, "{switch_indent}break;");
    }

    let _ = writeln!(out, "{indent}}}");
    out
}

/// Convert a union label value to C++ expression
fn label_to_cpp(disc: &IdlType, label: &str) -> String {
    let trimmed = label.trim();
    // If it's a numeric literal, return as-is
    if trimmed
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == '-')
    {
        return trimmed.to_string();
    }
    // For named types (enums), use the label directly (may be qualified)
    if let IdlType::Named(enum_name) = disc {
        // If label is already qualified, use it
        if trimmed.contains("::") {
            trimmed.to_string()
        } else {
            // Qualify with enum name
            let enum_ident = last_ident(enum_name);
            format!("{enum_ident}::{trimmed}")
        }
    } else {
        trimmed.to_string()
    }
}

fn emit_encode_union_field(
    field: &crate::ast::Field,
    idx: &DefinitionIndex,
    indent: &str,
    value_expr: &str,
) -> String {
    emit_encode_type(indent, &field.field_type, idx, value_expr, &field.name, 0)
}

fn emit_decode_union_field(
    field: &crate::ast::Field,
    idx: &DefinitionIndex,
    indent: &str,
    value_expr: &str,
) -> String {
    emit_decode_type(indent, &field.field_type, idx, value_expr, &field.name, 0)
}

// Inline encode/decode type emitters for union fields
// These are simplified versions that work with arbitrary value expressions

fn emit_encode_type(
    indent: &str,
    ty: &IdlType,
    idx: &DefinitionIndex,
    value_expr: &str,
    field_name: &str,
    depth: u32,
) -> String {
    match ty {
        IdlType::Primitive(p) => match p {
            PrimitiveType::String => encode_string(indent, value_expr),
            PrimitiveType::WString => encode_wstring(indent, value_expr),
            PrimitiveType::WChar => encode_wchar(indent, value_expr),
            PrimitiveType::Fixed { .. } => encode_fixed(indent, value_expr),
            PrimitiveType::Void => format!("{indent}// void: no encoding\n"),
            _ => primitive_scalar_layout(p).map_or_else(
                || format!("{indent}// unsupported primitive: {p:?}\n"),
                |layout| encode_scalar(indent, layout.0, layout.1, value_expr),
            ),
        },
        IdlType::Array { inner, size } => {
            encode_array(indent, inner, *size, idx, value_expr, field_name, depth)
        }
        IdlType::Sequence { inner, .. } => {
            encode_sequence(indent, inner, idx, value_expr, field_name, depth)
        }
        IdlType::Map { key, value, .. } => {
            encode_map(indent, key, value, idx, value_expr, field_name, depth)
        }
        IdlType::Named(nm) => {
            let type_ident = last_ident(nm);
            if idx.structs.contains_key(type_ident) || idx.unions.contains_key(type_ident) {
                format!(
                    "{indent}{{\n\
                     {indent}    int bytes = {value_expr}.encode_cdr2_le(dst + offset, len - offset);\n\
                     {indent}    if (bytes < 0) return -1;\n\
                     {indent}    offset += static_cast<std::size_t>(bytes);\n\
                     {indent}}}\n"
                )
            } else if idx.bitsets.contains_key(type_ident) || idx.bitmasks.contains_key(type_ident)
            {
                encode_scalar(
                    indent,
                    8,
                    8,
                    &format!("static_cast<std::uint64_t>({value_expr})"),
                )
            } else if idx.enums.contains_key(type_ident) {
                encode_scalar(
                    indent,
                    4,
                    4,
                    &format!("static_cast<std::int32_t>({value_expr})"),
                )
            } else if let Some(td) = idx.typedefs.get(type_ident) {
                emit_encode_type(indent, &td.base_type, idx, value_expr, field_name, depth)
            } else {
                format!("{indent}return -1; // unsupported named type `{type_ident}`\n")
            }
        }
    }
}

fn emit_decode_type(
    indent: &str,
    ty: &IdlType,
    idx: &DefinitionIndex,
    value_expr: &str,
    field_name: &str,
    depth: u32,
) -> String {
    match ty {
        IdlType::Primitive(p) => match p {
            PrimitiveType::String => decode_string(indent, value_expr),
            PrimitiveType::WString => decode_wstring(indent, value_expr),
            PrimitiveType::WChar => decode_wchar(indent, value_expr),
            PrimitiveType::Fixed { .. } => decode_fixed(indent, value_expr),
            PrimitiveType::Void => format!("{indent}// void: no decoding\n"),
            _ => primitive_scalar_layout(p).map_or_else(
                || format!("{indent}// unsupported primitive: {p:?}\n"),
                |layout| decode_scalar(indent, layout.0, layout.1, value_expr),
            ),
        },
        IdlType::Array { inner, size } => {
            decode_array(indent, inner, *size, idx, value_expr, field_name, depth)
        }
        IdlType::Sequence { inner, bound } => {
            decode_sequence(indent, inner, *bound, idx, value_expr, field_name, depth)
        }
        IdlType::Map { key, value, .. } => {
            decode_map(indent, key, value, idx, value_expr, field_name, depth)
        }
        IdlType::Named(nm) => {
            let type_ident = last_ident(nm);
            if idx.structs.contains_key(type_ident) || idx.unions.contains_key(type_ident) {
                format!(
                    "{indent}{{\n\
                     {indent}    int bytes = {value_expr}.decode_cdr2_le(src + offset, len - offset);\n\
                     {indent}    if (bytes < 0) return -1;\n\
                     {indent}    offset += static_cast<std::size_t>(bytes);\n\
                     {indent}}}\n"
                )
            } else if idx.bitsets.contains_key(type_ident) || idx.bitmasks.contains_key(type_ident)
            {
                let mut out = String::new();
                let _ = write!(
                    out,
                    "{indent}offset = cdr2::align_offset(offset, 8);\n\
                     {indent}if (!cdr2::can_read(len, offset, 8)) return -1;\n\
                     {indent}{{\n\
                     {indent}    std::uint64_t tmp;\n\
                     {indent}    std::memcpy(&tmp, src + offset, 8);\n\
                     {indent}    {value_expr} = static_cast<std::remove_reference_t<decltype({value_expr})>>(tmp);\n\
                     {indent}    offset += 8;\n\
                     {indent}}}\n"
                );
                out
            } else if idx.enums.contains_key(type_ident) {
                let mut out = String::new();
                let _ = write!(
                    out,
                    "{indent}offset = cdr2::align_offset(offset, 4);\n\
                     {indent}if (!cdr2::can_read(len, offset, 4)) return -1;\n\
                     {indent}{{\n\
                     {indent}    std::int32_t tmp;\n\
                     {indent}    std::memcpy(&tmp, src + offset, 4);\n\
                     {indent}    {value_expr} = static_cast<std::remove_reference_t<decltype({value_expr})>>(tmp);\n\
                     {indent}    offset += 4;\n\
                     {indent}}}\n"
                );
                out
            } else if let Some(td) = idx.typedefs.get(type_ident) {
                emit_decode_type(indent, &td.base_type, idx, value_expr, field_name, depth)
            } else {
                format!("{indent}return -1; // unsupported named type `{type_ident}`\n")
            }
        }
    }
}

/// Returns (alignment, size) for primitive types
#[allow(clippy::missing_const_for_fn)]
fn primitive_scalar_layout(prim: &crate::types::PrimitiveType) -> Option<(usize, usize)> {
    use crate::types::PrimitiveType;
    match prim {
        PrimitiveType::Octet
        | PrimitiveType::UInt8
        | PrimitiveType::Int8
        | PrimitiveType::Boolean
        | PrimitiveType::Char => Some((1, 1)),
        PrimitiveType::Short
        | PrimitiveType::Int16
        | PrimitiveType::UnsignedShort
        | PrimitiveType::UInt16 => Some((2, 2)),
        PrimitiveType::Long
        | PrimitiveType::Int32
        | PrimitiveType::UnsignedLong
        | PrimitiveType::UInt32
        | PrimitiveType::Float
        | PrimitiveType::WChar => Some((4, 4)),
        PrimitiveType::LongLong
        | PrimitiveType::Int64
        | PrimitiveType::UnsignedLongLong
        | PrimitiveType::UInt64
        | PrimitiveType::Double
        | PrimitiveType::LongDouble => Some((8, 8)),
        PrimitiveType::Fixed { .. } => Some((4, 16)),
        PrimitiveType::String | PrimitiveType::WString | PrimitiveType::Void => None,
    }
}

fn encode_scalar(indent: &str, align: usize, size: usize, value_expr: &str) -> String {
    if value_expr.contains("static_cast") {
        let type_name = match size {
            1 => "std::uint8_t",
            2 => "std::uint16_t",
            4 => "std::int32_t",
            _ => "std::uint64_t",
        };
        format!(
            "{indent}offset = cdr2::align_offset(offset, {align});\n\
             {indent}if (!cdr2::can_write(len, offset, {size})) return -1;\n\
             {indent}{{\n\
             {indent}    {type_name} tmp = {value_expr};\n\
             {indent}    std::memcpy(dst + offset, &tmp, {size});\n\
             {indent}}}\n\
             {indent}offset += {size};\n"
        )
    } else {
        format!(
            "{indent}offset = cdr2::align_offset(offset, {align});\n\
             {indent}if (!cdr2::can_write(len, offset, {size})) return -1;\n\
             {indent}std::memcpy(dst + offset, &({value_expr}), {size});\n\
             {indent}offset += {size};\n"
        )
    }
}

fn decode_scalar(indent: &str, align: usize, size: usize, value_expr: &str) -> String {
    format!(
        "{indent}offset = cdr2::align_offset(offset, {align});\n\
         {indent}if (!cdr2::can_read(len, offset, {size})) return -1;\n\
         {indent}std::memcpy(&({value_expr}), src + offset, {size});\n\
         {indent}offset += {size};\n"
    )
}

fn encode_string(indent: &str, value_expr: &str) -> String {
    format!(
        "{indent}offset = cdr2::align_offset(offset, 4);\n\
         {indent}{{\n\
         {indent}    std::uint32_t str_len = static_cast<std::uint32_t>({value_expr}.size() + 1);\n\
         {indent}    if (!cdr2::can_write(len, offset, 4 + str_len)) return -1;\n\
         {indent}    std::memcpy(dst + offset, &str_len, 4);\n\
         {indent}    offset += 4;\n\
         {indent}    std::memcpy(dst + offset, {value_expr}.c_str(), str_len);\n\
         {indent}    offset += str_len;\n\
         {indent}}}\n"
    )
}

fn decode_string(indent: &str, value_expr: &str) -> String {
    format!(
        "{indent}offset = cdr2::align_offset(offset, 4);\n\
         {indent}if (!cdr2::can_read(len, offset, 4)) return -1;\n\
         {indent}{{\n\
         {indent}    std::uint32_t str_len;\n\
         {indent}    std::memcpy(&str_len, src + offset, 4);\n\
         {indent}    offset += 4;\n\
         {indent}    if (str_len == 0 || !cdr2::can_read(len, offset, str_len)) return -1;\n\
         {indent}    {value_expr}.assign(reinterpret_cast<const char*>(src + offset), str_len - 1);\n\
         {indent}    offset += str_len;\n\
         {indent}}}\n"
    )
}

fn encode_wstring(indent: &str, value_expr: &str) -> String {
    format!(
        "{indent}offset = cdr2::align_offset(offset, 4);\n\
         {indent}{{\n\
         {indent}    std::uint32_t byte_len = static_cast<std::uint32_t>(({value_expr}.size() + 1) * sizeof(wchar_t));\n\
         {indent}    if (!cdr2::can_write(len, offset, 4 + byte_len)) return -1;\n\
         {indent}    std::memcpy(dst + offset, &byte_len, 4);\n\
         {indent}    offset += 4;\n\
         {indent}    std::memcpy(dst + offset, {value_expr}.c_str(), byte_len);\n\
         {indent}    offset += byte_len;\n\
         {indent}}}\n"
    )
}

fn decode_wstring(indent: &str, value_expr: &str) -> String {
    format!(
        "{indent}offset = cdr2::align_offset(offset, 4);\n\
         {indent}if (!cdr2::can_read(len, offset, 4)) return -1;\n\
         {indent}{{\n\
         {indent}    std::uint32_t byte_len;\n\
         {indent}    std::memcpy(&byte_len, src + offset, 4);\n\
         {indent}    offset += 4;\n\
         {indent}    if (byte_len < sizeof(wchar_t) || !cdr2::can_read(len, offset, byte_len)) return -1;\n\
         {indent}    std::size_t char_count = (byte_len / sizeof(wchar_t)) - 1;\n\
         {indent}    {value_expr}.assign(reinterpret_cast<const wchar_t*>(src + offset), char_count);\n\
         {indent}    offset += byte_len;\n\
         {indent}}}\n"
    )
}

fn encode_wchar(indent: &str, value_expr: &str) -> String {
    format!(
        "{indent}offset = cdr2::align_offset(offset, 4);\n\
         {indent}if (!cdr2::can_write(len, offset, 4)) return -1;\n\
         {indent}{{\n\
         {indent}    std::uint32_t wc = static_cast<std::uint32_t>({value_expr});\n\
         {indent}    if (wc > 0x10FFFF) return -1;\n\
         {indent}    std::memcpy(dst + offset, &wc, 4);\n\
         {indent}    offset += 4;\n\
         {indent}}}\n"
    )
}

fn decode_wchar(indent: &str, value_expr: &str) -> String {
    format!(
        "{indent}offset = cdr2::align_offset(offset, 4);\n\
         {indent}if (!cdr2::can_read(len, offset, 4)) return -1;\n\
         {indent}{{\n\
         {indent}    std::uint32_t wc;\n\
         {indent}    std::memcpy(&wc, src + offset, 4);\n\
         {indent}    if (wc > 0x10FFFF) return -1;\n\
         {indent}    {value_expr} = static_cast<wchar_t>(wc);\n\
         {indent}    offset += 4;\n\
         {indent}}}\n"
    )
}

fn encode_fixed(indent: &str, value_expr: &str) -> String {
    format!(
        "{indent}offset = cdr2::align_offset(offset, 4);\n\
         {indent}if (!cdr2::can_write(len, offset, 16)) return -1;\n\
         {indent}{{\n\
         {indent}    auto bytes = {value_expr}.to_le_bytes();\n\
         {indent}    std::memcpy(dst + offset, bytes.data(), 16);\n\
         {indent}    offset += 16;\n\
         {indent}}}\n"
    )
}

fn decode_fixed(indent: &str, value_expr: &str) -> String {
    format!(
        "{indent}offset = cdr2::align_offset(offset, 4);\n\
         {indent}if (!cdr2::can_read(len, offset, 16)) return -1;\n\
         {indent}{{\n\
         {indent}    std::array<std::uint8_t, 16> bytes;\n\
         {indent}    std::memcpy(bytes.data(), src + offset, 16);\n\
         {indent}    {value_expr} = decltype({value_expr})::from_le_bytes(bytes);\n\
         {indent}    offset += 16;\n\
         {indent}}}\n"
    )
}

fn encode_array(
    indent: &str,
    inner: &IdlType,
    size: u32,
    idx: &DefinitionIndex,
    value_expr: &str,
    field_name: &str,
    depth: u32,
) -> String {
    let mut out = String::new();
    let var = loop_var(depth);
    let align = idx.align_of(inner);
    let _ = writeln!(out, "{indent}offset = cdr2::align_offset(offset, {align});");
    let _ = writeln!(out, "{indent}for (std::size_t {var} = 0; {var} < {size}; ++{var}) {{");
    let next_indent = format!("{indent}    ");
    let element_value = format!("{value_expr}[{var}]");
    out.push_str(&emit_encode_type(
        &next_indent,
        inner,
        idx,
        &element_value,
        &format!("{field_name}_elem"),
        depth + 1,
    ));
    let _ = writeln!(out, "{indent}}}");
    out
}

fn decode_array(
    indent: &str,
    inner: &IdlType,
    size: u32,
    idx: &DefinitionIndex,
    value_expr: &str,
    field_name: &str,
    depth: u32,
) -> String {
    let mut out = String::new();
    let var = loop_var(depth);
    let align = idx.align_of(inner);
    let _ = writeln!(out, "{indent}offset = cdr2::align_offset(offset, {align});");
    let _ = writeln!(out, "{indent}for (std::size_t {var} = 0; {var} < {size}; ++{var}) {{");
    let next_indent = format!("{indent}    ");
    let element_value = format!("{value_expr}[{var}]");
    out.push_str(&emit_decode_type(
        &next_indent,
        inner,
        idx,
        &element_value,
        &format!("{field_name}_elem"),
        depth + 1,
    ));
    let _ = writeln!(out, "{indent}}}");
    out
}

fn encode_sequence(
    indent: &str,
    inner: &IdlType,
    idx: &DefinitionIndex,
    value_expr: &str,
    field_name: &str,
    depth: u32,
) -> String {
    let mut out = String::new();
    let var = loop_var(depth);
    let _ = writeln!(out, "{indent}offset = cdr2::align_offset(offset, 4);");
    let _ = write!(
        out,
        "{indent}{{\n\
         {indent}    std::uint32_t seq_len = static_cast<std::uint32_t>({value_expr}.size());\n\
         {indent}    if (!cdr2::can_write(len, offset, 4)) return -1;\n\
         {indent}    std::memcpy(dst + offset, &seq_len, 4);\n\
         {indent}    offset += 4;\n\
         {indent}}}\n"
    );
    let _ = writeln!(
        out,
        "{indent}for (std::size_t {var} = 0; {var} < {value_expr}.size(); ++{var}) {{"
    );
    let next_indent = format!("{indent}    ");
    let element_value = format!("{value_expr}[{var}]");
    out.push_str(&emit_encode_type(
        &next_indent,
        inner,
        idx,
        &element_value,
        &format!("{field_name}_elem"),
        depth + 1,
    ));
    let _ = writeln!(out, "{indent}}}");
    out
}

fn decode_sequence(
    indent: &str,
    inner: &IdlType,
    bound: Option<u32>,
    idx: &DefinitionIndex,
    value_expr: &str,
    field_name: &str,
    depth: u32,
) -> String {
    let mut out = String::new();
    let var = loop_var(depth);

    if let Some(max_size) = bound {
        // Bounded sequence: std::array
        let _ = write!(
            out,
            "{indent}offset = cdr2::align_offset(offset, 4);\n\
             {indent}if (!cdr2::can_read(len, offset, 4)) return -1;\n\
             {indent}{{\n\
             {indent}    std::uint32_t seq_len;\n\
             {indent}    std::memcpy(&seq_len, src + offset, 4);\n\
             {indent}    offset += 4;\n\
             {indent}    if (seq_len > {max_size}) return -1;\n\
             {indent}    for (std::size_t {var} = 0; {var} < seq_len && {var} < {max_size}; ++{var}) {{\n"
        );
        let inner_indent = format!("{indent}        ");
        let element_value = format!("{value_expr}[{var}]");
        out.push_str(&emit_decode_type(
            &inner_indent,
            inner,
            idx,
            &element_value,
            &format!("{field_name}_elem"),
            depth + 1,
        ));
        let _ = write!(
            out,
            "{indent}    }}\n\
             {indent}}}\n"
        );
    } else {
        // Unbounded sequence: std::vector
        let _ = write!(
            out,
            "{indent}offset = cdr2::align_offset(offset, 4);\n\
             {indent}if (!cdr2::can_read(len, offset, 4)) return -1;\n\
             {indent}{{\n\
             {indent}    std::uint32_t seq_len;\n\
             {indent}    std::memcpy(&seq_len, src + offset, 4);\n\
             {indent}    offset += 4;\n\
             {indent}    {value_expr}.resize(seq_len);\n\
             {indent}}}\n"
        );
        let _ = writeln!(
            out,
            "{indent}for (std::size_t {var} = 0; {var} < {value_expr}.size(); ++{var}) {{"
        );
        let next_indent = format!("{indent}    ");
        let element_value = format!("{value_expr}[{var}]");
        out.push_str(&emit_decode_type(
            &next_indent,
            inner,
            idx,
            &element_value,
            &format!("{field_name}_elem"),
            depth + 1,
        ));
        let _ = writeln!(out, "{indent}}}");
    }
    out
}

fn encode_map(
    indent: &str,
    key: &IdlType,
    value: &IdlType,
    idx: &DefinitionIndex,
    value_expr: &str,
    field_name: &str,
    depth: u32,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{indent}offset = cdr2::align_offset(offset, 4);");
    let _ = write!(
        out,
        "{indent}{{\n\
         {indent}    std::uint32_t map_len = static_cast<std::uint32_t>({value_expr}.size());\n\
         {indent}    if (!cdr2::can_write(len, offset, 4)) return -1;\n\
         {indent}    std::memcpy(dst + offset, &map_len, 4);\n\
         {indent}    offset += 4;\n\
         {indent}}}\n"
    );
    let _ = writeln!(out, "{indent}for (const auto& kv : {value_expr}) {{");
    let next_indent = format!("{indent}    ");
    out.push_str(&emit_encode_type(
        &next_indent,
        key,
        idx,
        "kv.first",
        &format!("{field_name}_key"),
        depth,
    ));
    out.push_str(&emit_encode_type(
        &next_indent,
        value,
        idx,
        "kv.second",
        &format!("{field_name}_value"),
        depth,
    ));
    let _ = writeln!(out, "{indent}}}");
    out
}

fn decode_map(
    indent: &str,
    key: &IdlType,
    value: &IdlType,
    idx: &DefinitionIndex,
    value_expr: &str,
    field_name: &str,
    depth: u32,
) -> String {
    let key_cpp = type_to_cpp(key);
    let value_cpp = type_to_cpp(value);
    let mut out = String::new();
    let var = loop_var(depth);
    let _ = write!(
        out,
        "{indent}offset = cdr2::align_offset(offset, 4);\n\
         {indent}if (!cdr2::can_read(len, offset, 4)) return -1;\n\
         {indent}{{\n\
         {indent}    std::uint32_t map_len;\n\
         {indent}    std::memcpy(&map_len, src + offset, 4);\n\
         {indent}    offset += 4;\n\
         {indent}    {value_expr}.clear();\n\
         {indent}    for (std::uint32_t {var} = 0; {var} < map_len; ++{var}) {{\n"
    );
    let inner_indent = format!("{indent}        ");
    let _ = write!(
        out,
        "{inner_indent}{key_cpp} key{{}};\n\
         {inner_indent}{value_cpp} val{{}};\n"
    );
    out.push_str(&emit_decode_type(
        &inner_indent,
        key,
        idx,
        "key",
        &format!("{field_name}_key"),
        depth,
    ));
    out.push_str(&emit_decode_type(
        &inner_indent,
        value,
        idx,
        "val",
        &format!("{field_name}_value"),
        depth,
    ));
    let _ = write!(
        out,
        "{inner_indent}{value_expr}[key] = std::move(val);\n\
         {indent}    }}\n\
         {indent}}}\n"
    );
    out
}
