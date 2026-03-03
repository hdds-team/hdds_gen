// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! CDR2 decode trait generation for Rust structs.
//!
//! Emits `Cdr2Decode` trait implementations for deserialization.

#![allow(clippy::uninlined_format_args)]

use super::{push_fmt, RustGenerator};
use crate::ast::{Field, Struct};
use crate::types::{IdlType, PrimitiveType};

impl RustGenerator {
    /// Emit `Cdr2Decode` trait implementation
    pub(super) fn emit_cdr2_decode_impl(s: &Struct) -> String {
        if super::helpers::is_compact_mutable_struct(s) {
            return Self::emit_pl_cdr2_compact_decode_impl(s);
        }

        if super::helpers::is_mutable_struct(s) {
            return Self::emit_pl_cdr2_decode_impl(s);
        }

        let mut code = String::new();

        push_fmt(
            &mut code,
            format_args!("impl Cdr2Decode for {} {{\n", s.name),
        );
        code.push_str("    fn decode_cdr2_le(src: &[u8]) -> Result<(Self, usize), CdrError> {\n");
        code.push_str("        let mut offset: usize = 0;\n\n");

        // Decode each field with alignment
        for field in &s.fields {
            if field.is_non_serialized() {
                continue;
            }
            if field.is_optional() {
                code.push_str(&Self::emit_optional_field_decode(field));
            } else {
                let alignment = Self::cdr2_alignment(&field.field_type);

                // Insert padding if needed
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

                code.push_str(&Self::emit_decode_field(field));
            }
        }

        // Construct struct and return with consumed bytes
        code.push_str("        Ok((Self {\n");
        for field in &s.fields {
            let fname = super::super::keywords::rust_ident(&field.name);
            if field.is_non_serialized() {
                push_fmt(
                    &mut code,
                    format_args!("            {fname}: Default::default(),\n"),
                );
            } else if field.is_external() {
                push_fmt(
                    &mut code,
                    format_args!("            {fname}: Box::new({fname}),\n"),
                );
            } else {
                push_fmt(&mut code, format_args!("            {fname},\n"));
            }
        }
        code.push_str("        }, offset))\n");
        code.push_str("    }\n");
        code.push_str("}\n\n");

        code
    }

    /// Emit field decoding logic
    #[must_use]
    fn emit_decode_field(field: &Field) -> String {
        let mut code = String::new();
        Self::append_decode_field(&mut code, field);
        code
    }

