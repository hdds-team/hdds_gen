// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! CDR2 encode function generation for C.
//!
//! Emits field-by-field serialization code.

use super::super::super::keywords::c_ident;
use super::super::{
    helpers::{c_name, last_ident_owned},
    index::DefinitionIndex,
    CStandard,
};
use crate::ast::Field;
use crate::types::{IdlType, PrimitiveType};
use std::fmt::Write;

fn loop_var(depth: u32) -> &'static str {
    const VARS: &[&str] = &["i", "j", "k", "l", "m", "n"];
    VARS.get(depth as usize).unwrap_or(&"n")
}

/// Groups the three field-related expressions to keep function signatures short.
struct FieldExprs<'a> {
    value: &'a str,
    ptr: &'a str,
    name: &'a str,
}

pub(super) fn emit_encode_field(
    f: &Field,
    idx: &DefinitionIndex,
    parent: &str,
    c_std: CStandard,
) -> String {
    let escaped = c_ident(&f.name);
    let value_expr = format!("{}->{}", parent, escaped);
    let ptr_expr = format!("&({})", value_expr);

    if f.is_optional() {
        let mut out = String::new();
        let _ = write!(
            out,
            "    err = cdr_need_write(len, offset, 1);\n    if (err) {{ return err; }}\n"
        );
        let _ = writeln!(
            out,
            "    dst[offset++] = {parent}->has_{name} ? 0x01 : 0x00;",
            parent = parent,
            name = escaped
        );
        let _ = writeln!(
            out,
            "    if ({parent}->has_{name}) {{",
            parent = parent,
            name = escaped
        );
        let fe = FieldExprs { value: &value_expr, ptr: &ptr_expr, name: &escaped };
        out.push_str(&emit_encode_type("        ", &f.field_type, idx, &fe, c_std, 0));
        out.push_str("    }\n");
        out
    } else {
        let fe = FieldExprs { value: &value_expr, ptr: &ptr_expr, name: &escaped };
        emit_encode_type("    ", &f.field_type, idx, &fe, c_std, 0)
    }
}

