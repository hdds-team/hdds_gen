// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! CDR2 decode function generation for C.
//!
//! Emits field-by-field deserialization code.

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

pub(super) fn emit_decode_field(
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
            "    err = cdr_need_read(len, offset, 1);\n    if (err) {{ return err; }}\n"
        );
        let _ = writeln!(
            out,
            "    {parent}->has_{name} = (src[offset++] != 0) ? 1 : 0;",
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
        out.push_str(&emit_decode_type("        ", &f.field_type, idx, &fe, c_std, 0));
        out.push_str("    }\n");
        out
    } else {
        let fe = FieldExprs { value: &value_expr, ptr: &ptr_expr, name: &escaped };
        emit_decode_type("    ", &f.field_type, idx, &fe, c_std, 0)
    }
}

fn emit_decode_type(
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
                decode_c_string(indent, &format!("{field_name}_len"), ptr_expr, is_c89)
            }
            PrimitiveType::WString => {
                decode_c_wstring(indent, &format!("{field_name}_bytes"), ptr_expr, is_c89)
            }
            PrimitiveType::WChar => decode_wchar(indent, ptr_expr, is_c89),
            PrimitiveType::Fixed { .. } => decode_fixed(indent, ptr_expr, is_c89),
            PrimitiveType::Void => format!("{indent}/* void: no decoding */\n", indent = indent),
            _ => super::primitive_scalar_layout(p).map_or_else(
                || format!("{indent}/* unsupported primitive: {:?} */\n", p),
                |layout| decode_scalar(indent, layout.align, layout.width, ptr_expr),
            ),
        },
        IdlType::Array { inner, size } => {
            let var = loop_var(depth);
            let mut out = String::new();
            if is_c89 {
                let _ = writeln!(out, "{indent}for ({var} = 0; {var} < {size}; ++{var}) {{");
            } else {
                let _ = writeln!(out, "{indent}for (uint32_t {var} = 0; {var} < {size}; ++{var}) {{");
            }
            let next_indent = format!("{indent}    ", indent = indent);
            let element_value = format!("{value_expr}[{var}]");
            let element_ptr = format!("&({value_expr}[{var}])");
            let elem_name = format!("{field_name}_elem");
            let elem_fe = FieldExprs { value: &element_value, ptr: &element_ptr, name: &elem_name };
            out.push_str(&emit_decode_type(&next_indent, inner, idx, &elem_fe, c_std, depth + 1));
            let _ = writeln!(out, "{indent}}}");
            out
        }
        IdlType::Sequence { inner, .. } => {
            let var = loop_var(depth);
            let mut out = String::new();
            let _ = write!(
                out,
                "{indent}err = cdr_skip(src, &offset, len, CDR_ALIGN_4);\n{indent}if (err) {{ return err; }}\n"
            );
            let _ = write!(
                out,
                "{indent}err = cdr_need_read(len, offset, CDR_SIZE_INT32);\n{indent}if (err) {{ return err; }}\n"
            );
            let _ = writeln!(
                out,
                "{indent}memcpy(&{value}.len, src + offset, CDR_SIZE_INT32);",
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
            let element_value = format!("{value}.data[{var}]", value = value_expr);
            let element_ptr = format!("&({value}.data[{var}])", value = value_expr);
            let elem_name = format!("{field_name}_elem");
            let elem_fe = FieldExprs { value: &element_value, ptr: &element_ptr, name: &elem_name };
            out.push_str(&emit_decode_type(&next_indent, inner, idx, &elem_fe, c_std, depth + 1));
            let _ = writeln!(out, "{indent}}}");
            out
        }
        IdlType::Map { key, value, .. } => {
            let var = loop_var(depth);
            let mut out = String::new();
            let _ = write!(
                out,
                "{indent}err = cdr_skip(src, &offset, len, CDR_ALIGN_4);\n{indent}if (err) {{ return err; }}\n"
            );
            let _ = write!(
                out,
                "{indent}err = cdr_need_read(len, offset, CDR_SIZE_INT32);\n{indent}if (err) {{ return err; }}\n"
            );
            let _ = writeln!(
                out,
                "{indent}memcpy(&{value}.len, src + offset, CDR_SIZE_INT32);",
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
            let key_value = format!("{value}.keys[{var}]", value = value_expr);
            let key_ptr = format!("&({value}.keys[{var}])", value = value_expr);
            let key_name = format!("{field_name}_key");
            let key_fe = FieldExprs { value: &key_value, ptr: &key_ptr, name: &key_name };
            out.push_str(&emit_decode_type(&next_indent, key, idx, &key_fe, c_std, depth + 1));
            let val_value = format!("{value}.values[{var}]", value = value_expr);
            let val_ptr = format!("&({value}.values[{var}])", value = value_expr);
            let val_name = format!("{field_name}_value");
            let val_fe = FieldExprs { value: &val_value, ptr: &val_ptr, name: &val_name };
            out.push_str(&emit_decode_type(&next_indent, value, idx, &val_fe, c_std, depth + 1));
            let _ = writeln!(out, "{indent}}}");
            out
        }
        IdlType::Named(nm) => {
            let type_ident = last_ident_owned(nm);
            if idx.structs.contains_key(&type_ident) {
                format!(
                    "{indent}{{ int e = {fname}_decode_cdr2_le({ptr}, src + offset, len - offset); if (e < 0) return e; err = cdr_add(&offset, (size_t)e); if (err) return err; }}\n",
                    indent = indent,
                    fname = c_name(&type_ident),
                    ptr = ptr_expr
                )
            } else if idx.bitsets.contains_key(&type_ident)
                || idx.bitmasks.contains_key(&type_ident)
            {
                decode_scalar(indent, 8, 8, ptr_expr)
            } else if idx.enums.contains_key(&type_ident) {
                decode_scalar(indent, 4, 4, ptr_expr)
            } else if let Some(td) = idx.typedefs.get(&type_ident) {
                let td_fe = FieldExprs { value: value_expr, ptr: ptr_expr, name: field_name };
                emit_decode_type(indent, &td.base_type, idx, &td_fe, c_std, depth)
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

fn decode_scalar(indent: &str, align: usize, size: usize, ptr_expr: &str) -> String {
    format!(
        "{indent}err = cdr_skip(src, &offset, len, {align});\n{indent}if (err) {{ return err; }}\n{indent}err = cdr_need_read(len, offset, {size});\n{indent}if (err) {{ return err; }}\n{indent}memcpy({ptr_expr}, src + offset, {size});\n{indent}err = cdr_add(&offset, {size});\n{indent}if (err) {{ return err; }}\n"
    )
}

fn decode_c_string(indent: &str, len_var: &str, target_expr: &str, is_c89: bool) -> String {
    let var_decl = if is_c89 {
        format!("{indent}{len_var} = 0;\n")
    } else {
        format!("{indent}uint32_t {len_var} = 0;\n")
    };
    format!(
        "{indent}err = cdr_skip(src, &offset, len, CDR_ALIGN_4);\n\
         {indent}if (err) {{ return err; }}\n\
         {var_decl}\
         {indent}err = cdr_need_read(len, offset, CDR_SIZE_INT32);\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}memcpy(&{len_var}, src + offset, CDR_SIZE_INT32);\n\
         {indent}err = cdr_add(&offset, CDR_SIZE_INT32);\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}if ({len_var} > CDR_MAX_STRING) return -CDR_INVALID_DATA;\n\
         {indent}err = cdr_need_read(len, offset, {len_var});\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}{{\n\
         {indent}    char* target = *{target_expr};\n\
         {indent}    if (target == NULL) return -CDR_INVALID_DATA;\n\
         {indent}    memcpy(target, src + offset, {len_var});\n\
         {indent}}}\n\
         {indent}err = cdr_add(&offset, {len_var});\n\
         {indent}if (err) {{ return err; }}\n"
    )
}

fn decode_c_wstring(indent: &str, bytes_var: &str, target_expr: &str, is_c89: bool) -> String {
    let var_decl = if is_c89 {
        format!("{indent}{bytes_var} = 0;\n")
    } else {
        format!("{indent}uint32_t {bytes_var} = 0;\n")
    };
    format!(
        "{indent}err = cdr_skip(src, &offset, len, CDR_ALIGN_4);\n\
         {indent}if (err) {{ return err; }}\n\
         {var_decl}\
         {indent}err = cdr_need_read(len, offset, CDR_SIZE_INT32);\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}memcpy(&{bytes_var}, src + offset, CDR_SIZE_INT32);\n\
         {indent}err = cdr_add(&offset, CDR_SIZE_INT32);\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}if ({bytes_var} > CDR_MAX_STRING) return -CDR_INVALID_DATA;\n\
         {indent}err = cdr_need_read(len, offset, {bytes_var});\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}{{\n\
         {indent}    wchar_t* target = *{target_expr};\n\
         {indent}    if (target == NULL) return -CDR_INVALID_DATA;\n\
         {indent}    memcpy(target, src + offset, {bytes_var});\n\
         {indent}}}\n\
         {indent}err = cdr_add(&offset, {bytes_var});\n\
         {indent}if (err) {{ return err; }}\n"
    )
}

fn decode_wchar(indent: &str, ptr_expr: &str, is_c89: bool) -> String {
    let var_decl = if is_c89 {
        format!("{indent}scalar = 0;\n")
    } else {
        format!("{indent}uint32_t scalar = 0;\n")
    };
    format!(
        "{indent}err = cdr_skip(src, &offset, len, CDR_ALIGN_4);\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}err = cdr_need_read(len, offset, CDR_SIZE_WCHAR);\n\
         {indent}if (err) {{ return err; }}\n\
         {var_decl}\
         {indent}memcpy(&scalar, src + offset, CDR_SIZE_WCHAR);\n\
         {indent}err = cdr_add(&offset, CDR_SIZE_WCHAR);\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}if (scalar > CDR_UNICODE_MAX) return -CDR_INVALID_DATA;\n\
         {indent}if (scalar > (uint32_t)WCHAR_MAX) return -CDR_INVALID_DATA;\n\
         {indent}*{ptr_expr} = (wchar_t)scalar;\n"
    )
}

fn decode_fixed(indent: &str, ptr_expr: &str, is_c89: bool) -> String {
    let var_decl = if is_c89 {
        ""
    } else {
        "uint8_t raw[CDR_SIZE_FIXED128];\n    "
    };
    format!(
        "{indent}err = cdr_skip(src, &offset, len, CDR_ALIGN_4);\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}err = cdr_need_read(len, offset, CDR_SIZE_FIXED128);\n\
         {indent}if (err) {{ return err; }}\n\
         {indent}{var_decl}memcpy(raw, src + offset, CDR_SIZE_FIXED128);\n\
         {indent}cdr_fixed128_from_le({ptr_expr}, raw);\n\
         {indent}err = cdr_add(&offset, CDR_SIZE_FIXED128);\n\
         {indent}if (err) {{ return err; }}\n"
    )
}