    /// Decode an `@optional` field in standard CDR2: 1-byte presence flag + value.
    // codegen function - line count from template output
    #[allow(clippy::too_many_lines)]
    fn emit_optional_field_decode(field: &Field) -> String {
        let mut code = String::new();
        let fname = super::super::keywords::rust_ident(&field.name);
        let field_ty = Self::type_to_rust(&field.field_type);
        let alignment = Self::cdr2_alignment(&field.field_type);

        // Read presence flag and decode value in a single let-binding (avoids needless_late_init)
        Self::decode_buffer_check(&mut code, "        ", "1");
        push_fmt(
            &mut code,
            format_args!("        let {fname}: Option<{field_ty}> = if src[offset] != 0 {{\n"),
        );
        code.push_str("            offset += 1;\n");

        // Alignment for value (after presence flag)
        if alignment > 1 {
            push_fmt(
                &mut code,
                format_args!(
                    "            let padding = ({alignment} - (offset % {alignment})) % {alignment};\n"
                ),
            );
            code.push_str("            offset += padding;\n");
        }

        // Decode value based on type - Some(...) must be the last expression in each branch
        match &field.field_type {
            IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString) => {
                Self::decode_buffer_check(&mut code, "            ", "4");
                code.push_str(
                    "            let __hdds_len = {\n                let mut __hdds_tmp = [0u8; 4];\n                __hdds_tmp.copy_from_slice(&src[offset..offset+4]);\n                u32::from_le_bytes(__hdds_tmp) as usize\n            };\n",
                );
                code.push_str("            offset += 4;\n");
                Self::decode_buffer_check(&mut code, "            ", "__hdds_len + 1");
                code.push_str(
                    "            let __hdds_s = std::str::from_utf8(&src[offset..offset+__hdds_len])\n                .map_err(|_| CdrError::InvalidEncoding)?;\n",
                );
                code.push_str("            offset += __hdds_len + 1; // skip null terminator\n");
                code.push_str("            Some(__hdds_s.to_string())\n");
            }
            IdlType::Primitive(PrimitiveType::Boolean) => {
                Self::decode_buffer_check(&mut code, "            ", "1");
                code.push_str("            let __hdds_v = src[offset] != 0;\n");
                code.push_str("            offset += 1;\n");
                code.push_str("            Some(__hdds_v)\n");
            }
            IdlType::Primitive(PrimitiveType::Octet | PrimitiveType::UInt8) => {
                Self::decode_buffer_check(&mut code, "            ", "1");
                code.push_str("            let __hdds_v = src[offset];\n");
                code.push_str("            offset += 1;\n");
                code.push_str("            Some(__hdds_v)\n");
            }
            IdlType::Primitive(PrimitiveType::Int8) => {
                Self::decode_buffer_check(&mut code, "            ", "1");
                code.push_str("            let __hdds_v = i8::from_le_bytes([src[offset]]);\n");
                code.push_str("            offset += 1;\n");
                code.push_str("            Some(__hdds_v)\n");
            }
            IdlType::Primitive(PrimitiveType::Char) => {
                Self::decode_buffer_check(&mut code, "            ", "1");
                code.push_str("            let __hdds_v = char::from(src[offset]);\n");
                code.push_str("            offset += 1;\n");
                code.push_str("            Some(__hdds_v)\n");
            }
            IdlType::Primitive(p) => {
                // Numeric types (2, 4, 8 bytes)
                if let Some(size) = Self::cdr2_fixed_size(&IdlType::Primitive(*p)) {
                    let rust_type = Self::type_to_rust(&IdlType::Primitive(*p));
                    Self::decode_buffer_check(&mut code, "            ", &size.to_string());
                    push_fmt(
                        &mut code,
                        format_args!(
                            "            let __hdds_v = {{\n                let mut __hdds_tmp = [0u8; {size}];\n                __hdds_tmp.copy_from_slice(&src[offset..offset+{size}]);\n                {rust_type}::from_le_bytes(__hdds_tmp)\n            }};\n"
                        ),
                    );
                    push_fmt(&mut code, format_args!("            offset += {size};\n"));
                    code.push_str("            Some(__hdds_v)\n");
                }
            }
            IdlType::Sequence { inner, .. }
                if matches!(
                    **inner,
                    IdlType::Primitive(PrimitiveType::Char | PrimitiveType::WChar)
                ) =>
            {
                // Bounded string (string<N>) - decode as String
                Self::decode_buffer_check(&mut code, "            ", "4");
                code.push_str(
                    "            let __hdds_len = {\n                let mut __hdds_tmp = [0u8; 4];\n                __hdds_tmp.copy_from_slice(&src[offset..offset+4]);\n                u32::from_le_bytes(__hdds_tmp) as usize\n            };\n",
                );
                code.push_str("            offset += 4;\n");
                Self::decode_buffer_check(&mut code, "            ", "__hdds_len + 1");
                code.push_str(
                    "            let __hdds_s = std::str::from_utf8(&src[offset..offset+__hdds_len])\n                .map_err(|_| CdrError::InvalidEncoding)?;\n",
                );
                code.push_str("            offset += __hdds_len + 1;\n");
                code.push_str("            Some(__hdds_s.to_string())\n");
            }
            _ => {
                // Named, Sequence, Array, Map - delegate to Cdr2Decode
                let ty = Self::type_to_rust(&field.field_type);
                push_fmt(
                    &mut code,
                    format_args!(
                        "            let (__hdds_val, __hdds_used) = <{ty}>::decode_cdr2_le(&src[offset..])?;\n"
                    ),
                );
                code.push_str("            offset += __hdds_used;\n");
                code.push_str("            Some(__hdds_val)\n");
            }
        }

        code.push_str("        } else {\n");
        code.push_str("            offset += 1;\n");
        code.push_str("            None\n");
        code.push_str("        };\n\n");

