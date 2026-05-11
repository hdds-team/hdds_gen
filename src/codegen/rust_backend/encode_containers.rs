// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Encode helpers for container types (sequences, arrays, maps).
//!
//! Generates CDR2 serialization code for complex IDL containers.

use super::{push_fmt, CdrVersion, RustGenerator};
use crate::types::{IdlType, PrimitiveType};

impl RustGenerator {
    pub(super) fn append_encode_sequence(
        dst: &mut String,
        field_name: &str,
        inner: &IdlType,
        version: CdrVersion,
    ) {
        let suffix = super::helpers::xcdr_method_suffix(version);
        push_fmt(
            dst,
            format_args!("        // Encode sequence field '{field_name}'\n"),
        );
        push_fmt(
            dst,
            format_args!(
                "        let count_u32 = u32::try_from(self.{field_name}.len()).map_err(|_| CdrError::InvalidEncoding)?;\n"
            ),
        );
        Self::encode_buffer_check(dst, "        ", "4");
        dst.push_str("        dst[offset..offset+4].copy_from_slice(&count_u32.to_le_bytes());\n");
        dst.push_str("        offset += 4;\n");
        push_fmt(
            dst,
            format_args!("        for elem in &self.{field_name} {{\n"),
        );

        if matches!(inner, IdlType::Primitive(PrimitiveType::Fixed { .. })) {
            Self::encode_buffer_check(dst, "            ", "16");
            push_fmt(dst, format_args!("            let raw = elem.raw();\n"));
            push_fmt(
                dst,
                format_args!(
                    "            dst[offset..offset+16].copy_from_slice(&raw.to_le_bytes());\n"
                ),
            );
            dst.push_str("            offset += 16;\n");
        } else if matches!(inner, IdlType::Primitive(PrimitiveType::WChar)) {
            Self::encode_buffer_check(dst, "            ", "4");
            dst.push_str("            let scalar = u32::from(*elem);\n");
            dst.push_str(
                "            dst[offset..offset+4].copy_from_slice(&scalar.to_le_bytes());\n",
            );
            dst.push_str("            offset += 4;\n");
        } else if let Some(elem_size) = Self::cdr2_fixed_size(inner) {
            let size_expr = elem_size.to_string();
            Self::encode_buffer_check(dst, "            ", &size_expr);
            push_fmt(
                dst,
                format_args!(
                    "            dst[offset..offset+{elem_size}].copy_from_slice(&elem.to_le_bytes());\n"
                ),
            );
            push_fmt(dst, format_args!("            offset += {elem_size};\n"));
        } else if matches!(
            inner,
            IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString)
        ) || super::helpers::is_bounded_string(inner)
        {
            // Handle unbounded strings AND bounded strings (string<N> -> Sequence<Char, N>)
            Self::encode_string_expr(dst, "            ", "elem");
        } else if let IdlType::Named(name) = inner {
            if super::helpers::is_named_mutable(name) {
                Self::encode_buffer_check(dst, "            ", "4");
                dst.push_str("            let elem_start = offset;\n");
                dst.push_str("            offset += 4; // DHEADER per element\n");
                push_fmt(
                    dst,
                    format_args!("            elem.encode_{suffix}_le_at(dst, &mut offset)?;\n"),
                );
                dst.push_str("            let elem_len = u32::try_from(offset - (elem_start + 4)).map_err(|_| CdrError::InvalidEncoding)?;\n");
                dst.push_str("            dst[elem_start..elem_start+4].copy_from_slice(&elem_len.to_le_bytes());\n");
            } else {
                push_fmt(
                    dst,
                    format_args!("            elem.encode_{suffix}_le_at(dst, &mut offset)?;\n"),
                );
            }
        } else {
            push_fmt(
                dst,
                format_args!("            elem.encode_{suffix}_le_at(dst, &mut offset)?;\n"),
            );
        }
        dst.push_str("        }\n\n");
    }

    pub(super) fn append_encode_array(
        dst: &mut String,
        field_name: &str,
        inner: &IdlType,
        size: u32,
        version: CdrVersion,
    ) {
        let suffix = super::helpers::xcdr_method_suffix(version);
        push_fmt(
            dst,
            format_args!("        // Encode array field '{field_name}'\n"),
        );

        // Only use direct memcpy for byte-sized types (u8, i8, bool)
        if Self::is_byte_copyable(inner) {
            let total = size as usize;
            Self::encode_buffer_check(dst, "        ", &total.to_string());
            push_fmt(
                dst,
                format_args!(
                    "        dst[offset..offset+{total}].copy_from_slice(self.{field_name}.as_ref());\n"
                ),
            );
            push_fmt(dst, format_args!("        offset += {total};\n\n"));
        } else if let Some(elem_size) = Self::is_primitive_like(inner) {
            // For multi-byte primitives (f32, i32, etc.), encode each element with to_le_bytes
            let total = elem_size * (size as usize);
            Self::encode_buffer_check(dst, "        ", &total.to_string());
            dst.push_str("        for elem in &self.");
            dst.push_str(field_name);
            dst.push_str(" {\n");
            push_fmt(
                dst,
                format_args!(
                    "            dst[offset..offset+{elem_size}].copy_from_slice(&elem.to_le_bytes());\n"
                ),
            );
            push_fmt(dst, format_args!("            offset += {elem_size};\n"));
            dst.push_str("        }\n\n");
        } else {
            dst.push_str("        for elem in &self.");
            dst.push_str(field_name);
            dst.push_str(" {\n");
            if matches!(inner, IdlType::Primitive(PrimitiveType::Fixed { .. })) {
                Self::encode_buffer_check(dst, "            ", "16");
                push_fmt(dst, format_args!("            let raw = elem.raw();\n"));
                push_fmt(
                    dst,
                    format_args!(
                        "            dst[offset..offset+16].copy_from_slice(&raw.to_le_bytes());\n"
                    ),
                );
                dst.push_str("            offset += 16;\n");
            } else if matches!(inner, IdlType::Primitive(PrimitiveType::WChar)) {
                Self::encode_buffer_check(dst, "            ", "4");
                dst.push_str("            let scalar = u32::from(*elem);\n");
                dst.push_str(
                    "            dst[offset..offset+4].copy_from_slice(&scalar.to_le_bytes());\n",
                );
                dst.push_str("            offset += 4;\n");
            } else if matches!(
                inner,
                IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString)
            ) || super::helpers::is_bounded_string(inner)
            {
                // Handle unbounded strings AND bounded strings (string<N> -> Sequence<Char, N>)
                Self::encode_string_expr(dst, "            ", "elem");
            } else {
                push_fmt(
                    dst,
                    format_args!("            elem.encode_{suffix}_le_at(dst, &mut offset)?;\n"),
                );
            }
            dst.push_str("        }\n\n");
        }
    }

    pub(super) fn append_encode_map(
        dst: &mut String,
        field_name: &str,
        key: &IdlType,
        value: &IdlType,
        version: CdrVersion,
    ) {
        push_fmt(
            dst,
            format_args!("        // Encode map field '{field_name}'\n"),
        );
        push_fmt(
            dst,
            format_args!(
                "        let count_u32 = u32::try_from(self.{field_name}.len()).map_err(|_| CdrError::InvalidEncoding)?;\n"
            ),
        );
        Self::encode_buffer_check(dst, "        ", "4");
        dst.push_str("        dst[offset..offset+4].copy_from_slice(&count_u32.to_le_bytes());\n");
        dst.push_str("        offset += 4;\n");
        push_fmt(
            dst,
            format_args!("        for (k, v) in &self.{field_name} {{\n"),
        );

        let key_size = Self::is_primitive_like(key);
        let value_size = Self::is_primitive_like(value);

        if let (Some(k), Some(v)) = (key_size, value_size) {
            let combined = (k + v).to_string();
            Self::encode_buffer_check(dst, "            ", &combined);
            push_fmt(
                dst,
                format_args!(
                    "            dst[offset..offset+{k}].copy_from_slice(&k.to_le_bytes());\n"
                ),
            );
            push_fmt(dst, format_args!("            offset += {k};\n"));
            push_fmt(
                dst,
                format_args!(
                    "            dst[offset..offset+{v}].copy_from_slice(&v.to_le_bytes());\n"
                ),
            );
            push_fmt(dst, format_args!("            offset += {v};\n"));
        } else {
            dst.push_str("            // Encode key\n");
            Self::append_encode_map_component(dst, "            ", "k", key, key_size, version);
            dst.push_str("            // Encode value\n");
            Self::append_encode_map_component(dst, "            ", "v", value, value_size, version);
        }
        dst.push_str("        }\n\n");
    }

    pub(super) fn append_encode_map_component(
        dst: &mut String,
        indent: &str,
        expr: &str,
        ty: &IdlType,
        fixed_size: Option<usize>,
        version: CdrVersion,
    ) {
        let suffix = super::helpers::xcdr_method_suffix(version);
        if let Some(size) = fixed_size {
            match ty {
                IdlType::Primitive(PrimitiveType::Boolean) => {
                    Self::encode_buffer_check(dst, indent, "1");
                    push_fmt(
                        dst,
                        format_args!("{indent}dst[offset] = u8::from(*{expr});\n"),
                    );
                    push_fmt(dst, format_args!("{indent}offset += 1;\n"));
                }
                IdlType::Primitive(PrimitiveType::Char) => {
                    Self::encode_buffer_check(dst, indent, "1");
                    push_fmt(
                        dst,
                        format_args!("{indent}let scalar = u32::from(*{expr});\n"),
                    );
                    push_fmt(
                        dst,
                        format_args!(
                            "{indent}let byte = u8::try_from(scalar).map_err(|_| CdrError::InvalidEncoding)?;\n"
                        ),
                    );
                    push_fmt(dst, format_args!("{indent}dst[offset] = byte;\n"));
                    push_fmt(dst, format_args!("{indent}offset += 1;\n"));
                }
                IdlType::Primitive(PrimitiveType::WChar) => {
                    Self::encode_buffer_check(dst, indent, "4");
                    push_fmt(
                        dst,
                        format_args!("{indent}let scalar = u32::from(*{expr});\n"),
                    );
                    push_fmt(
                        dst,
                        format_args!(
                            "{indent}dst[offset..offset+4].copy_from_slice(&scalar.to_le_bytes());\n"
                        ),
                    );
                    push_fmt(dst, format_args!("{indent}offset += 4;\n"));
                }
                IdlType::Primitive(_) => {
                    let size_expr = size.to_string();
                    Self::encode_buffer_check(dst, indent, &size_expr);
                    push_fmt(
                        dst,
                        format_args!(
                            "{indent}dst[offset..offset+{size}].copy_from_slice(&(*{expr}).to_le_bytes());\n"
                        ),
                    );
                    push_fmt(dst, format_args!("{indent}offset += {size};\n"));
                }
                _ => {
                    push_fmt(
                        dst,
                        format_args!("{indent}{expr}.encode_{suffix}_le_at(dst, &mut offset)?;\n"),
                    );
                }
            }
            return;
        }

        if matches!(ty, IdlType::Primitive(PrimitiveType::Fixed { .. })) {
            Self::encode_buffer_check(dst, indent, "16");
            push_fmt(dst, format_args!("{indent}let raw = {expr}.raw();\n"));
            push_fmt(
                dst,
                format_args!(
                    "{indent}dst[offset..offset+16].copy_from_slice(&raw.to_le_bytes());\n"
                ),
            );
            push_fmt(dst, format_args!("{indent}offset += 16;\n"));
        } else if matches!(ty, IdlType::Primitive(PrimitiveType::WChar)) {
            Self::encode_buffer_check(dst, indent, "4");
            push_fmt(
                dst,
                format_args!("{indent}let scalar = u32::from(*{expr});\n"),
            );
            push_fmt(
                dst,
                format_args!(
                    "{indent}dst[offset..offset+4].copy_from_slice(&scalar.to_le_bytes());\n"
                ),
            );
            push_fmt(dst, format_args!("{indent}offset += 4;\n"));
        } else if matches!(
            ty,
            IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString)
        ) || super::helpers::is_bounded_string(ty)
        {
            // Handle unbounded strings AND bounded strings (string<N> -> Sequence<Char, N>)
            Self::encode_string_expr(dst, indent, expr);
        } else {
            push_fmt(
                dst,
                format_args!("{indent}{expr}.encode_{suffix}_le_at(dst, &mut offset)?;\n"),
            );
        }
    }
}
