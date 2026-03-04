// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! CDR2 decode method generation for C++.
//!
//! Emits inline `decode_cdr2_le()` methods for struct types.

use super::super::super::keywords::cpp_ident;
use super::super::helpers::last_ident_owned;
use super::super::index::DefinitionIndex;
use crate::ast::Field;
use crate::types::{IdlType, PrimitiveType};
use std::fmt::Write;

pub(super) fn emit_decode_field_compat(
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
        // For optional fields, read presence flag first, then conditionally decode value
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{indent}// Optional field '{}': read presence flag",
            f.name
        );
        let _ = writeln!(
            out,
            "{indent}if (!cdr2::can_read(len, offset, 1)) return -1;"
        );
        let _ = writeln!(out, "{indent}{{");
        let _ = writeln!(
            out,
            "{indent}    bool has_{name} = (src[offset++] != 0);",
            name = escaped
        );
        let _ = writeln!(out, "{indent}    if (has_{name}) {{", name = escaped);

        // Create a temporary to decode into, then assign to optional
        let inner_type = super::super::helpers::type_to_cpp(&f.field_type);
        let _ = writeln!(out, "{indent}        {inner_type} tmp_{}{{}};\n", escaped);
        // Decode the value into temp variable
        let temp_expr = format!("tmp_{}", escaped);
        out.push_str(&emit_decode_type(
            &format!("{indent}        "),
            &f.field_type,
            idx,
            &temp_expr,
            &escaped,
        ));
        let _ = writeln!(
            out,
            "{indent}        {base_expr} = std::move(tmp_{name});",
            name = escaped
        );
        let _ = writeln!(out, "{indent}    }} else {{");
        let _ = writeln!(out, "{indent}        {base_expr} = std::nullopt;");
        let _ = writeln!(out, "{indent}    }}");
        let _ = writeln!(out, "{indent}}}");
        out
    } else {
        emit_decode_type(indent, &f.field_type, idx, &base_expr, &escaped)
    }
}

/// For MUTABLE structs, we don't emit presence flags since EMHEADER handles optional fields
/// by simply omitting them when absent. This is a direct decode without optional wrapping.
pub(super) fn emit_decode_type_for_mutable(
    indent: &str,
    ty: &IdlType,
    idx: &DefinitionIndex,
    value_expr: &str,
    field_name: &str,
) -> String {
    emit_decode_type(indent, ty, idx, value_expr, field_name)
}