        code
    }

    /// Compact `PL_CDR2` decoder for simple mutable structs composed only of
    /// primitive, non-optional fields (e.g., `Point3D`).
    ///
    /// Assumes layout: `[EMHEADER1][payload]...` without inner `DHEADER`.
    fn emit_pl_cdr2_compact_decode_impl(s: &Struct) -> String {
        let mut code = String::new();

        push_fmt(
            &mut code,
            format_args!("impl Cdr2Decode for {} {{\n", s.name),
        );
        code.push_str("    fn decode_cdr2_le(src: &[u8]) -> Result<(Self, usize), CdrError> {\n");
        code.push_str("        let mut offset: usize = 0;\n\n");

        for field in &s.fields {
            if field.is_non_serialized() {
                continue;
            }
            let ident = super::super::keywords::rust_ident(&field.name);
            let field_ty = Self::type_to_rust(&field.field_type);
            push_fmt(
                &mut code,
                format_args!("        let {fname}: {field_ty};\n", fname = ident),
            );
        }
        code.push('\n');

        for (idx, field) in s.fields.iter().enumerate() {
            if field.is_non_serialized() {
                continue;
            }
            let ident = super::super::keywords::rust_ident(&field.name);
            let member_id = Self::compute_member_id(s, idx, field);
            code.push_str(
                "        if src.len() < offset + 4 { return Err(CdrError::UnexpectedEof); }\n",
            );
            code.push_str(
                "        let em = u32::from_le_bytes(src[offset..offset+4].try_into().unwrap());\n",
            );
            code.push_str("        offset += 4;\n");
            code.push_str("        let member_id = em & 0x0fff_ffff;\n");
            push_fmt(
                &mut code,
                format_args!(
                    "        if member_id != {member_id:#010X}u32 {{ return Err(CdrError::InvalidEncoding); }}\n",
                ),
            );

            if let IdlType::Primitive(p) = &field.field_type {
                // Reuse primitive decoder without DHEADER/PL framing.
                Self::append_decode_primitive(&mut code, &ident, p);
            }
        }

        code.push_str("        Ok((Self {\n");
        for field in &s.fields {
            let fname = super::super::keywords::rust_ident(&field.name);
            if field.is_non_serialized() {
                push_fmt(
                    &mut code,
                    format_args!("            {fname}: Default::default(),\n"),
                );
            } else if field.is_external() {
                push_fmt(
                    &mut code,
                    format_args!("            {fname}: Box::new({fname}),\n"),
                );
            } else {
                push_fmt(&mut code, format_args!("            {fname},\n"));
            }
        }
        code.push_str("        }, offset))\n");
        code.push_str("    }\n");
        code.push_str("}\n\n");

        code
    }

    #[allow(clippy::too_many_lines)]
    fn emit_pl_cdr2_decode_impl(s: &Struct) -> String {
        let mut code = String::new();

        push_fmt(
            &mut code,
            format_args!("impl Cdr2Decode for {} {{\n", s.name),
        );
        code.push_str("    fn decode_cdr2_le(src: &[u8]) -> Result<(Self, usize), CdrError> {\n");
        code.push_str("        if src.len() < 4 { return Err(CdrError::UnexpectedEof); }\n");
        code.push_str("        let payload_len = {\n");
        code.push_str("            let mut tmp = [0u8; 4];\n");
        code.push_str("            tmp.copy_from_slice(&src[..4]);\n");
        code.push_str("            u32::from_le_bytes(tmp) as usize\n");
        code.push_str("        };\n");
        code.push_str("        let end = 4 + payload_len;\n");
        code.push_str("        if src.len() < end { return Err(CdrError::UnexpectedEof); }\n");
        code.push_str("        let mut offset: usize = 4;\n\n");

        for field in &s.fields {
            if field.is_non_serialized() {
                continue;
            }
            let ident = super::super::keywords::rust_ident(&field.name);
            let field_ty = Self::type_to_rust(&field.field_type);
            push_fmt(
                &mut code,
                format_args!(
                    "        let mut {fname}_value: Option<{field_ty}> = None;\n",
                    fname = ident,
                    field_ty = field_ty
                ),
            );
        }
        code.push('\n');

        code.push_str("        while offset < end {\n");
        code.push_str(
            "            if src.len() < offset + 8 { return Err(CdrError::UnexpectedEof); }\n",
        );
        code.push_str(
            "            let em = u32::from_le_bytes(src[offset..offset+4].try_into().unwrap());\n",
        );
        code.push_str("            offset += 4;\n");
        code.push_str("            let lc = (em >> 28) & 0x7;\n");
        code.push_str("            let must_understand = (em >> 31) & 1 == 1;\n");
        code.push_str("            let member_id = em & 0x0fff_ffff;\n");
        code.push_str("            let member_len = match lc { 0 => 1, 1 => 2, 2 => 4, 3 => 8, 5 => { let len = u32::from_le_bytes(src[offset..offset+4].try_into().unwrap()) as usize; offset += 4; len }, _ => end - offset };\n");
        code.push_str("            let member_end = offset.saturating_add(member_len).min(end);\n");
        code.push_str("            match member_id {\n");

        for (idx, field) in s.fields.iter().enumerate() {
            if field.is_non_serialized() {
                continue;
            }
            let ident = super::super::keywords::rust_ident(&field.name);
            let member_id = Self::compute_member_id(s, idx, field);
            let alignment = Self::cdr2_alignment(&field.field_type);

            push_fmt(
                &mut code,
                format_args!("                {member_id:#010X} => {{\n"),
            );
            push_fmt(
                &mut code,
                format_args!(
                    "                    let padding = ({alignment} - (offset % {alignment})) % {alignment};\n"
                ),
            );
            code.push_str("                    if src.len() < offset + padding { return Err(CdrError::UnexpectedEof); }\n");
            code.push_str("                    offset += padding;\n");

            // Decode and compute used length by re-encoding
            match &field.field_type {
                IdlType::Primitive(p) => {
                    let align = Self::cdr2_alignment(&field.field_type);
                    let size = Self::cdr2_fixed_size(&field.field_type).unwrap_or(0);
                    push_fmt(
                        &mut code,
                        format_args!("                    let mut local = offset;\n"),
                    );
                    push_fmt(
                        &mut code,
                        format_args!(
                            "                    let pad = ({align} - (local % {align})) % {align};\n"
                        ),
                    );
                    push_fmt(
                        &mut code,
                        format_args!(
                            "                    if src.len() < local + pad + {size} {{ return Err(CdrError::UnexpectedEof); }}\n"
                        ),
                    );
                    push_fmt(
                        &mut code,
                        format_args!("                    local += pad;\n"),
                    );
                    let read = match p {
                        PrimitiveType::Octet | PrimitiveType::UInt8 => "src[local]".to_string(),
                        PrimitiveType::Int8 => "i8::from_le_bytes([src[local]])".to_string(),
                        PrimitiveType::Boolean => "(src[local] != 0)".to_string(),
                        PrimitiveType::Char => "char::from(src[local])".to_string(),
                        PrimitiveType::Short | PrimitiveType::Int16 => {
                            "i16::from_le_bytes(src[local..local+2].try_into().unwrap())".to_string()
                        }
                        PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => {
                            "u16::from_le_bytes(src[local..local+2].try_into().unwrap())".to_string()
                        }
                        PrimitiveType::Long | PrimitiveType::Int32 => {
                            "i32::from_le_bytes(src[local..local+4].try_into().unwrap())".to_string()
                        }
                        PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => {
                            "u32::from_le_bytes(src[local..local+4].try_into().unwrap())".to_string()
                        }
                        PrimitiveType::Float => {
                            "f32::from_le_bytes(src[local..local+4].try_into().unwrap())".to_string()
                        }
                        PrimitiveType::WChar => {
                            "char::from_u32(u32::from_le_bytes(src[local..local+4].try_into().unwrap())).ok_or(CdrError::InvalidEncoding)?".to_string()
                        }
                        PrimitiveType::LongLong | PrimitiveType::Int64 => {
                            "i64::from_le_bytes(src[local..local+8].try_into().unwrap())".to_string()
                        }
                        PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => {
                            "u64::from_le_bytes(src[local..local+8].try_into().unwrap())".to_string()
                        }
                        PrimitiveType::Double | PrimitiveType::LongDouble => {
                            "f64::from_le_bytes(src[local..local+8].try_into().unwrap())".to_string()
                        }
                        // @audit-ok: string literal for generated code - unsupported primitives will fail at codegen time
                        _ => "unimplemented!()".to_string(),
                    };
                    push_fmt(
                        &mut code,
                        format_args!("                    let value_decoded = {read};\n"),
                    );
                    push_fmt(
                        &mut code,
                        format_args!("                    offset = member_end;\n"),
                    );
                }
                IdlType::Sequence { inner, .. } => {
                    if let IdlType::Named(name) = &**inner {
                        if super::helpers::is_named_mutable(name) {
                            // PL_CDR2 sequence of mutable structs with DHEADER per element
                            code.push_str("                    if src.len() < offset + 4 { return Err(CdrError::UnexpectedEof); }\n");
                            code.push_str("                    let mut len_buf = [0u8; 4];\n");
                            code.push_str("                    len_buf.copy_from_slice(&src[offset..offset+4]);\n");
                            code.push_str("                    let count = u32::from_le_bytes(len_buf) as usize;\n");
                            code.push_str("                    offset += 4;\n");
                            let elem_ty = Self::type_to_rust(inner);
                            push_fmt(
                                &mut code,
                                format_args!(
                                    "                    let mut items: Vec<{elem_ty}> = Vec::with_capacity(count);\n"
                                ),
                            );
                            code.push_str("                    for _ in 0..count {\n");
                            code.push_str("                        if src.len() < offset + 4 { return Err(CdrError::UnexpectedEof); }\n");
                            code.push_str("                        let mut hdr_buf = [0u8; 4];\n");
                            code.push_str("                        hdr_buf.copy_from_slice(&src[offset..offset+4]);\n");
                            code.push_str("                        let elem_len = u32::from_le_bytes(hdr_buf) as usize;\n");
                            code.push_str("                        offset += 4;\n");
                            push_fmt(
                                &mut code,
                                format_args!(
                                    "                        let (elem, used) = <{elem_ty}>::decode_cdr2_le(&src[offset..])?;\n"
                                ),
                            );
                            code.push_str("                        let advance = usize::min(elem_len, used);\n");
                            code.push_str("                        offset += advance;\n");
                            code.push_str("                        items.push(elem);\n");
                            code.push_str("                    }\n");
                            code.push_str("                    let value_decoded = items;\n");
                        } else {
                            let ty = Self::type_to_rust(&field.field_type);
                            push_fmt(
                                &mut code,
                                format_args!(
                                    "                    let (value_decoded, used) = <{ty}>::decode_cdr2_le(&src[offset..])?;\n",
                                ),
                            );
                            code.push_str("                    let advance = usize::min(member_end - offset, used);\n");
                            code.push_str("                    offset += advance;\n");
                        }
                    } else {
                        let ty = Self::type_to_rust(&field.field_type);
                        push_fmt(
                            &mut code,
                            format_args!(
                                "                    let (value_decoded, used) = <{ty}>::decode_cdr2_le(&src[offset..])?;\n",
                            ),
                        );
                        code.push_str("                    let advance = usize::min(member_end - offset, used);\n");
                        code.push_str("                    offset += advance;\n");
                    }
                }
                _ => {
                    let ty = Self::type_to_rust(&field.field_type);
                    push_fmt(
                        &mut code,
                        format_args!(
                            "                    let (value_decoded, used) = <{ty}>::decode_cdr2_le(&src[offset..])?;\n",
                        ),
                    );
                    code.push_str("                    let advance = usize::min(member_end - offset, used);\n");
                    code.push_str("                    offset += advance;\n");
                }
            }
            push_fmt(
                &mut code,
                format_args!(
                    "                    {fname}_value = Some(value_decoded);\n",
                    fname = ident
                ),
            );
            code.push_str("                }\n");
        }

        code.push_str("                _ => {\n");
        code.push_str("                    if must_understand {\n");
        code.push_str("                        return Err(CdrError::InvalidEncoding);\n");
        code.push_str("                    }\n");
        code.push_str("                    offset = member_end;\n");
        code.push_str("                }\n");
        code.push_str("            }\n");
        code.push_str("        }\n\n");

        code.push_str("        Ok((Self {\n");
        for field in &s.fields {
            let fname = super::super::keywords::rust_ident(&field.name);
            let ext = field.is_external();
            if field.is_non_serialized() {
                push_fmt(
                    &mut code,
                    format_args!("            {fname}: Default::default(),\n"),
                );
            } else if field.is_optional() && ext {
                push_fmt(
                    &mut code,
                    format_args!("            {fname}: {fname}_value.map(Box::new),\n"),
                );
            } else if field.is_optional() {
                push_fmt(
                    &mut code,
                    format_args!("            {fname}: {fname}_value,\n"),
                );
            } else if ext {
                push_fmt(
                    &mut code,
                    format_args!(
                        "            {fname}: Box::new({fname}_value.ok_or(CdrError::InvalidEncoding)?),\n",
                    ),
                );
            } else {
                push_fmt(
                    &mut code,
                    format_args!(
                        "            {fname}: {fname}_value.ok_or(CdrError::InvalidEncoding)?,\n",
                    ),
                );
            }
        }
        code.push_str("        }, offset))\n");
        code.push_str("    }\n");
        code.push_str("}\n\n");

        code
    }

    fn append_decode_field(dst: &mut String, field: &Field) {
        let ident = super::super::keywords::rust_ident(&field.name);
        match &field.field_type {
            IdlType::Primitive(p) => Self::append_decode_primitive(dst, &ident, p),
            IdlType::Sequence { inner, .. } => {
                // Bounded string (string<N>) is represented as Sequence<Char>
                if matches!(
                    **inner,
                    IdlType::Primitive(PrimitiveType::Char | PrimitiveType::WChar)
                ) {
                    Self::append_decode_primitive(dst, &ident, &PrimitiveType::String);
                } else {
                    Self::append_decode_sequence(dst, &ident, inner);
                }
            }
            IdlType::Array { inner, size } => {
                Self::append_decode_array(dst, &ident, inner, *size);
            }
            IdlType::Map { key, value, .. } => {
                Self::append_decode_map(dst, &ident, key, value);
            }
            IdlType::Named(name) => Self::append_decode_named(dst, &ident, name),
        }
    }

    fn append_decode_primitive(dst: &mut String, field_name: &str, primitive: &PrimitiveType) {
        match primitive {
            PrimitiveType::Octet | PrimitiveType::UInt8 => Self::decode_u8(dst, field_name),
            PrimitiveType::Int8 => Self::decode_i8(dst, field_name),
            PrimitiveType::Boolean => Self::decode_boolean(dst, field_name),
            PrimitiveType::Char => Self::decode_char(dst, field_name),
            PrimitiveType::WChar => Self::decode_wchar(dst, field_name),
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
            | PrimitiveType::LongDouble => Self::decode_numeric(dst, field_name, *primitive),
            PrimitiveType::Fixed { digits, scale } => {
                Self::decode_fixed(dst, field_name, *digits, *scale);
            }
            PrimitiveType::String | PrimitiveType::WString => Self::decode_string(dst, field_name),
            PrimitiveType::Void => Self::decode_void(dst, field_name),
        }
    }

    fn decode_u8(dst: &mut String, field_name: &str) {
        Self::decode_buffer_check(dst, "        ", "1");
        push_fmt(
            dst,
            format_args!("        let {field_name} = src[offset];\n"),
        );
        dst.push_str("        offset += 1;\n\n");
    }

    fn decode_i8(dst: &mut String, field_name: &str) {
        Self::decode_buffer_check(dst, "        ", "1");
        push_fmt(
            dst,
            format_args!("        let {field_name} = i8::from_le_bytes([src[offset]]);\n"),
        );
        dst.push_str("        offset += 1;\n\n");
    }

    fn decode_boolean(dst: &mut String, field_name: &str) {
        Self::decode_buffer_check(dst, "        ", "1");
        push_fmt(
            dst,
            format_args!("        let {field_name} = src[offset] != 0;\n"),
        );
        dst.push_str("        offset += 1;\n\n");
    }

    fn decode_char(dst: &mut String, field_name: &str) {
        Self::decode_buffer_check(dst, "        ", "1");
        push_fmt(
            dst,
            format_args!("        let {field_name} = char::from(src[offset]);\n"),
        );
        dst.push_str("        offset += 1;\n\n");
    }

    fn decode_wchar(dst: &mut String, field_name: &str) {
        Self::decode_buffer_check(dst, "        ", "4");
        dst.push_str(
            "        let scalar = {\n            let mut __hdds_tmp = [0u8; 4];\n            __hdds_tmp.copy_from_slice(&src[offset..offset+4]);\n            u32::from_le_bytes(__hdds_tmp)\n        };\n",
        );
        push_fmt(
            dst,
            format_args!(
                "        let {field_name} = char::from_u32(scalar).ok_or(CdrError::InvalidEncoding)?;\n"
            ),
        );
        dst.push_str("        offset += 4;\n\n");
    }

    fn decode_numeric(dst: &mut String, field_name: &str, primitive: PrimitiveType) {
        let Some(size) = Self::cdr2_fixed_size(&IdlType::Primitive(primitive)) else {
            push_fmt(
                dst,
                format_args!("        return Err(CdrError::InvalidEncoding);\n"),
            );
            dst.push('\n');
            return;
        };
        let size_expr = size.to_string();
        let rust_type = Self::type_to_rust(&IdlType::Primitive(primitive));
        Self::decode_buffer_check(dst, "        ", &size_expr);
        push_fmt(
            dst,
            format_args!(
                "        let {field_name} = {{\n            let mut __hdds_tmp = [0u8; {size}];\n            __hdds_tmp.copy_from_slice(&src[offset..offset+{size}]);\n            {rust_type}::from_le_bytes(__hdds_tmp)\n        }};\n"
            ),
        );
        push_fmt(dst, format_args!("        offset += {size};\n\n"));
    }

    fn decode_fixed(dst: &mut String, field_name: &str, digits: u32, scale: u32) {
        push_fmt(
            dst,
            format_args!("        // Decode fixed<{digits}, {scale}> field '{field_name}'\n"),
        );
        Self::decode_buffer_check(dst, "        ", "16");
        dst.push_str(
            "        let raw = {\n            let mut __hdds_tmp = [0u8; 16];\n            __hdds_tmp.copy_from_slice(&src[offset..offset+16]);\n            i128::from_le_bytes(__hdds_tmp)\n        };\n",
        );
        push_fmt(
            dst,
            format_args!("        let {field_name} = Fixed::<{digits}, {scale}>::from_raw(raw);\n"),
        );
        dst.push_str("        offset += 16;\n\n");
    }

    fn decode_string(dst: &mut String, field_name: &str) {
        push_fmt(
            dst,
            format_args!("        // Decode String field '{field_name}'\n"),
        );
        Self::decode_buffer_check(dst, "        ", "4");
        dst.push_str(
            "        let len = {\n            let mut __hdds_tmp = [0u8; 4];\n            __hdds_tmp.copy_from_slice(&src[offset..offset+4]);\n            u32::from_le_bytes(__hdds_tmp) as usize\n        };\n",
        );
        dst.push_str("        offset += 4;\n");
        Self::decode_buffer_check(dst, "        ", "len");
        dst.push_str("        // CDR: len includes null terminator\n");
        dst.push_str("        let str_len = len.saturating_sub(1);\n");
        dst.push_str("        let s = std::str::from_utf8(&src[offset..offset+str_len])\n");
        dst.push_str("            .map_err(|_| CdrError::InvalidEncoding)?;\n");
        push_fmt(
            dst,
            format_args!("        let {field_name} = s.to_string();\n"),
        );
        dst.push_str("        offset += len; // len includes null terminator\n\n");
    }

    fn decode_void(dst: &mut String, field_name: &str) {
        push_fmt(
            dst,
            format_args!("        // Decode void field '{field_name}' (no payload)\n"),
        );
        push_fmt(dst, format_args!("        let {field_name} = ();\n\n"));
    }

    fn append_decode_named(dst: &mut String, field_name: &str, type_name: &str) {
        push_fmt(
            dst,
            format_args!("        // Decode named field '{field_name}' of type '{type_name}'\n"),
        );
        push_fmt(
            dst,
            format_args!(
                "        let ({field_name}, __used) = <{type_name}>::decode_cdr2_le(&src[offset..])?;\n"
            ),
        );
        dst.push_str("        offset += __used;\n\n");
    }

    pub(super) fn decode_buffer_check(dst: &mut String, indent: &str, size_expr: &str) {
        push_fmt(
            dst,
            format_args!("{indent}if src.len() < offset + {size_expr} {{\n"),
        );
        push_fmt(
            dst,
            format_args!("{indent}    return Err(CdrError::UnexpectedEof);\n"),
        );
        push_fmt(dst, format_args!("{indent}}}\n"));
    }
}
