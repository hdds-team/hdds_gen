// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! CDR2 encode method generation for C++.
//!
//! Emits inline `encode_cdr2_le()` methods for struct types.

use super::super::super::keywords::cpp_ident;
use super::super::helpers::last_ident_owned;
use super::super::index::DefinitionIndex;
use crate::ast::Field;
use crate::types::{IdlType, PrimitiveType};
use std::fmt::Write;

pub(super) fn emit_encode_field_compat(
    f: &Field,
    idx: &DefinitionIndex,
    indent: &str,
    fastdds_compat: bool,
) -> String {
    let escaped = cpp_ident(&f.name);
    let base_expr = if fastdds_compat {
        format!("this->m_{}", escaped)
    } else {
        format!("this->{}", escaped)
    };

    if f.is_optional() {
        // For optional fields, emit presence flag + conditional value encoding
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{indent}// Optional field '{}': write presence flag",
            f.name
        );
        let _ = writeln!(
            out,
            "{indent}if (!cdr2::can_write(len, offset, 1)) return -1;"
        );
        let _ = writeln!(
            out,
            "{indent}dst[offset++] = {base_expr}.has_value() ? 0x01 : 0x00;"
        );
        let _ = writeln!(out, "{indent}if ({base_expr}.has_value()) {{");
        // Encode the value with dereferenced optional
        let value_expr = format!("(*{base_expr})");
        out.push_str(&emit_encode_type(
            &format!("{indent}    "),
            &f.field_type,
            idx,
            &value_expr,
            &escaped,
            0,
        ));
        let _ = writeln!(out, "{indent}}}");
        out
    } else {
        emit_encode_type(indent, &f.field_type, idx, &base_expr, &escaped, 0)
    }
}

/// For MUTABLE structs, we don't emit presence flags since EMHEADER handles optional fields
/// by simply omitting them when absent.
pub(super) fn emit_encode_type_for_mutable(
    indent: &str,
    ty: &IdlType,
    idx: &DefinitionIndex,
    value_expr: &str,
    field_name: &str,
) -> String {
    emit_encode_type(indent, ty, idx, value_expr, field_name, 0)
}

fn loop_var(depth: u32) -> &'static str {
    const VARS: &[&str] = &["i", "j", "k", "l", "m", "n"];
    VARS.get(depth as usize).unwrap_or(&"n")
}

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
            _ => super::primitive_scalar_layout(p).map_or_else(
                || format!("{indent}// unsupported primitive: {:?}\n", p),
                |layout| encode_scalar(indent, layout.align, layout.width, value_expr),
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
            let type_ident = last_ident_owned(nm);
            if idx.structs.contains_key(&type_ident) || idx.unions.contains_key(&type_ident) {
                // F01-spec-correct: propagate global offset via `_at` API
                // instead of slicing the destination buffer (which loses the
                // outer alignment context inside the callee).
                format!(
                    "{indent}{{\n\
                     {indent}    int err = {value}.encode_cdr2_le_at(dst, len, offset);\n\
                     {indent}    if (err) return err;\n\
                     {indent}}}\n",
                    indent = indent,
                    value = value_expr
                )
            } else if idx.bitsets.contains_key(&type_ident) {
                // Bitsets are structs with bitfields - use to_uint64() method.
                // XCDR2 §7.4.3.4.1 Tab.15: 8-byte primitives align on 4 (cap-4).
                encode_scalar(indent, 4, 8, &format!("({}).to_uint64()", value_expr))
            } else if idx.bitmasks.contains_key(&type_ident) {
                // Bitmasks are enum class : uint64_t - use static_cast.
                // XCDR2 §7.4.3.4.1 Tab.15: 8-byte primitives align on 4 (cap-4).
                encode_scalar(
                    indent,
                    4,
                    8,
                    &format!("static_cast<std::uint64_t>({})", value_expr),
                )
            } else if idx.enums.contains_key(&type_ident) {
                encode_scalar(
                    indent,
                    4,
                    4,
                    &format!("static_cast<std::int32_t>({})", value_expr),
                )
            } else if let Some(td) = idx.typedefs.get(&type_ident) {
                emit_encode_type(indent, &td.base_type, idx, value_expr, field_name, depth)
            } else {
                format!(
                    "{indent}return -1; // unsupported named type `{}`\n",
                    type_ident
                )
            }
        }
    }
}

fn encode_scalar(indent: &str, align: usize, size: usize, value_expr: &str) -> String {
    // Check if value_expr is a cast expression (contains static_cast) - need temp variable
    // to avoid taking address of rvalue
    if value_expr.contains("static_cast") || value_expr.ends_with(')') {
        let type_name = match size {
            1 => "std::uint8_t",
            2 => "std::uint16_t",
            4 => "std::int32_t",
            // 8 and any other size default to uint64_t
            _ => "std::uint64_t",
        };
        format!(
            "{indent}if (!cdr2::pad_to_align(dst, offset, len, {align})) return -1;\n\
             {indent}if (!cdr2::can_write(len, offset, {size})) return -1;\n\
             {indent}{{\n\
             {indent}    {type_name} tmp = {value};\n\
             {indent}    std::memcpy(dst + offset, &tmp, {size});\n\
             {indent}}}\n\
             {indent}offset += {size};\n",
            indent = indent,
            align = align,
            size = size,
            type_name = type_name,
            value = value_expr
        )
    } else {
        format!(
            "{indent}if (!cdr2::pad_to_align(dst, offset, len, {align})) return -1;\n\
             {indent}if (!cdr2::can_write(len, offset, {size})) return -1;\n\
             {indent}std::memcpy(dst + offset, &({value}), {size});\n\
             {indent}offset += {size};\n",
            indent = indent,
            align = align,
            size = size,
            value = value_expr
        )
    }
}