fn emit_decode_type(
    indent: &str,
    ty: &IdlType,
    idx: &DefinitionIndex,
    value_expr: &str,
    field_name: &str,
) -> String {
    match ty {
        IdlType::Primitive(p) => match p {
            PrimitiveType::String => decode_string(indent, value_expr),
            PrimitiveType::WString => decode_wstring(indent, value_expr),
            PrimitiveType::WChar => decode_wchar(indent, value_expr),
            PrimitiveType::Fixed { .. } => decode_fixed(indent, value_expr),
            PrimitiveType::Void => format!("{indent}// void: no decoding\n"),
            _ => super::primitive_scalar_layout(p).map_or_else(
                || format!("{indent}// unsupported primitive: {:?}\n", p),
                |layout| decode_scalar(indent, layout.align, layout.width, value_expr),
            ),
        },
        IdlType::Array { inner, size } => {
            decode_array(indent, inner, *size, idx, value_expr, field_name)
        }
        IdlType::Sequence { inner, bound } => {
            decode_sequence(indent, inner, *bound, idx, value_expr, field_name)
        }
        IdlType::Map { key, value, .. } => {
            decode_map(indent, key, value, idx, value_expr, field_name)
        }
        IdlType::Named(nm) => {
            let type_ident = last_ident_owned(nm);
            if idx.structs.contains_key(&type_ident) {
                format!(
                    "{indent}{{\n\
                     {indent}    int bytes = {value}.decode_cdr2_le(src + offset, len - offset);\n\
                     {indent}    if (bytes < 0) return -1;\n\
                     {indent}    offset += static_cast<std::size_t>(bytes);\n\
                     {indent}}}\n",
                    indent = indent,
                    value = value_expr
                )
            } else if idx.bitsets.contains_key(&type_ident) {
                // Bitsets are structs with bitfields - use from_uint64() method
                let mut out = String::new();
                let _ = write!(
                    out,
                    "{indent}offset = cdr2::align_offset(offset, 8);\n\
                     {indent}if (!cdr2::can_read(len, offset, 8)) return -1;\n\
                     {indent}{{\n\
                     {indent}    std::uint64_t tmp;\n\
                     {indent}    std::memcpy(&tmp, src + offset, 8);\n\
                     {indent}    ({value}).from_uint64(tmp);\n\
                     {indent}    offset += 8;\n\
                     {indent}}}\n",
                    indent = indent,
                    value = value_expr
                );
                out
            } else if idx.bitmasks.contains_key(&type_ident) {
                // Bitmasks are enum class : uint64_t - use static_cast
                let mut out = String::new();
                let _ = write!(
                    out,
                    "{indent}offset = cdr2::align_offset(offset, 8);\n\
                     {indent}if (!cdr2::can_read(len, offset, 8)) return -1;\n\
                     {indent}{{\n\
                     {indent}    std::uint64_t tmp;\n\
                     {indent}    std::memcpy(&tmp, src + offset, 8);\n\
                     {indent}    {value} = static_cast<std::remove_reference_t<decltype({value})>>(tmp);\n\
                     {indent}    offset += 8;\n\
                     {indent}}}\n",
                    indent = indent,
                    value = value_expr
                );
                out
            } else if idx.enums.contains_key(&type_ident) {
                let mut out = String::new();
                let _ = write!(
                    out,
                    "{indent}offset = cdr2::align_offset(offset, 4);\n\
                     {indent}if (!cdr2::can_read(len, offset, 4)) return -1;\n\
                     {indent}{{\n\
                     {indent}    std::int32_t tmp;\n\
                     {indent}    std::memcpy(&tmp, src + offset, 4);\n\
                     {indent}    {value} = static_cast<std::remove_reference_t<decltype({value})>>(tmp);\n\
                     {indent}    offset += 4;\n\
                     {indent}}}\n",
                    indent = indent,
                    value = value_expr
                );
                out
            } else if let Some(td) = idx.typedefs.get(&type_ident) {
                emit_decode_type(indent, &td.base_type, idx, value_expr, field_name)
            } else {
                format!(
                    "{indent}return -1; // unsupported named type `{}`\n",
                    type_ident
                )
            }
        }
    }
}

fn decode_scalar(indent: &str, align: usize, size: usize, value_expr: &str) -> String {
    format!(
        "{indent}offset = cdr2::align_offset(offset, {align});\n\
         {indent}if (!cdr2::can_read(len, offset, {size})) return -1;\n\
         {indent}std::memcpy(&({value}), src + offset, {size});\n\
         {indent}offset += {size};\n",
        indent = indent,
        align = align,
        size = size,
        value = value_expr
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
         {indent}    {value}.assign(reinterpret_cast<const char*>(src + offset), str_len - 1);\n\
         {indent}    offset += str_len;\n\
         {indent}}}\n",
        indent = indent,
        value = value_expr
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
         {indent}    {value}.assign(reinterpret_cast<const wchar_t*>(src + offset), char_count);\n\
         {indent}    offset += byte_len;\n\
         {indent}}}\n",
        indent = indent,
        value = value_expr
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
         {indent}    {value} = static_cast<wchar_t>(wc);\n\
         {indent}    offset += 4;\n\
         {indent}}}\n",
        indent = indent,
        value = value_expr
    )
}

fn decode_fixed(indent: &str, value_expr: &str) -> String {
    format!(
        "{indent}offset = cdr2::align_offset(offset, 4);\n\
         {indent}if (!cdr2::can_read(len, offset, 16)) return -1;\n\
         {indent}{{\n\
         {indent}    std::array<std::uint8_t, 16> bytes;\n\
         {indent}    std::memcpy(bytes.data(), src + offset, 16);\n\
         {indent}    {value} = decltype({value})::from_le_bytes(bytes);\n\
         {indent}    offset += 16;\n\
         {indent}}}\n",
        indent = indent,
        value = value_expr
    )
}