fn emit_encode_type(
    indent: &str,
    ty: &IdlType,
    idx: &DefinitionIndex,
    fe: &FieldExprs<'_>,
    c_std: CStandard,
    depth: u32,
) -> String {
    let is_c89 = matches!(c_std, CStandard::C89);
    let value_expr = fe.value;
    let ptr_expr = fe.ptr;
    let field_name = fe.name;

    match ty {
        IdlType::Primitive(p) => match p {
            PrimitiveType::String => {
                encode_c_string(indent, &format!("{field_name}_len"), value_expr, is_c89)
            }
            PrimitiveType::WString => {
                encode_c_wstring(indent, &format!("{field_name}_bytes"), value_expr, is_c89)
            }
            PrimitiveType::WChar => encode_wchar(indent, value_expr, is_c89),
            PrimitiveType::Fixed { .. } => encode_fixed(indent, ptr_expr, is_c89),
            PrimitiveType::Void => format!("{indent}/* void: no encoding */\n", indent = indent),
            _ => super::primitive_scalar_layout(p).map_or_else(
                || format!("{indent}/* unsupported primitive: {:?} */\n", p),
                |layout| encode_scalar(indent, layout.align, layout.width, ptr_expr),
            ),
        },
        IdlType::Array { inner, size } => {
            let var = loop_var(depth);
            let mut out = String::new();
            let align = idx.align_of(inner);
            let _ = write!(
                out,
                "{indent}err = cdr_pad(dst, &offset, len, {align});\n{indent}if (err) {{ return err; }}\n"
            );
            // C89: use pre-declared var, C99+: declare in for-loop
            if is_c89 {
                let _ = writeln!(out, "{indent}for ({var} = 0; {var} < {size}; ++{var}) {{");
            } else {
                let _ = writeln!(out, "{indent}for (uint32_t {var} = 0; {var} < {size}; ++{var}) {{");
            }
            let next_indent = format!("{indent}    ", indent = indent);
            let element_ty = inner.as_ref();
            let element_value = format!("{value_expr}[{var}]");
            let element_ptr = format!("&({value_expr}[{var}])");
            let elem_name = format!("{field_name}_elem");
            let elem_fe = FieldExprs { value: &element_value, ptr: &element_ptr, name: &elem_name };
            out.push_str(&emit_encode_type(&next_indent, element_ty, idx, &elem_fe, c_std, depth + 1));
            let _ = writeln!(out, "{indent}}}");
            out
        }
        IdlType::Sequence { inner, .. } => {
            let var = loop_var(depth);
            let mut out = String::new();
            let _ = write!(
                out,
                "{indent}err = cdr_pad(dst, &offset, len, CDR_ALIGN_4);\n{indent}if (err) {{ return err; }}\n"
            );
            let _ = write!(
                out,
                "{indent}err = cdr_need_write(len, offset, CDR_SIZE_INT32);\n{indent}if (err) {{ return err; }}\n"
            );
            let _ = writeln!(
                out,
                "{indent}memcpy(dst + offset, &({value}.len), CDR_SIZE_INT32);",
                value = value_expr
            );
            let _ = write!(
                out,
                "{indent}err = cdr_add(&offset, CDR_SIZE_INT32);\n{indent}if (err) {{ return err; }}\n"
            );
            if is_c89 {
                let _ = writeln!(
                    out,
                    "{indent}for ({var} = 0; {var} < {value}.len; ++{var}) {{",
                    value = value_expr
                );
            } else {
                let _ = writeln!(
                    out,
                    "{indent}for (uint32_t {var} = 0; {var} < {value}.len; ++{var}) {{",
                    value = value_expr
                );
            }
            let next_indent = format!("{indent}    ", indent = indent);
            let element_ty = inner.as_ref();
            let element_value = format!("{value}.data[{var}]", value = value_expr);
            let element_ptr = format!("&({value}.data[{var}])", value = value_expr);
            let elem_name = format!("{field_name}_elem");
            let elem_fe = FieldExprs { value: &element_value, ptr: &element_ptr, name: &elem_name };
            out.push_str(&emit_encode_type(&next_indent, element_ty, idx, &elem_fe, c_std, depth + 1));
            let _ = writeln!(out, "{indent}}}");
            out
        }
        IdlType::Map { key, value, .. } => {
            let var = loop_var(depth);
            let mut out = String::new();
            let _ = write!(
                out,
                "{indent}err = cdr_pad(dst, &offset, len, CDR_ALIGN_4);\n{indent}if (err) {{ return err; }}\n"
            );
            let _ = write!(
                out,
                "{indent}err = cdr_need_write(len, offset, CDR_SIZE_INT32);\n{indent}if (err) {{ return err; }}\n"
            );
            let _ = writeln!(
                out,
                "{indent}memcpy(dst + offset, &({value}.len), CDR_SIZE_INT32);",
                value = value_expr
            );
            let _ = write!(
                out,
                "{indent}err = cdr_add(&offset, CDR_SIZE_INT32);\n{indent}if (err) {{ return err; }}\n"
            );
            if is_c89 {
                let _ = writeln!(
                    out,
                    "{indent}for ({var} = 0; {var} < {value}.len; ++{var}) {{",
                    value = value_expr
                );
            } else {
                let _ = writeln!(
                    out,
                    "{indent}for (uint32_t {var} = 0; {var} < {value}.len; ++{var}) {{",
                    value = value_expr
                );
            }
            let next_indent = format!("{indent}    ", indent = indent);
            let key_ty = key.as_ref();
            let val_ty = value.as_ref();
            let key_value = format!("{value}.keys[{var}]", value = value_expr);
            let key_ptr = format!("&({value}.keys[{var}])", value = value_expr);
            let key_name = format!("{field_name}_key");
            let key_fe = FieldExprs { value: &key_value, ptr: &key_ptr, name: &key_name };
            out.push_str(&emit_encode_type(&next_indent, key_ty, idx, &key_fe, c_std, depth + 1));
            let val_value = format!("{value}.values[{var}]", value = value_expr);
            let val_ptr = format!("&({value}.values[{var}])", value = value_expr);
            let val_name = format!("{field_name}_value");
            let val_fe = FieldExprs { value: &val_value, ptr: &val_ptr, name: &val_name };
            out.push_str(&emit_encode_type(&next_indent, val_ty, idx, &val_fe, c_std, depth + 1));
            let _ = writeln!(out, "{indent}}}");
            out
        }
        IdlType::Named(nm) => {
            let type_ident = last_ident_owned(nm);
            if idx.structs.contains_key(&type_ident) {
                format!(
                    "{indent}{{ int e = {fname}_encode_cdr2_le({ptr}, dst + offset, len - offset); if (e < 0) return e; err = cdr_add(&offset, (size_t)e); if (err) return err; }}\n",
                    indent = indent,
                    fname = c_name(&type_ident),
                    ptr = ptr_expr
                )
            } else if idx.bitsets.contains_key(&type_ident)
                || idx.bitmasks.contains_key(&type_ident)
            {
                encode_scalar(indent, 8, 8, ptr_expr)
            } else if idx.enums.contains_key(&type_ident) {
                encode_scalar(indent, 4, 4, ptr_expr)
            } else if let Some(td) = idx.typedefs.get(&type_ident) {
                let td_fe = FieldExprs { value: value_expr, ptr: ptr_expr, name: field_name };
                emit_encode_type(indent, &td.base_type, idx, &td_fe, c_std, depth)
            } else {
                format!(
                    "{indent}return -CDR_INVALID_DATA; /* unsupported named type `{type_ident}` */\n",
                    indent = indent,
                    type_ident = type_ident
                )
            }
        }
    }
}

