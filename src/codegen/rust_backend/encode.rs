// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! CDR2 encode trait generation for Rust structs.
//!
//! Emits `Cdr2Encode` trait implementations for serialization.

#![allow(clippy::uninlined_format_args)]

use super::{push_fmt, RustGenerator};
use crate::ast::{Field, Struct};
use crate::types::{IdlType, PrimitiveType};

impl RustGenerator {
    pub(super) fn emit_cdr2_encode_impl(s: &Struct) -> String {
        if super::helpers::is_compact_mutable_struct(s) {
            return Self::emit_pl_cdr2_compact_encode_impl(s);
        }

        if super::helpers::is_mutable_struct(s) {
            return Self::emit_pl_cdr2_encode_impl(s);
        }

        let mut code = String::new();

        push_fmt(
            &mut code,
            format_args!("impl Cdr2Encode for {} {{\n", s.name),
        );
        code.push_str(
            "    fn encode_cdr2_le(&self, dst: &mut [u8]) -> Result<usize, CdrError> {\n",
        );
        code.push_str("        let mut offset: usize = 0;\n\n");

        for field in &s.fields {
            if field.is_non_serialized() {
                continue;
            }
            if field.is_optional() {
                code.push_str(&Self::emit_optional_field_encode(field));
            } else {
                let alignment = Self::cdr2_alignment(&field.field_type);
                if alignment > 1 {
                    push_fmt(
                        &mut code,
                        format_args!(
                            "        // Align to {}-byte boundary for field '{}'\n",
                            alignment, field.name
                        ),
                    );
                    push_fmt(
                        &mut code,
                        format_args!(
                            "        let padding = ({alignment} - (offset % {alignment})) % {alignment};\n"
                        ),
                    );
                    code.push_str("        offset += padding;\n\n");
                }

                code.push_str(&Self::emit_encode_field(field));
            }
        }

        code.push_str("        Ok(offset)\n");
        code.push_str("    }\n\n");

        code.push_str("    fn max_cdr2_size(&self) -> usize {\n");
        code.push_str("        // Conservative estimate with max padding\n");

        let mut size_expr = String::new();
        for (i, field) in s.fields.iter().enumerate() {
            if i > 0 {
                size_expr.push_str(" + ");
            }

            // Optional fields have a 1-byte presence flag
            if field.is_optional() {
                size_expr.push_str("1 + ");
            }

            let alignment = Self::cdr2_alignment(&field.field_type);
            if alignment > 1 {
                push_fmt(&mut size_expr, format_args!("{} + ", alignment - 1));
            }

            if let Some(fixed_size) = Self::cdr2_fixed_size(&field.field_type) {
                push_fmt(&mut size_expr, format_args!("{fixed_size}"));
            } else {
                match &field.field_type {
                    IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString) => {
                        size_expr.push_str("(4 + 256 + 1)");
                    }
                    IdlType::Sequence { .. } => {
                        size_expr.push_str("(4 + 1024)");
                    }
                    _ => {
                        size_expr.push_str("64");
                    }
                }
            }
        }

        if size_expr.is_empty() {
            size_expr = "0".to_string();
        }

        push_fmt(&mut code, format_args!("        {size_expr}\n"));
        code.push_str("    }\n");
        code.push_str("}\n\n");

