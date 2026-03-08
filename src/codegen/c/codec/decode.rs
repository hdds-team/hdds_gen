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
        out.push_str(&emit_decode_type(
            "        ",
            &f.field_type,
            idx,
            &value_expr,
            &ptr_expr,
            &escaped,
            c_std,
        ));
        out.push_str("    }\n");
        out
    } else {
        emit_decode_type(
            "    ",
            &f.field_type,
            idx,
            &value_expr,
            &ptr_expr,
            &escaped,
            c_std,
        )
    }
}

fn emit_decode_type(
    indent: &str,
    ty: &IdlType,
    idx: &DefinitionIndex,
    value_expr: &str,
    ptr_expr: &str,
    field_name: &str,
    c_std: CStandard,
) -> String {
    let is_c89 = matches!(c_std, CStandard::C89);

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
            let mut out = String::new();
            if is_c89 {
                let _ = writeln!(out, "{indent}for (i = 0; i < {size}; ++i) {{");
            } else {
                let _ = writeln!(out, "{indent}for (uint32_t i = 0; i < {size}; ++i) {{");
            }
            let next_indent = format!("{indent}    ", indent = indent);
            let element_ptr = format!("&((*{value})[i])", value = ptr_expr);
            let element_value = format!("{}[i]", value_expr);
            out.push_str(&emit_decode_type(
                &next_indent,
                inner,
                idx,
                &element_value,
                &element_ptr,
                &format!("{field_name}_elem"),
                c_std,
            ));
            let _ = writeln!(out, "{indent}}}");
            out
        }
        IdlType::Sequence { inner, .. } => {
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
                    "{indent}for (i = 0; i < {value}.len; ++i) {{",
                    value = value_expr
                );
            } else {
                let _ = writeln!(
                    out,
                    "{indent}for (uint32_t i = 0; i < {value}.len; ++i) {{",
                    value = value_expr
                );
            }
            let next_indent = format!("{indent}    ", indent = indent);
            let element_ptr = format!("({value}.data + i)", value = value_expr);
            let element_value = format!("*({value}.data + i)", value = value_expr);
            out.push_str(&emit_decode_type(
                &next_indent,
                inner,
                idx,
                &element_value,
                &element_ptr,
                &format!("{field_name}_elem"),
                c_std,
            ));
            let _ = writeln!(out, "{indent}}}");
            out
        }
        IdlType::Map { key, value, .. } => {
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
                    "{indent}for (i = 0; i < {value}.len; ++i) {{",
                    value = value_expr
                );
            } else {
                let _ = writeln!(
                    out,
                    "{indent}for (uint32_t i = 0; i < {value}.len; ++i) {{",
                    value = value_expr
                );
            }
            let next_indent = format!("{indent}    ", indent = indent);
            let key_ptr = format!("({value}.keys + i)", value = value_expr);
            let key_value = format!("*({value}.keys + i)", value = value_expr);
            out.push_str(&emit_decode_type(
                &next_indent,
                key,
                idx,
                &key_value,
                &key_ptr,
                &format!("{field_name}_key"),
                c_std,
            ));
            let val_ptr = format!("({value}.values + i)", value = value_expr);
            let val_value = format!("*({value}.values + i)", value = value_expr);
            out.push_str(&emit_decode_type(
                &next_indent,
                value,
                idx,
                &val_value,
                &val_ptr,
                &format!("{field_name}_value"),
                c_std,
            ));
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
                emit_decode_type(
                    indent,
                    &td.base_type,
                    idx,
                    value_expr,
                    ptr_expr,
                    field_name,
                    c_std,
                )
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
         {indent}char* target = *{target_expr};\n\
         {indent}if (target == NULL) return -CDR_INVALID_DATA;\n\
         {indent}memcpy(target, src + offset, {len_var});\n\
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
         {indent}wchar_t* target = *{target_expr};\n\
         {indent}if (target == NULL) return -CDR_INVALID_DATA;\n\
         {indent}memcpy(target, src + offset, {bytes_var});\n\
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