fn decode_array(
    indent: &str,
    inner: &IdlType,
    size: u32,
    idx: &DefinitionIndex,
    value_expr: &str,
    field_name: &str,
) -> String {
    let mut out = String::new();
    let align = idx.align_of(inner);
    let _ = writeln!(out, "{indent}offset = cdr2::align_offset(offset, {align});");
    let _ = writeln!(out, "{indent}for (std::size_t i = 0; i < {size}; ++i) {{");
    let next_indent = format!("{indent}    ");
    let element_value = format!("{}[i]", value_expr);
    out.push_str(&emit_decode_type(
        &next_indent,
        inner,
        idx,
        &element_value,
        &format!("{field_name}_elem"),
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
) -> String {
    let mut out = String::new();

    // Bounded sequences use std::array (fixed size), unbounded use std::vector (dynamic)
    if let Some(max_size) = bound {
        // Bounded sequence: std::array - copy directly without resize
        let _ = write!(
            out,
            "{indent}offset = cdr2::align_offset(offset, 4);\n\
             {indent}if (!cdr2::can_read(len, offset, 4)) return -1;\n\
             {indent}{{\n\
             {indent}    std::uint32_t seq_len;\n\
             {indent}    std::memcpy(&seq_len, src + offset, 4);\n\
             {indent}    offset += 4;\n\
             {indent}    if (seq_len > {max_size}) return -1; // Exceeds bounded size\n\
             {indent}    for (std::size_t i = 0; i < seq_len && i < {max_size}; ++i) {{\n",
            indent = indent,
            max_size = max_size
        );
        let inner_indent = format!("{indent}        ");
        let element_value = format!("{}[i]", value_expr);
        out.push_str(&emit_decode_type(
            &inner_indent,
            inner,
            idx,
            &element_value,
            &format!("{field_name}_elem"),
        ));
        let _ = write!(
            out,
            "{indent}    }}\n\
             {indent}}}\n",
            indent = indent
        );
    } else {
        // Unbounded sequence: std::vector - resize dynamically
        let _ = write!(
            out,
            "{indent}offset = cdr2::align_offset(offset, 4);\n\
             {indent}if (!cdr2::can_read(len, offset, 4)) return -1;\n\
             {indent}{{\n\
             {indent}    std::uint32_t seq_len;\n\
             {indent}    std::memcpy(&seq_len, src + offset, 4);\n\
             {indent}    offset += 4;\n\
             {indent}    {value}.resize(seq_len);\n\
             {indent}}}\n",
            indent = indent,
            value = value_expr
        );
        let _ = writeln!(
            out,
            "{indent}for (std::size_t i = 0; i < {value}.size(); ++i) {{",
            indent = indent,
            value = value_expr
        );
        let next_indent = format!("{indent}    ");
        let element_value = format!("{}[i]", value_expr);
        out.push_str(&emit_decode_type(
            &next_indent,
            inner,
            idx,
            &element_value,
            &format!("{field_name}_elem"),
        ));
        let _ = writeln!(out, "{indent}}}");
    }
    out
}

fn decode_map(
    indent: &str,
    key: &IdlType,
    value: &IdlType,
    idx: &DefinitionIndex,
    value_expr: &str,
    field_name: &str,
) -> String {
    let key_cpp = super::super::helpers::type_to_cpp(key);
    let value_cpp = super::super::helpers::type_to_cpp(value);
    let mut out = String::new();
    let _ = write!(
        out,
        "{indent}offset = cdr2::align_offset(offset, 4);\n\
         {indent}if (!cdr2::can_read(len, offset, 4)) return -1;\n\
         {indent}{{\n\
         {indent}    std::uint32_t map_len;\n\
         {indent}    std::memcpy(&map_len, src + offset, 4);\n\
         {indent}    offset += 4;\n\
         {indent}    {value_expr}.clear();\n\
         {indent}    for (std::uint32_t i = 0; i < map_len; ++i) {{\n",
        indent = indent,
        value_expr = value_expr
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
    ));
    out.push_str(&emit_decode_type(
        &inner_indent,
        value,
        idx,
        "val",
        &format!("{field_name}_value"),
    ));
    let _ = write!(
        out,
        "{inner_indent}{value_expr}[key] = std::move(val);\n\
         {indent}    }}\n\
         {indent}}}\n",
        inner_indent = inner_indent,
        indent = indent,
        value_expr = value_expr
    );
    out
}
