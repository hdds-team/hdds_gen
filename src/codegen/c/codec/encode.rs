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
        out.push_str(&emit_encode_type(
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
        emit_encode_type(
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

fn emit_encode_type(
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
            let mut out = String::new();
            let align = idx.align_of(inner);
            let _ = write!(
                out,
                "{indent}err = cdr_pad(dst, &offset, len, {align});\n{indent}if (err) {{ return err; }}\n"
            );
            // C89: use pre-declared 'i', C99+: declare in for-loop
            if is_c89 {
                let _ = writeln!(out, "{indent}for (i = 0; i < {size}; ++i) {{");
            } else {
                let _ = writeln!(out, "{indent}for (uint32_t i = 0; i < {size}; ++i) {{");
            }
            let next_indent = format!("{indent}    ", indent = indent);
            let element_ty = inner.as_ref();
            let element_value = format!("{}[i]", value_expr);
            let element_ptr = format!("&({}[i])", value_expr);
            out.push_str(&emit_encode_type(
                &next_indent,
                element_ty,
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
            let element_ty = inner.as_ref();
            let element_value = format!("*({value}.data + i)", value = value_expr);
            let element_ptr = format!("({value}.data + i)", value = value_expr);
            out.push_str(&emit_encode_type(
                &next_indent,
                element_ty,
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
            let key_ty = key.as_ref();
            let val_ty = value.as_ref();
            let key_value = format!("*({value}.keys + i)", value = value_expr);
            let key_ptr = format!("({value}.keys + i)", value = value_expr);
            out.push_str(&emit_encode_type(
                &next_indent,
                key_ty,
                idx,
                &key_value,
                &key_ptr,
                &format!("{field_name}_key"),
                c_std,
            ));
            let val_value = format!("*({value}.values + i)", value = value_expr);
            let val_ptr = format!("({value}.values + i)", value = value_expr);
            out.push_str(&emit_encode_type(
                &next_indent,
                val_ty,
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
                emit_encode_type(
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