fn encode_scalar(indent: &str, align: usize, size: usize, ptr_expr: &str) -> String {
    format!(
        "{indent}err = cdr_pad(dst, &offset, len, {align});\n{indent}if (err) {{ return err; }}\n{indent}err = cdr_need_write(len, offset, {size});\n{indent}if (err) {{ return err; }}\n{indent}memcpy(dst + offset, {ptr_expr}, {size});\n{indent}err = cdr_add(&offset, {size});\n{indent}if (err) {{ return err; }}\n"
    )
}

fn encode_c_string(indent: &str, len_var: &str, value_expr: &str, is_c89: bool) -> String {
    // For C89, the variable is already declared at function start
    let var_decl = if is_c89 {
        format!("{indent}{len_var} = (uint32_t)(strlen({value_expr}) + 1);\n")
    } else {
        format!("{indent}uint32_t {len_var} = (uint32_t)(strlen({value_expr}) + 1);\n")
    };
    format!(
        "{indent}err = cdr_pad(dst, &offset, len, CDR_ALIGN_4);\n\
         {indent}if (err) {{ return err; }}\n\
         {var_decl}\
         {indent}err = cdr_need_write(len, offset, CDR_SIZE_INT32);\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}memcpy(dst + offset, &{len_var}, CDR_SIZE_INT32);\n\
         {indent}err = cdr_add(&offset, CDR_SIZE_INT32);\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}err = cdr_need_write(len, offset, {len_var});\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}memcpy(dst + offset, {value_expr}, {len_var});\n\
         {indent}err = cdr_add(&offset, {len_var});\n\
         {indent}if (err) {{ return err; }}\n"
    )
}

fn encode_c_wstring(indent: &str, bytes_var: &str, value_expr: &str, is_c89: bool) -> String {
    let var_decl = if is_c89 {
        format!("{indent}{bytes_var} = (uint32_t)((wcslen({value_expr}) + 1) * sizeof(wchar_t));\n")
    } else {
        format!(
            "{indent}uint32_t {bytes_var} = (uint32_t)((wcslen({value_expr}) + 1) * sizeof(wchar_t));\n"
        )
    };
    format!(
        "{indent}err = cdr_pad(dst, &offset, len, CDR_ALIGN_4);\n\
         {indent}if (err) {{ return err; }}\n\
         {var_decl}\
         {indent}err = cdr_need_write(len, offset, CDR_SIZE_INT32);\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}memcpy(dst + offset, &{bytes_var}, CDR_SIZE_INT32);\n\
         {indent}err = cdr_add(&offset, CDR_SIZE_INT32);\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}err = cdr_need_write(len, offset, {bytes_var});\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}memcpy(dst + offset, {value_expr}, {bytes_var});\n\
         {indent}err = cdr_add(&offset, {bytes_var});\n\
         {indent}if (err) {{ return err; }}\n"
    )
}

fn encode_wchar(indent: &str, value_expr: &str, is_c89: bool) -> String {
    let var_decl = if is_c89 {
        format!("{indent}scalar = (uint32_t)({value_expr});\n")
    } else {
        format!("{indent}uint32_t scalar = (uint32_t)({value_expr});\n")
    };
    format!(
        "{indent}err = cdr_pad(dst, &offset, len, CDR_ALIGN_4);\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}err = cdr_need_write(len, offset, CDR_SIZE_WCHAR);\n\
         {indent}if (err) {{ return err; }}\n\
         {var_decl}\
         {indent}if (scalar > CDR_UNICODE_MAX) {{ return -CDR_INVALID_DATA; }}\n\
         {indent}memcpy(dst + offset, &scalar, CDR_SIZE_WCHAR);\n\
         {indent}err = cdr_add(&offset, CDR_SIZE_WCHAR);\n\
         {indent}if (err) {{ return err; }}\n"
    )
}

fn encode_fixed(indent: &str, ptr_expr: &str, is_c89: bool) -> String {
    // For C89, raw[] is declared at function start
    let var_decl = if is_c89 {
        ""
    } else {
        "uint8_t raw[CDR_SIZE_FIXED128];\n    "
    };
    format!(
        "{indent}err = cdr_pad(dst, &offset, len, CDR_ALIGN_4);\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}err = cdr_need_write(len, offset, CDR_SIZE_FIXED128);\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}{var_decl}cdr_fixed128_to_le({ptr_expr}, raw);\n\
         {indent}memcpy(dst + offset, raw, CDR_SIZE_FIXED128);\n\
         {indent}err = cdr_add(&offset, CDR_SIZE_FIXED128);\n\
         {indent}if (err) {{ return err; }}\n"
    )
}
