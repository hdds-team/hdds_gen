// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Maximum serialized size computation for C.
//!
//! Generates `max_size` functions that compute worst-case buffer requirements.

use super::super::super::keywords::c_ident;
use super::super::{
    helpers::{c_name, last_ident, last_ident_owned},
    index::DefinitionIndex,
};
use crate::ast::Field;
use crate::types::{IdlType, PrimitiveType};
use std::fmt::Write;

pub(super) fn emit_max_field(f: &Field, idx: &DefinitionIndex, parent: &str) -> String {
    let escaped = c_ident(&f.name);
    let value_expr = format!("{}->{}", parent, escaped);
    let ptr_expr = format!("&({})", value_expr);
    let mut out = String::new();
    if f.is_optional() {
        out.push_str("    offset += 1; /* optional presence flag */\n");
    }
    out.push_str(&emit_max_type(
        "    ",
        &f.field_type,
        idx,
        &value_expr,
        &ptr_expr,
        &escaped,
    ));
    out
}

pub(super) fn label_to_c(discr: &IdlType, label: &str) -> String {
    let trimmed = label.trim();
    if trimmed.chars().any(|c| c.is_ascii_digit()) {
        return trimmed.to_string();
    }
    let ty = match discr {
        IdlType::Named(n) => last_ident(n).to_string(),
        _ => String::new(),
    };
    let ty_up: String = ty.chars().map(|c| c.to_ascii_uppercase()).collect();
    let variant = last_ident(trimmed);
    let var_up: String = variant.chars().map(|c| c.to_ascii_uppercase()).collect();
    if ty_up.is_empty() {
        var_up
    } else {
        format!("{}_{}", ty_up, var_up)
    }
}

fn emit_max_type(
    indent: &str,
    ty: &IdlType,
    idx: &DefinitionIndex,
    value_expr: &str,
    ptr_expr: &str,
    field_name: &str,
) -> String {
    match ty {
        IdlType::Primitive(p) => match p {
            PrimitiveType::String => format!(
                "{indent}offset = cdr_align(offset, 4) + 4 + (strlen({value}) + 1);\n",
                indent = indent,
                value = value_expr
            ),
            PrimitiveType::WString => format!(
                "{indent}offset = cdr_align(offset, 4) + 4 + (wcslen({value}) + 1) * sizeof(wchar_t);\n",
                indent = indent,
                value = value_expr
            ),
            PrimitiveType::Void => format!(
                "{indent}/* void: no size contribution */\n",
                indent = indent
            ),
            _ => super::primitive_scalar_layout(p).map_or_else(
                || format!("{indent}/* unsupported primitive: {:?} */\n", p),
                |layout| super::max_scalar(indent, layout.align, layout.width),
            ),
        },
        IdlType::Array { inner, size } => {
            emit_max_array(indent, inner, *size, idx, value_expr, field_name)
        }
        IdlType::Sequence { inner, .. } => {
            emit_max_sequence(indent, inner, idx, value_expr, field_name)
        }
        IdlType::Map { key, value, .. } => {
            emit_max_map(indent, key, value, idx, value_expr, field_name)
        }
        IdlType::Named(nm) => {
            let type_ident = last_ident_owned(nm);
            if idx.structs.contains_key(&type_ident) {
                format!(
                    "{indent}offset = cdr_align(offset, 4) + {fname}_max_cdr2_size({ptr});\n",
                    indent = indent,
                    fname = c_name(&type_ident),
                    ptr = ptr_expr
                )
            } else if idx.bitsets.contains_key(&type_ident)
                || idx.bitmasks.contains_key(&type_ident)
            {
                super::max_scalar(indent, 8, 8)
            } else if idx.enums.contains_key(&type_ident) {
                super::max_scalar(indent, 4, 4)
            } else if let Some(td) = idx.typedefs.get(&type_ident) {
                emit_max_type(indent, &td.base_type, idx, value_expr, ptr_expr, field_name)
            } else {
                format!(
                    "{indent}return offset; /* unsupported named type `{type_ident}` */\n",
                    indent = indent,
                    type_ident = type_ident
                )
            }
        }
    }
}

fn emit_max_array(
    indent: &str,
    inner: &IdlType,
    size: u32,
    idx: &DefinitionIndex,
    value_expr: &str,
    field_name: &str,
) -> String {
    let mut out = String::new();
    let align = idx.align_of(inner);
    let _ = writeln!(out, "{indent}offset = cdr_align(offset, {align});");
    let _ = writeln!(out, "{indent}for (uint32_t i = 0; i < {size}; ++i) {{");
    let next_indent = format!("{indent}    ", indent = indent);
    let element_value = format!("{}[i]", value_expr);
    let element_ptr = format!("&({}[i])", value_expr);
    out.push_str(&emit_max_type(
        &next_indent,
        inner,
        idx,
        &element_value,
        &element_ptr,
        &format!("{field_name}_elem"),
    ));
    let _ = writeln!(out, "{indent}}}");
    out
}

fn emit_max_sequence(
    indent: &str,
    inner: &IdlType,
    idx: &DefinitionIndex,
    value_expr: &str,
    field_name: &str,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{indent}offset = cdr_align(offset, 4) + 4;");
    let _ = writeln!(
        out,
        "{indent}for (uint32_t i = 0; i < {value}.len; ++i) {{",
        value = value_expr
    );
    let next_indent = format!("{indent}    ", indent = indent);
    let element_value = format!("*({value}.data + i)", value = value_expr);
    let element_ptr = format!("({value}.data + i)", value = value_expr);
    out.push_str(&emit_max_type(
        &next_indent,
        inner,
        idx,
        &element_value,
        &element_ptr,
        &format!("{field_name}_elem"),
    ));
    let _ = writeln!(out, "{indent}}}");
    out
}

fn emit_max_map(
    indent: &str,
    key: &IdlType,
    value: &IdlType,
    idx: &DefinitionIndex,
    value_expr: &str,
    field_name: &str,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{indent}offset = cdr_align(offset, 4) + 4;");
    let _ = writeln!(
        out,
        "{indent}for (uint32_t i = 0; i < {value}.len; ++i) {{",
        value = value_expr
    );
    let next_indent = format!("{indent}    ", indent = indent);
    let key_value = format!("*({value}.keys + i)", value = value_expr);
    let key_ptr = format!("({value}.keys + i)", value = value_expr);
    out.push_str(&emit_max_type(
        &next_indent,
        key,
        idx,
        &key_value,
        &key_ptr,
        &format!("{field_name}_key"),
    ));
    let val_value = format!("*({value}.values + i)", value = value_expr);
    let val_ptr = format!("({value}.values + i)", value = value_expr);
    out.push_str(&emit_max_type(
        &next_indent,
        value,
        idx,
        &val_value,
        &val_ptr,
        &format!("{field_name}_value"),
    ));
    let _ = writeln!(out, "{indent}}}");
    out
}