        code
    }

    fn emit_encode_field(field: &Field) -> String {
        let mut code = String::new();
        Self::append_encode_field(&mut code, field);
        code
    }

    /// Encode an `@optional` field in standard CDR2: 1-byte presence flag + value.
    fn emit_optional_field_encode(field: &Field) -> String {
        let mut code = String::new();
        let fname = super::super::keywords::rust_ident(&field.name);
        let alignment = Self::cdr2_alignment(&field.field_type);

        // Boolean presence flag (1 byte)
        Self::encode_buffer_check(&mut code, "        ", "1");
        push_fmt(
            &mut code,
            format_args!("        dst[offset] = u8::from(self.{fname}.is_some());\n"),
        );
        code.push_str("        offset += 1;\n");
        push_fmt(
            &mut code,
            format_args!("        if let Some(ref value) = self.{fname} {{\n"),
        );

        // Alignment for the value (after presence flag)
        if alignment > 1 {
            push_fmt(
                &mut code,
                format_args!(
                    "            let padding = ({alignment} - (offset % {alignment})) % {alignment};\n"
                ),
            );
            code.push_str(
                "            if dst.len() < offset + padding { return Err(CdrError::BufferTooSmall); }\n",
            );
            code.push_str("            dst[offset..offset+padding].fill(0);\n");
            code.push_str("            offset += padding;\n");
        }

        // Encode the inner value using `value` expression
        match &field.field_type {
            IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString) => {
                Self::encode_string_expr(&mut code, "            ", "value");
            }
            IdlType::Primitive(PrimitiveType::Boolean) => {
                Self::encode_buffer_check(&mut code, "            ", "1");
                code.push_str("            dst[offset] = u8::from(*value);\n");
                code.push_str("            offset += 1;\n\n");
            }
            IdlType::Primitive(PrimitiveType::Octet | PrimitiveType::UInt8) => {
                Self::encode_buffer_check(&mut code, "            ", "1");
                code.push_str("            dst[offset] = *value;\n");
                code.push_str("            offset += 1;\n\n");
            }
            IdlType::Primitive(PrimitiveType::Int8) => {
                Self::encode_buffer_check(&mut code, "            ", "1");
                code.push_str("            dst[offset] = value.to_le_bytes()[0];\n");
                code.push_str("            offset += 1;\n\n");
            }
            IdlType::Primitive(PrimitiveType::Char) => {
                Self::encode_buffer_check(&mut code, "            ", "1");
                code.push_str("            dst[offset] = u8::try_from(*value as u32).map_err(|_| CdrError::InvalidEncoding)?;\n");
                code.push_str("            offset += 1;\n\n");
            }
            IdlType::Primitive(p) => {
                // Numeric types (2, 4, 8 bytes)
                if let Some(size) = Self::cdr2_fixed_size(&IdlType::Primitive(*p)) {
                    Self::encode_buffer_check(&mut code, "            ", &size.to_string());
                    push_fmt(
                        &mut code,
                        format_args!(
                            "            dst[offset..offset+{size}].copy_from_slice(&value.to_le_bytes());\n"
                        ),
                    );
                    push_fmt(&mut code, format_args!("            offset += {size};\n\n"));
                }
            }
            IdlType::Sequence { inner, .. }
                if matches!(
                    **inner,
                    IdlType::Primitive(PrimitiveType::Char | PrimitiveType::WChar)
                ) =>
            {
                // Bounded string (string<N>) - encode as string
                Self::encode_string_expr(&mut code, "            ", "value");
            }
            _ => {
                // Named, Sequence, Array, Map - delegate to Cdr2Encode
                code.push_str(
                    "            let used = value.encode_cdr2_le(&mut dst[offset..])?;\n",
                );
                code.push_str("            offset += used;\n\n");
            }
        }

        code.push_str("        }\n\n");
        code
    }

    /// Compact `PL_CDR2` encoder for simple mutable structs composed only of
    /// primitive, non-optional fields (e.g., `Point3D`).
    ///
    /// Layout (per member):
    ///   `EMHEADER1` (LC based on fixed size, no `NEXTINT`) + payload bytes
    fn emit_pl_cdr2_compact_encode_impl(s: &Struct) -> String {
        let mut code = String::new();

        push_fmt(
            &mut code,
            format_args!("impl Cdr2Encode for {} {{\n", s.name),
        );
        code.push_str(
            "    fn encode_cdr2_le(&self, dst: &mut [u8]) -> Result<usize, CdrError> {\n",
        );
        code.push_str("        let mut offset: usize = 0;\n\n");

        for (idx, field) in s.fields.iter().enumerate() {
            if field.is_non_serialized() {
                continue;
            }
            let member_id = Self::compute_member_id(s, idx, field);
            let fixed = Self::cdr2_fixed_size(&field.field_type).unwrap_or(0);
            let lc: u32 = match fixed {
                1 => 0,
                2 => 1,
                4 => 2,
                8 => 3,
                _ => 5, // fallback (NEXTINT) - not expected for compact structs
            };

            code.push_str(
                "        if dst.len() < offset + 4 { return Err(CdrError::BufferTooSmall); }\n",
            );
            let mu_bit = if field.is_key() || field.is_must_understand() {
                "0x8000_0000u32 | "
            } else {
                ""
            };
            push_fmt(
                &mut code,
                format_args!(
                    "        let emheader = {mu_bit}({lc}u32 << 28) | ({member_id:#010X}u32 & 0x0fff_ffff);\n"
                ),
            );
            code.push_str(
                "        dst[offset..offset+4].copy_from_slice(&emheader.to_le_bytes());\n",
            );
            code.push_str("        offset += 4;\n");

            // Payload: reuse primitive encoder without extra alignment.
            let ident = super::super::keywords::rust_ident(&field.name);
            if let IdlType::Primitive(p) = &field.field_type {
                code.push_str(&Self::encode_primitive_compact(
                    p, &ident, false, "        ",
                ));
            }
        }

        code.push_str("        Ok(offset)\n");
        code.push_str("    }\n\n");

        // max_cdr2_size = sum of EMHEADER1 (4 bytes) + fixed payload per field
        code.push_str("    fn max_cdr2_size(&self) -> usize {\n");
        let mut size_expr = String::new();
        for field in &s.fields {
            if field.is_non_serialized() {
                continue;
            }
            let fixed = Self::cdr2_fixed_size(&field.field_type).unwrap_or(0);
            if !size_expr.is_empty() {
                size_expr.push_str(" + ");
            }
            push_fmt(&mut size_expr, format_args!("4 + {fixed}"));
        }
        if size_expr.is_empty() {
            size_expr.push('0');
        }
        push_fmt(&mut code, format_args!("        {size_expr}\n"));
        code.push_str("    }\n");
        code.push_str("}\n\n");

        code
    }

    #[allow(clippy::too_many_lines)]
    fn emit_pl_cdr2_encode_impl(s: &Struct) -> String {
        let mut code = String::new();

        push_fmt(
            &mut code,
            format_args!("impl Cdr2Encode for {} {{\n", s.name),
        );
        code.push_str(
            "    fn encode_cdr2_le(&self, dst: &mut [u8]) -> Result<usize, CdrError> {\n",
        );
        code.push_str("        let mut offset: usize = 0;\n\n");
        code.push_str("        if dst.len() < 4 { return Err(CdrError::BufferTooSmall); }\n");
        code.push_str("        offset += 4; // reserve DHEADER\n\n");

        for (idx, field) in s.fields.iter().enumerate() {
            if field.is_non_serialized() {
                continue;
            }
            let alignment = Self::cdr2_alignment(&field.field_type);
            let member_id = Self::compute_member_id(s, idx, field);
            let ident = super::super::keywords::rust_ident(&field.name);

            if field.is_optional() {
                push_fmt(
                    &mut code,
                    format_args!("        if let Some(value) = &self.{ident} {{\n"),
                );
            }

            // Choose LengthCode (LC) and whether to emit NEXTINT based on field type.
            let (lc, use_nextint) = match &field.field_type {
                IdlType::Primitive(p) => {
                    let fixed = Self::cdr2_fixed_size(&IdlType::Primitive(*p)).unwrap_or(0);
                    match fixed {
                        1 => (0u32, false),
                        2 => (1u32, false),
                        4 => (2u32, false),
                        8 => (3u32, false),
                        _ => (5u32, true),
                    }
                }
                _ => (5u32, true),
            };

            if use_nextint {
                code.push_str(
                    "        if dst.len() < offset + 8 { return Err(CdrError::BufferTooSmall); }\n",
                );
            } else {
                code.push_str(
                    "        if dst.len() < offset + 4 { return Err(CdrError::BufferTooSmall); }\n",
                );
            }
            let mu_bit = if field.is_key() || field.is_must_understand() {
                "0x8000_0000u32 | "
            } else {
                ""
            };
            push_fmt(
                &mut code,
                format_args!(
                    "        let emheader = {mu_bit}({lc}u32 << 28) | ({member_id:#010X}u32 & 0x0fff_ffff);\n"
                ),
            );
            code.push_str(
                "        dst[offset..offset+4].copy_from_slice(&emheader.to_le_bytes());\n",
            );
            code.push_str("        offset += 4;\n");

            if use_nextint {
                code.push_str("        let member_len_pos = offset;\n");
                code.push_str("        offset += 4; // reserve NEXTINT (member length)\n");
            }
            code.push_str("        let member_start = offset;\n");

            match &field.field_type {
                IdlType::Primitive(p) => {
                    // Compact primitives: no extra struct-level alignment here.
                    // For LC=5 primitives, use inline encoder (with NEXTINT).
                    // For LC<4 primitives, use compact encoder (no NEXTINT, no extra alignment).
                    if matches!(lc, 5u32) {
                        code.push_str(&Self::encode_primitive_inline(
                            p,
                            &ident,
                            field.is_optional(),
                            "        ",
                        ));
                    } else {
                        code.push_str(&Self::encode_primitive_compact(
                            p,
                            &ident,
                            field.is_optional(),
                            "        ",
                        ));
                    }
                }
                // Non-primitive fields: keep alignment before payload.
                IdlType::Sequence { inner, .. } => {
                    push_fmt(
                        &mut code,
                        format_args!(
                            "        let padding = ({alignment} - (offset % {alignment})) % {alignment};\n"
                        ),
                    );
                    code.push_str(
                        "        if dst.len() < offset + padding { return Err(CdrError::BufferTooSmall); }\n",
                    );
                    code.push_str("        dst[offset..offset+padding].fill(0);\n");
                    code.push_str("        offset += padding;\n");
                    if let IdlType::Named(name) = &**inner {
                        if super::helpers::is_named_mutable(name) && !field.is_optional() {
                            // PL_CDR2 sequence of mutable structs: DHEADER per element + nested PL payload
                            push_fmt(
                                &mut code,
                                format_args!(
                                    "        let count_u32 = u32::try_from(self.{fname}.len()).map_err(|_| CdrError::InvalidEncoding)?;\n",
                                    fname = ident
                                ),
                            );
                            Self::encode_buffer_check(&mut code, "        ", "4");
                            code.push_str("        dst[offset..offset+4].copy_from_slice(&count_u32.to_le_bytes());\n");
                            code.push_str("        offset += 4;\n");
                            push_fmt(
                                &mut code,
                                format_args!(
                                    "        for elem in &self.{fname} {{\n",
                                    fname = ident
                                ),
                            );
                            Self::encode_buffer_check(&mut code, "            ", "4");
                            code.push_str("            let elem_start = offset;\n");
                            code.push_str("            offset += 4; // DHEADER per element\n");
                            code.push_str(
                                "            let used = elem.encode_cdr2_le(&mut dst[offset..])?;\n",
                            );
                            code.push_str("            offset += used;\n");
                            code.push_str("            let elem_len = u32::try_from(offset - (elem_start + 4)).map_err(|_| CdrError::InvalidEncoding)?;\n");
                            code.push_str(
                                "            dst[elem_start..elem_start+4].copy_from_slice(&elem_len.to_le_bytes());\n",
                            );
                            code.push_str("        }\n\n");
                        } else {
                            let value_expr = if field.is_optional() {
                                "value".to_string()
                            } else {
                                format!("self.{}", ident)
                            };
                            push_fmt(
                                &mut code,
                                format_args!(
                                    "        let used = {value_expr}.encode_cdr2_le(&mut dst[offset..])?;\n",
                                    value_expr = value_expr
                                ),
                            );
                            code.push_str("        offset += used;\n\n");
                        }
                    } else {
                        let value_expr = if field.is_optional() {
                            "value".to_string()
                        } else {
                            format!("self.{}", ident)
                        };
                        push_fmt(
                            &mut code,
                            format_args!(
                                "        let used = {value_expr}.encode_cdr2_le(&mut dst[offset..])?;\n",
                                value_expr = value_expr
                            ),
                        );
                        code.push_str("        offset += used;\n\n");
                    }
                }
                _ => {
                    push_fmt(
                        &mut code,
                        format_args!(
                            "        let padding = ({alignment} - (offset % {alignment})) % {alignment};\n"
                        ),
                    );
                    code.push_str(
                        "        if dst.len() < offset + padding { return Err(CdrError::BufferTooSmall); }\n",
                    );
                    code.push_str("        dst[offset..offset+padding].fill(0);\n");
                    code.push_str("        offset += padding;\n");

                    let value_expr = if field.is_optional() {
                        "value".to_string()
                    } else {
                        format!("self.{}", ident)
                    };
                    push_fmt(
                        &mut code,
                        format_args!(
                            "        let used = {value_expr}.encode_cdr2_le(&mut dst[offset..])?;\n",
                            value_expr = value_expr
                        ),
                    );
                    code.push_str("        offset += used;\n\n");
                }
            }

            // Fill NEXTINT (member length) for LC=5 only
            if use_nextint {
                code.push_str("        let member_len = offset - member_start;\n");
                code.push_str(
                    "        let member_len_u32 = u32::try_from(member_len).map_err(|_| CdrError::InvalidEncoding)?;\n",
                );
                code.push_str(
                    "        dst[member_len_pos..member_len_pos+4].copy_from_slice(&member_len_u32.to_le_bytes());\n",
                );
            }

            if field.is_optional() {
                code.push_str("        }\n\n");
            }
        }

        code.push_str(
            "        let payload_len = u32::try_from(offset - 4).map_err(|_| CdrError::InvalidEncoding)?;\n",
        );
        code.push_str("        dst[..4].copy_from_slice(&payload_len.to_le_bytes());\n");
        code.push_str("        Ok(offset)\n");
        code.push_str("    }\n\n");

        code.push_str("    fn max_cdr2_size(&self) -> usize {\n");
        code.push_str("        let mut size = 4; // DHEADER\n");
        for field in &s.fields {
            if field.is_non_serialized() {
                continue;
            }
            let ident = super::super::keywords::rust_ident(&field.name);
            let alignment = Self::cdr2_alignment(&field.field_type);
            let pad = alignment.saturating_sub(1);
            let add = match &field.field_type {
                IdlType::Primitive(p) => {
                    Self::cdr2_fixed_size(&IdlType::Primitive(*p)).unwrap_or(8)
                }
                IdlType::Sequence { .. } => 4 + 256, // delimiter + conservative payload
                IdlType::Array { .. } | IdlType::Named(_) => 64,
                IdlType::Map { .. } => 128,
            };
            if field.is_optional() {
                push_fmt(
                    &mut code,
                    format_args!(
                        "        if let Some(_value) = &self.{fname} {{ size += 4 + {pad} + {add}; }}\n",
                        fname = ident,
                        pad = pad,
                        add = add
                    ),
                );
            } else {
                push_fmt(
                    &mut code,
                    format_args!("        size += 4 + {pad} + {add};\n", pad = pad, add = add),
                );
            }
        }
        code.push_str("        size\n");
        code.push_str("    }\n");
        code.push_str("}\n\n");

        code
    }

    fn append_encode_field(dst: &mut String, field: &Field) {
        let fname = super::super::keywords::rust_ident(&field.name);
        match &field.field_type {
            IdlType::Primitive(p) => Self::append_encode_primitive(dst, &fname, p),
            IdlType::Sequence { inner, .. } => {
                // Bounded string (string<N>) is represented as Sequence<Char>
                if matches!(
                    **inner,
                    IdlType::Primitive(PrimitiveType::Char | PrimitiveType::WChar)
                ) {
                    Self::append_encode_primitive(dst, &fname, &PrimitiveType::String);
                } else {
                    Self::append_encode_sequence(dst, &fname, inner);
                }
            }
            IdlType::Array { inner, size } => {
                Self::append_encode_array(dst, &fname, inner, *size);
            }
            IdlType::Map { key, value, .. } => {
                Self::append_encode_map(dst, &fname, key, value);
            }
            IdlType::Named(name) => Self::append_encode_named(dst, &fname, name),
        }
    }

    /// Inline primitive encoder for `PL_CDR2` (avoids calling `encode_cdr2_le` on primitives).
    #[allow(clippy::too_many_lines)]
    fn encode_primitive_inline(
        p: &PrimitiveType,
        field_name: &str,
        is_optional: bool,
        indent: &str,
    ) -> String {
        let mut out = String::new();
        match p {
            PrimitiveType::Octet
            | PrimitiveType::UInt8
            | PrimitiveType::Int8
            | PrimitiveType::Boolean
            | PrimitiveType::Char => {
                push_fmt(
                    &mut out,
                    format_args!(
                        "{indent}if dst.len() < offset + 1 {{ return Err(CdrError::BufferTooSmall); }}\n"
                    ),
                );
                let expr = if is_optional {
                    "u8::from(*value)".to_string()
                } else {
                    format!("u8::from(self.{field_name})")
                };
                push_fmt(&mut out, format_args!("{indent}dst[offset] = {expr};\n"));
                push_fmt(&mut out, format_args!("{indent}offset += 1;\n\n"));
            }
            PrimitiveType::Short
            | PrimitiveType::UnsignedShort
            | PrimitiveType::Int16
            | PrimitiveType::UInt16 => {
                let align = 2;
                push_fmt(
                    &mut out,
                    format_args!("{indent}let pad = ({align} - (offset % {align})) % {align};\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!(
                        "{indent}if dst.len() < offset + pad + 2 {{ return Err(CdrError::BufferTooSmall); }}\n"
                    ),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}dst[offset..offset+pad].fill(0);\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += pad;\n"));
                let expr = if is_optional {
                    "value.to_le_bytes()".to_string()
                } else {
                    format!("self.{field_name}.to_le_bytes()")
                };
                push_fmt(
                    &mut out,
                    format_args!("{indent}dst[offset..offset+2].copy_from_slice(&{expr});\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += 2;\n\n"));
            }
            PrimitiveType::Long
            | PrimitiveType::UnsignedLong
            | PrimitiveType::Int32
            | PrimitiveType::UInt32
            | PrimitiveType::Float
            | PrimitiveType::WChar => {
                let align = 4;
                push_fmt(
                    &mut out,
                    format_args!("{indent}let pad = ({align} - (offset % {align})) % {align};\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!(
                        "{indent}if dst.len() < offset + pad + 4 {{ return Err(CdrError::BufferTooSmall); }}\n"
                    ),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}dst[offset..offset+pad].fill(0);\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += pad;\n"));
                let expr = if is_optional {
                    "value.to_le_bytes()".to_string()
                } else {
                    format!("self.{field_name}.to_le_bytes()")
                };
                push_fmt(
                    &mut out,
                    format_args!("{indent}dst[offset..offset+4].copy_from_slice(&{expr});\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += 4;\n\n"));
            }
            PrimitiveType::LongLong
            | PrimitiveType::UnsignedLongLong
            | PrimitiveType::Int64
            | PrimitiveType::UInt64
            | PrimitiveType::Double
            | PrimitiveType::LongDouble => {
                let align = 8;
                push_fmt(
                    &mut out,
                    format_args!("{indent}let pad = ({align} - (offset % {align})) % {align};\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!(
                        "{indent}if dst.len() < offset + pad + 8 {{ return Err(CdrError::BufferTooSmall); }}\n"
                    ),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}dst[offset..offset+pad].fill(0);\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += pad;\n"));
                let expr = if is_optional {
                    "value.to_le_bytes()".to_string()
                } else {
                    format!("self.{field_name}.to_le_bytes()")
                };
                push_fmt(
                    &mut out,
                    format_args!("{indent}dst[offset..offset+8].copy_from_slice(&{expr});\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += 8;\n\n"));
            }
            PrimitiveType::String | PrimitiveType::WString | PrimitiveType::Fixed { .. } => {
                let expr = if is_optional {
                    "value".to_string()
                } else {
                    format!("&self.{field_name}")
                };
                push_fmt(
                    &mut out,
                    format_args!(
                        "{indent}let used = {expr}.encode_cdr2_le(&mut dst[offset..])?;\n"
                    ),
                );
                push_fmt(&mut out, format_args!("{indent}offset += used;\n\n"));
            }
            PrimitiveType::Void => {}
        }
        out
    }

    /// Compact primitive encoder used by compact `PL_CDR2` structs and
    /// `LC<4` members in `PL_CDR2` (no `NEXTINT`, no extra alignment).
    fn encode_primitive_compact(
        p: &PrimitiveType,
        field_name: &str,
        is_optional: bool,
        indent: &str,
    ) -> String {
        let mut out = String::new();
        match p {
            PrimitiveType::Octet
            | PrimitiveType::UInt8
            | PrimitiveType::Int8
            | PrimitiveType::Boolean
            | PrimitiveType::Char => {
                Self::encode_buffer_check(&mut out, indent, "1");
                let expr = if is_optional {
                    match p {
                        PrimitiveType::Boolean => "u8::from(*value)".to_string(),
                        PrimitiveType::Char => {
                            "u8::try_from((*value) as u32).map_err(|_| CdrError::InvalidEncoding)?"
                                .to_string()
                        }
                        _ => "(*value) as u8".to_string(),
                    }
                } else {
                    match p {
                        PrimitiveType::Boolean => format!("u8::from(self.{field_name})"),
                        PrimitiveType::Char => format!(
                            "u8::try_from(self.{field_name} as u32).map_err(|_| CdrError::InvalidEncoding)?"
                        ),
                        _ => format!("self.{field_name} as u8"),
                    }
                };
                push_fmt(&mut out, format_args!("{indent}dst[offset] = {expr};\n"));
                push_fmt(&mut out, format_args!("{indent}offset += 1;\n\n"));
            }
            PrimitiveType::Short
            | PrimitiveType::UnsignedShort
            | PrimitiveType::Int16
            | PrimitiveType::UInt16
            | PrimitiveType::Long
            | PrimitiveType::UnsignedLong
            | PrimitiveType::Int32
            | PrimitiveType::UInt32
            | PrimitiveType::Float
            | PrimitiveType::LongLong
            | PrimitiveType::UnsignedLongLong
            | PrimitiveType::Int64
            | PrimitiveType::UInt64
            | PrimitiveType::Double
            | PrimitiveType::LongDouble => {
                if let Some(size) = Self::cdr2_fixed_size(&IdlType::Primitive(*p)) {
                    let size_expr = size.to_string();
                    Self::encode_buffer_check(&mut out, indent, &size_expr);
                    let expr = if is_optional {
                        "(*value)".to_string()
                    } else {
                        format!("self.{field_name}")
                    };
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}dst[offset..offset+{size}].copy_from_slice(&{expr}.to_le_bytes());\n"
                        ),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += {size};\n\n"));
                }
            }
            PrimitiveType::String | PrimitiveType::WString | PrimitiveType::Fixed { .. } => {
                // For compact structs we don't expect variable-size primitives.
                push_fmt(
                    &mut out,
                    format_args!("{indent}return Err(CdrError::InvalidEncoding);\n"),
                );
            }
            PrimitiveType::Void | PrimitiveType::WChar => {}
        }
        out
    }

    fn append_encode_primitive(dst: &mut String, field_name: &str, primitive: &PrimitiveType) {
        match primitive {
            PrimitiveType::Octet | PrimitiveType::UInt8 => Self::encode_u8(dst, field_name),
            PrimitiveType::Int8 => Self::encode_i8(dst, field_name),
            PrimitiveType::Char => Self::encode_char(dst, field_name),
            PrimitiveType::WChar => Self::encode_wchar(dst, field_name),
            PrimitiveType::Boolean => Self::encode_boolean(dst, field_name),
            PrimitiveType::Short
            | PrimitiveType::UnsignedShort
            | PrimitiveType::Int16
            | PrimitiveType::UInt16
            | PrimitiveType::Long
            | PrimitiveType::UnsignedLong
            | PrimitiveType::Int32
            | PrimitiveType::UInt32
            | PrimitiveType::Float
            | PrimitiveType::LongLong
            | PrimitiveType::UnsignedLongLong
            | PrimitiveType::Int64
            | PrimitiveType::UInt64
            | PrimitiveType::Double
            | PrimitiveType::LongDouble => Self::encode_numeric(dst, field_name, *primitive),
            PrimitiveType::Fixed { digits, scale } => {
                Self::encode_fixed(dst, field_name, *digits, *scale);
            }
            PrimitiveType::String | PrimitiveType::WString => Self::encode_string(dst, field_name),
            PrimitiveType::Void => Self::encode_void(dst, field_name),
        }
    }

    fn encode_u8(dst: &mut String, field_name: &str) {
        Self::encode_buffer_check(dst, "        ", "1");
        push_fmt(
            dst,
            format_args!("        dst[offset] = self.{field_name};\n"),
        );
        dst.push_str("        offset += 1;\n\n");
    }

    fn encode_i8(dst: &mut String, field_name: &str) {
        Self::encode_buffer_check(dst, "        ", "1");
        push_fmt(
            dst,
            format_args!("        dst[offset] = self.{field_name}.to_le_bytes()[0];\n"),
        );
        dst.push_str("        offset += 1;\n\n");
    }

    fn encode_char(dst: &mut String, field_name: &str) {
        Self::encode_buffer_check(dst, "        ", "1");
        push_fmt(
            dst,
            format_args!("        let scalar = u32::from(self.{field_name});\n"),
        );
        push_fmt(
            dst,
            format_args!(
                "        let byte = u8::try_from(scalar).map_err(|_| CdrError::InvalidEncoding)?;\n"
            ),
        );
        push_fmt(dst, format_args!("        dst[offset] = byte;\n"));
        dst.push_str("        offset += 1;\n\n");
    }

    fn encode_wchar(dst: &mut String, field_name: &str) {
        Self::encode_buffer_check(dst, "        ", "4");
        push_fmt(
            dst,
            format_args!("        let scalar = u32::from(self.{field_name});\n"),
        );
        push_fmt(
            dst,
            format_args!("        dst[offset..offset+4].copy_from_slice(&scalar.to_le_bytes());\n"),
        );
        dst.push_str("        offset += 4;\n\n");
    }

    fn encode_boolean(dst: &mut String, field_name: &str) {
        Self::encode_buffer_check(dst, "        ", "1");
        push_fmt(
            dst,
            format_args!("        dst[offset] = u8::from(self.{field_name});\n"),
        );
        dst.push_str("        offset += 1;\n\n");
    }

    fn encode_numeric(dst: &mut String, field_name: &str, primitive: PrimitiveType) {
        let Some(size) = Self::cdr2_fixed_size(&IdlType::Primitive(primitive)) else {
            push_fmt(
                dst,
                format_args!("        return Err(CdrError::InvalidEncoding);\n"),
            );
            dst.push('\n');
            return;
        };
        let size_expr = size.to_string();
        Self::encode_buffer_check(dst, "        ", &size_expr);
        push_fmt(
            dst,
            format_args!(
                "        dst[offset..offset+{size}].copy_from_slice(&self.{field_name}.to_le_bytes());\n"
            ),
        );
        push_fmt(dst, format_args!("        offset += {size};\n\n"));
    }

    fn encode_fixed(dst: &mut String, field_name: &str, digits: u32, scale: u32) {
        push_fmt(
            dst,
            format_args!("        // Encode fixed<{digits}, {scale}> field '{field_name}'\n"),
        );
        Self::encode_buffer_check(dst, "        ", "16");
        push_fmt(
            dst,
            format_args!("        let raw = self.{field_name}.raw();\n"),
        );
        push_fmt(
            dst,
            format_args!("        dst[offset..offset+16].copy_from_slice(&raw.to_le_bytes());\n"),
        );
        dst.push_str("        offset += 16;\n\n");
    }

    fn encode_string(dst: &mut String, field_name: &str) {
        push_fmt(
            dst,
            format_args!("        // Encode String field '{field_name}'\n"),
        );
        Self::encode_string_expr(dst, "        ", &format!("self.{field_name}"));
    }

    pub(super) fn encode_string_expr(dst: &mut String, indent: &str, expr: &str) {
        push_fmt(
            dst,
            format_args!("{indent}let bytes = {expr}.as_bytes();\n"),
        );
        push_fmt(dst, format_args!("{indent}let len = bytes.len();\n"));
        push_fmt(
            dst,
            format_args!(
                "{indent}let len_u32 = u32::try_from(len).map_err(|_| CdrError::InvalidEncoding)?;\n"
            ),
        );
        Self::encode_buffer_check(dst, indent, "4 + len + 1");
        push_fmt(
            dst,
            format_args!(
                "{indent}dst[offset..offset+4].copy_from_slice(&(len_u32 + 1).to_le_bytes()); // CDR: length includes null terminator\n"
            ),
        );
        push_fmt(dst, format_args!("{indent}offset += 4;\n"));
        push_fmt(
            dst,
            format_args!("{indent}dst[offset..offset+len].copy_from_slice(bytes);\n"),
        );
        push_fmt(dst, format_args!("{indent}offset += len;\n"));
        push_fmt(
            dst,
            format_args!("{indent}dst[offset] = 0; // null terminator\n"),
        );
        push_fmt(dst, format_args!("{indent}offset += 1;\n\n"));
    }

    fn encode_void(dst: &mut String, field_name: &str) {
        push_fmt(
            dst,
            format_args!("        // Encode void field '{field_name}' (no payload)\n"),
        );
        dst.push_str("        // nothing to encode\n\n");
    }

    fn append_encode_named(dst: &mut String, field_name: &str, type_name: &str) {
        push_fmt(
            dst,
            format_args!("        // Encode named field '{field_name}' of type '{type_name}'\n"),
        );
        push_fmt(
            dst,
            format_args!(
                "        let used = self.{field_name}.encode_cdr2_le(&mut dst[offset..])?;\n"
            ),
        );
        dst.push_str("        offset += used;\n\n");
    }

    pub(super) fn encode_buffer_check(dst: &mut String, indent: &str, size_expr: &str) {
        push_fmt(
            dst,
            format_args!("{indent}if dst.len() < offset + {size_expr} {{\n"),
        );
        push_fmt(
            dst,
            format_args!("{indent}    return Err(CdrError::BufferTooSmall);\n"),
        );
        push_fmt(dst, format_args!("{indent}}}\n"));
    }
}