fn encode_string(indent: &str, value_expr: &str) -> String {
    format!(
        "{indent}if (!cdr2::pad_to_align(dst, offset, len, 4)) return -1;\n\
         {indent}{{\n\
         {indent}    std::uint32_t str_len = static_cast<std::uint32_t>({value}.size() + 1);\n\
         {indent}    if (!cdr2::can_write(len, offset, 4 + str_len)) return -1;\n\
         {indent}    std::memcpy(dst + offset, &str_len, 4);\n\
         {indent}    offset += 4;\n\
         {indent}    std::memcpy(dst + offset, {value}.c_str(), str_len);\n\
         {indent}    offset += str_len;\n\
         {indent}}}\n",
        indent = indent,
        value = value_expr
    )
}

fn encode_wstring(indent: &str, value_expr: &str) -> String {
    format!(
        "{indent}if (!cdr2::pad_to_align(dst, offset, len, 4)) return -1;\n\
         {indent}{{\n\
         {indent}    std::uint32_t byte_len = static_cast<std::uint32_t>(({value}.size() + 1) * sizeof(wchar_t));\n\
         {indent}    if (!cdr2::can_write(len, offset, 4 + byte_len)) return -1;\n\
         {indent}    std::memcpy(dst + offset, &byte_len, 4);\n\
         {indent}    offset += 4;\n\
         {indent}    std::memcpy(dst + offset, {value}.c_str(), byte_len);\n\
         {indent}    offset += byte_len;\n\
         {indent}}}\n",
        indent = indent,
        value = value_expr
    )
}

fn encode_wchar(indent: &str, value_expr: &str) -> String {
    format!(
        "{indent}if (!cdr2::pad_to_align(dst, offset, len, 4)) return -1;\n\
         {indent}if (!cdr2::can_write(len, offset, 4)) return -1;\n\
         {indent}{{\n\
         {indent}    std::uint32_t wc = static_cast<std::uint32_t>({value});\n\
         {indent}    if (wc > 0x10FFFF) return -1;\n\
         {indent}    std::memcpy(dst + offset, &wc, 4);\n\
         {indent}    offset += 4;\n\
         {indent}}}\n",
        indent = indent,
        value = value_expr
    )
}

fn encode_fixed(indent: &str, value_expr: &str) -> String {
    format!(
        "{indent}if (!cdr2::pad_to_align(dst, offset, len, 4)) return -1;\n\
         {indent}if (!cdr2::can_write(len, offset, 16)) return -1;\n\
         {indent}{{\n\
         {indent}    auto bytes = {value}.to_le_bytes();\n\
         {indent}    std::memcpy(dst + offset, bytes.data(), 16);\n\
         {indent}    offset += 16;\n\
         {indent}}}\n",
        indent = indent,
        value = value_expr
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
    let var = loop_var(depth);
    let mut out = String::new();
    let align = idx.align_of(inner);
    let _ = writeln!(
        out,
        "{indent}if (!cdr2::pad_to_align(dst, offset, len, {align})) return -1;"
    );
    let _ = writeln!(
        out,
        "{indent}for (std::size_t {var} = 0; {var} < {size}; ++{var}) {{"
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

fn encode_sequence(
    indent: &str,
    inner: &IdlType,
    idx: &DefinitionIndex,
    value_expr: &str,
    field_name: &str,
    depth: u32,
) -> String {
    let var = loop_var(depth);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{indent}if (!cdr2::pad_to_align(dst, offset, len, 4)) return -1;"
    );
    let _ = write!(
        out,
        "{indent}{{\n\
         {indent}    std::uint32_t seq_len = static_cast<std::uint32_t>({value}.size());\n\
         {indent}    if (!cdr2::can_write(len, offset, 4)) return -1;\n\
         {indent}    std::memcpy(dst + offset, &seq_len, 4);\n\
         {indent}    offset += 4;\n\
         {indent}}}\n",
        indent = indent,
        value = value_expr
    );
    let _ = writeln!(
        out,
        "{indent}for (std::size_t {var} = 0; {var} < {value}.size(); ++{var}) {{",
        indent = indent,
        var = var,
        value = value_expr
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
    let _ = writeln!(
        out,
        "{indent}if (!cdr2::pad_to_align(dst, offset, len, 4)) return -1;"
    );
    let _ = write!(
        out,
        "{indent}{{\n\
         {indent}    std::uint32_t map_len = static_cast<std::uint32_t>({value}.size());\n\
         {indent}    if (!cdr2::can_write(len, offset, 4)) return -1;\n\
         {indent}    std::memcpy(dst + offset, &map_len, 4);\n\
         {indent}    offset += 4;\n\
         {indent}}}\n",
        indent = indent,
        value = value_expr
    );
    let _ = writeln!(
        out,
        "{indent}for (const auto& kv : {value}) {{",
        indent = indent,
        value = value_expr
    );
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
