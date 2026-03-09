// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Union code generation for Rust.
//!
//! Generates Rust enum representations of IDL discriminated unions
//! with `Cdr2Encode`/`Cdr2Decode` implementations.

use super::helpers::to_pascal_case;
use super::{push_fmt, RustGenerator};
use crate::ast::{Union, UnionCase, UnionLabel};
use crate::types::{Annotation, IdlType, PrimitiveType};

impl RustGenerator {
    pub(super) fn generate_union(&self, u: &Union) -> String {
        let mut output = String::new();
        let indent = self.indent();
        let serde_derive = self.serde_derive();
        let serde_rename = self.serde_rename_attr();
        push_fmt(
            &mut output,
            format_args!("{indent}#[derive(Debug, Clone, PartialEq{serde_derive})]\n"),
        );
        if !serde_rename.is_empty() {
            push_fmt(&mut output, format_args!("{indent}{serde_rename}"));
        }
        let name = &u.name;
        push_fmt(&mut output, format_args!("{indent}pub enum {name} {{\n"));

        let mut default_case: Option<(String, String)> = None;

        for (idx, case) in u.cases.iter().enumerate() {
            let variant_name = Self::union_variant_name(&case.field.name, idx);
            let field_type = Self::type_to_rust(&case.field.field_type);
            let idl_type = case.field.field_type.to_idl_string();
            let needs_comment = matches!(
                case.field.field_type,
                IdlType::Array { .. } | IdlType::Sequence { .. } | IdlType::Map { .. }
            );

            if default_case.is_none()
                && (case
                    .labels
                    .iter()
                    .any(|label| matches!(label, UnionLabel::Default))
                    || case
                        .field
                        .annotations
                        .iter()
                        .any(|ann| matches!(ann, Annotation::Default | Annotation::DefaultLiteral)))
            {
                if let Some(expr) = Self::union_case_default_expr(&case.field.field_type) {
                    default_case = Some((variant_name.clone(), expr));
                }
            }

            let field_name = &case.field.name;
            if needs_comment {
                push_fmt(
                    &mut output,
                    format_args!(
                        "{indent}    {variant_name}({field_type}),  // was: {idl_type} {field_name}\n"
                    ),
                );
            } else {
                push_fmt(
                    &mut output,
                    format_args!("{indent}    {variant_name}({field_type}),\n"),
                );
            }
        }

        push_fmt(&mut output, format_args!("{indent}}}\n\n"));

        if let Some((variant, expr)) = default_case {
            push_fmt(
                &mut output,
                format_args!("{indent}impl Default for {name} {{\n"),
            );
            push_fmt(
                &mut output,
                format_args!("{indent}    fn default() -> Self {{\n"),
            );
            push_fmt(
                &mut output,
                format_args!("{indent}        Self::{variant}({expr})\n"),
            );
            push_fmt(&mut output, format_args!("{indent}    }}\n"));
            push_fmt(&mut output, format_args!("{indent}}}\n\n"));
        }

        // Generate Cdr2Encode impl
        output.push_str(&Self::emit_union_encode(u, &indent));

        // Generate Cdr2Decode impl
        output.push_str(&Self::emit_union_decode(u, &indent));

        output
    }

    fn union_variant_name(field_name: &str, idx: usize) -> String {
        if !field_name.starts_with(|c: char| c.is_alphabetic()) {
            return format!("Case{idx}");
        }
        to_pascal_case(field_name)
    }

    fn union_case_default_expr(idl_type: &IdlType) -> Option<String> {
        match idl_type {
            IdlType::Primitive(primitive) => match primitive {
                PrimitiveType::Void => Some("()".to_string()),
                PrimitiveType::Boolean => Some("false".to_string()),
                PrimitiveType::Char | PrimitiveType::WChar => Some("'\\0'".to_string()),
                PrimitiveType::String | PrimitiveType::WString => Some("String::new()".to_string()),
                PrimitiveType::Float | PrimitiveType::Double | PrimitiveType::LongDouble => {
                    Some("0.0".to_string())
                }
                PrimitiveType::Fixed { digits, scale } => {
                    Some(format!("Fixed::<{digits}, {scale}>::from_raw(0)"))
                }
                _ => Some("0".to_string()),
            },
            IdlType::Sequence { .. } => Some("Vec::new()".to_string()),
            IdlType::Map { .. } => Some("std::collections::HashMap::new()".to_string()),
            IdlType::Named(name) if name == "String" => Some("String::new()".to_string()),
            _ => None,
        }
    }

    /// Generate `Cdr2Encode` implementation for a union
    fn emit_union_encode(u: &Union, indent: &str) -> String {
        let mut code = String::new();
        let name = &u.name;

        push_fmt(
            &mut code,
            format_args!("{indent}impl Cdr2Encode for {name} {{\n"),
        );
        push_fmt(
            &mut code,
            format_args!(
                "{indent}    fn encode_cdr2_le(&self, dst: &mut [u8]) -> Result<usize, CdrError> {{\n"
            ),
        );
        push_fmt(
            &mut code,
            format_args!("{indent}        let mut offset: usize = 0;\n\n"),
        );

        // Get discriminator info
        let (disc_size, disc_align) = Self::discriminator_size_align(&u.discriminator);

        // Align for discriminator
        if disc_align > 1 {
            push_fmt(
                &mut code,
                format_args!("{indent}        // Align for discriminator\n"),
            );
            push_fmt(
                &mut code,
                format_args!(
                    "{indent}        let padding = ({disc_align} - (offset % {disc_align})) % {disc_align};\n"
                ),
            );
            push_fmt(
                &mut code,
                format_args!("{indent}        offset += padding;\n\n"),
            );
        }

        push_fmt(&mut code, format_args!("{indent}        match self {{\n"));

        for (idx, case) in u.cases.iter().enumerate() {
            let variant_name = Self::union_variant_name(&case.field.name, idx);
            let disc_value = Self::get_discriminator_expr(case, idx, &u.discriminator, u);

            push_fmt(
                &mut code,
                format_args!("{indent}            Self::{variant_name}(v) => {{\n"),
            );

            // Encode discriminator
            push_fmt(
                &mut code,
                format_args!("{indent}                // Encode discriminator\n"),
            );
            push_fmt(
                &mut code,
                format_args!(
                    "{indent}                if dst.len() < offset + {disc_size} {{ return Err(CdrError::BufferTooSmall); }}\n"
                ),
            );
            let disc_write = Self::discriminator_encode_expr(&u.discriminator, &disc_value);
            push_fmt(
                &mut code,
                format_args!("{indent}                {disc_write}\n"),
            );
            push_fmt(
                &mut code,
                format_args!("{indent}                offset += {disc_size};\n\n"),
            );

            // Encode value
            let value_encode = Self::emit_union_value_encode(&case.field.field_type, "v", indent);
            code.push_str(&value_encode);

            push_fmt(&mut code, format_args!("{indent}            }}\n"));
        }

        push_fmt(&mut code, format_args!("{indent}        }}\n"));
        push_fmt(&mut code, format_args!("{indent}        Ok(offset)\n"));
        push_fmt(&mut code, format_args!("{indent}    }}\n\n"));

        // max_cdr2_size
        push_fmt(
            &mut code,
            format_args!("{indent}    fn max_cdr2_size(&self) -> usize {{\n"),
        );
        push_fmt(
            &mut code,
            format_args!("{indent}        // Discriminator + max variant size (conservative)\n"),
        );
        let max_variant_size = u
            .cases
            .iter()
            .map(|c| Self::estimate_type_size(&c.field.field_type))
            .max()
            .unwrap_or(0);
        push_fmt(
            &mut code,
            format_args!(
                "{indent}        {} + {}\n",
                disc_size + disc_align,
                max_variant_size
            ),
        );
        push_fmt(&mut code, format_args!("{indent}    }}\n"));
        push_fmt(&mut code, format_args!("{indent}}}\n\n"));

        code
    }

    /// Generate `Cdr2Decode` implementation for a union
    fn emit_union_decode(u: &Union, indent: &str) -> String {
        let mut code = String::new();
        let name = &u.name;

        push_fmt(
            &mut code,
            format_args!("{indent}impl Cdr2Decode for {name} {{\n"),
        );
        push_fmt(
            &mut code,
            format_args!(
                "{indent}    fn decode_cdr2_le(src: &[u8]) -> Result<(Self, usize), CdrError> {{\n"
            ),
        );
        push_fmt(
            &mut code,
            format_args!("{indent}        let mut offset: usize = 0;\n\n"),
        );

        let (disc_size, disc_align) = Self::discriminator_size_align(&u.discriminator);

        // Align for discriminator
        if disc_align > 1 {
            push_fmt(
                &mut code,
                format_args!("{indent}        // Align for discriminator\n"),
            );
            push_fmt(
                &mut code,
                format_args!(
                    "{indent}        let padding = ({disc_align} - (offset % {disc_align})) % {disc_align};\n"
                ),
            );
            push_fmt(
                &mut code,
                format_args!("{indent}        offset += padding;\n\n"),
            );
        }

        // Read discriminator
        push_fmt(
            &mut code,
            format_args!("{indent}        // Decode discriminator\n"),
        );
        push_fmt(
            &mut code,
            format_args!(
                "{indent}        if src.len() < offset + {disc_size} {{ return Err(CdrError::UnexpectedEof); }}\n"
            ),
        );
        let disc_read = Self::discriminator_decode_expr(&u.discriminator);
        push_fmt(
            &mut code,
            format_args!("{indent}        let disc = {disc_read};\n"),
        );
        push_fmt(
            &mut code,
            format_args!("{indent}        offset += {disc_size};\n\n"),
        );

        push_fmt(&mut code, format_args!("{indent}        match disc {{\n"));

        let mut has_default = false;
        for (idx, case) in u.cases.iter().enumerate() {
            let variant_name = Self::union_variant_name(&case.field.name, idx);
            let is_default = case.labels.iter().any(|l| matches!(l, UnionLabel::Default));

            if is_default {
                has_default = true;
                push_fmt(&mut code, format_args!("{indent}            _ => {{\n"));
            } else {
                let (disc_value, is_guard) =
                    Self::get_discriminator_pattern(case, idx, &u.discriminator);
                if is_guard {
                    push_fmt(
                        &mut code,
                        format_args!("{indent}            d if d == {disc_value} as i32 => {{\n"),
                    );
                } else {
                    push_fmt(
                        &mut code,
                        format_args!("{indent}            {disc_value} => {{\n"),
                    );
                }
            }

            // Decode value
            let value_decode = Self::emit_union_value_decode(&case.field.field_type, indent);
            code.push_str(&value_decode);
            push_fmt(
                &mut code,
                format_args!("{indent}                Ok((Self::{variant_name}(val), offset))\n"),
            );
            push_fmt(&mut code, format_args!("{indent}            }}\n"));
        }

        if !has_default {
            push_fmt(
                &mut code,
                format_args!("{indent}            _ => Err(CdrError::InvalidEncoding),\n"),
            );
        }

        push_fmt(&mut code, format_args!("{indent}        }}\n"));
        push_fmt(&mut code, format_args!("{indent}    }}\n"));
        push_fmt(&mut code, format_args!("{indent}}}\n\n"));

        code
    }

    /// Get discriminator size and alignment
    #[allow(clippy::missing_const_for_fn)]
    fn discriminator_size_align(disc_type: &IdlType) -> (usize, usize) {
        #[allow(clippy::match_same_arms)]
        match disc_type {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Boolean
                | PrimitiveType::Octet
                | PrimitiveType::Int8
                | PrimitiveType::UInt8 => (1, 1),
                PrimitiveType::Short
                | PrimitiveType::UnsignedShort
                | PrimitiveType::Int16
                | PrimitiveType::UInt16 => (2, 2),
                PrimitiveType::Long
                | PrimitiveType::UnsignedLong
                | PrimitiveType::Int32
                | PrimitiveType::UInt32 => (4, 4),
                PrimitiveType::LongLong
                | PrimitiveType::UnsignedLongLong
                | PrimitiveType::Int64
                | PrimitiveType::UInt64 => (8, 8),
                _ => (4, 4), // Default to 32-bit
            },
            IdlType::Named(_)
            | IdlType::Sequence { .. }
            | IdlType::Array { .. }
            | IdlType::Map { .. } => (4, 4), // Enum and others, default to 32-bit
        }
    }

    /// Get discriminator value for a case
    /// Returns (pattern, `is_guard`) - if `is_guard` is true, the pattern is a match guard.
    fn get_discriminator_pattern(
        case: &UnionCase,
        idx: usize,
        disc_type: &IdlType,
    ) -> (String, bool) {
        for label in &case.labels {
            if let UnionLabel::Value(v) = label {
                // If it's a numeric literal, use as direct match pattern
                if v.parse::<i64>().is_ok() {
                    return (v.clone(), false);
                }
                // It's an enum variant name - needs a match guard since `EnumType::Variant as i32`
                // is not a valid pattern
                if let IdlType::Named(type_name) = disc_type {
                    let qualified = format!("{type_name}::{v}");
                    return (qualified, true);
                }
                // Fallback: use index
                return (idx.to_string(), false);
            }
        }
        // If no explicit value, use index
        (idx.to_string(), false)
    }

    /// Get discriminator value as a numeric expression (for encode).
    fn get_discriminator_expr(
        case: &UnionCase,
        _idx: usize,
        disc_type: &IdlType,
        union: &Union,
    ) -> String {
        if let Some(label) = case.labels.first() {
            match label {
                UnionLabel::Value(v) => {
                    if v.parse::<i64>().is_ok() {
                        return v.clone();
                    }
                    if let IdlType::Named(type_name) = disc_type {
                        return format!("{type_name}::{v} as u32");
                    }
                    return v.clone();
                }
                UnionLabel::Default => {
                    return Self::disc_default_value(disc_type, union);
                }
            }
        }
        Self::disc_default_value(disc_type, union)
    }

    /// Find a discriminant value for `default:` that does not collide with explicit cases.
    fn disc_default_value(disc_type: &IdlType, union: &Union) -> String {
        // Collect all explicit label values (numeric or enum ordinals).
        // For enum discriminants, labels are variant names (e.g. "X", "Y") that
        // can't be parsed as integers. We assign them sequential ordinals (0, 1, ...)
        // matching IDL default enum numbering.
        let mut used: Vec<i64> = Vec::new();
        let is_enum_disc = matches!(disc_type, IdlType::Named(_));
        let mut next_enum_ordinal: i64 = 0;
        for case in &union.cases {
            for label in &case.labels {
                if let UnionLabel::Value(v) = label {
                    if let Ok(n) = v.parse::<i64>() {
                        used.push(n);
                    } else if is_enum_disc {
                        used.push(next_enum_ordinal);
                        next_enum_ordinal += 1;
                    }
                }
            }
        }

        if matches!(disc_type, IdlType::Primitive(PrimitiveType::Boolean)) {
            // For bool: pick the unused one
            let true_used = used.contains(&1);
            let false_used = used.contains(&0);
            return if !false_used {
                "false".to_string()
            } else if !true_used {
                "true".to_string()
            } else {
                // Both used -- fallback (should not happen: both cases + default is illogical)
                "false".to_string()
            };
        }

        // For integers: find first non-negative value not in use
        let mut candidate: i64 = 0;
        loop {
            if !used.contains(&candidate) {
                return candidate.to_string();
            }
            candidate += 1;
            if candidate > 0x7FFF_FFFF {
                break;
            }
        }
        // Fallback (extremely unlikely)
        "0".to_string()
    }

    /// Generate discriminator encode expression
    fn discriminator_encode_expr(disc_type: &IdlType, value: &str) -> String {
        let (size, _) = Self::discriminator_size_align(disc_type);
        match size {
            1 => format!("dst[offset] = {value} as u8;"),
            2 => format!("dst[offset..offset+2].copy_from_slice(&({value} as i16).to_le_bytes());"),
            8 => format!("dst[offset..offset+8].copy_from_slice(&({value} as i64).to_le_bytes());"),
            _ => format!("dst[offset..offset+4].copy_from_slice(&({value} as i32).to_le_bytes());"),
        }
    }

    /// Generate discriminator decode expression
    fn discriminator_decode_expr(disc_type: &IdlType) -> String {
        let (size, _) = Self::discriminator_size_align(disc_type);
        match size {
            1 => "src[offset] as i32".to_string(),
            2 => "i16::from_le_bytes([src[offset], src[offset+1]]) as i32".to_string(),
            8 => "i64::from_le_bytes(src[offset..offset+8].try_into().unwrap())".to_string(),
            _ => "i32::from_le_bytes(src[offset..offset+4].try_into().unwrap())".to_string(),
        }
    }

    /// Emit encode code for a union value
    fn emit_union_value_encode(ty: &IdlType, var: &str, indent: &str) -> String {
        let mut code = String::new();
        let alignment = Self::cdr2_alignment(ty);

        if alignment > 1 {
            push_fmt(
                &mut code,
                format_args!("{indent}                // Align for value\n"),
            );
            push_fmt(
                &mut code,
                format_args!(
                    "{indent}                let padding = ({alignment} - (offset % {alignment})) % {alignment};\n"
                ),
            );
            push_fmt(
                &mut code,
                format_args!("{indent}                offset += padding;\n\n"),
            );
        }

        match ty {
            IdlType::Primitive(p) => {
                let encode = Self::primitive_encode_inline(p, var);
                code.push_str(&encode.replace('\n', &format!("\n{indent}                ")));
                code.push('\n');
            }
            IdlType::Named(_) => {
                // Nested type - call its encode
                push_fmt(
                    &mut code,
                    format_args!(
                        "{indent}                let n = {var}.encode_cdr2_le(&mut dst[offset..])?;\n"
                    ),
                );
                push_fmt(
                    &mut code,
                    format_args!("{indent}                offset += n;\n"),
                );
            }
            IdlType::Sequence { inner, .. } => {
                push_fmt(
                    &mut code,
                    format_args!("{indent}                // Encode sequence length\n"),
                );
                push_fmt(
                    &mut code,
                    format_args!("{indent}                let len = {var}.len() as u32;\n"),
                );
                push_fmt(
                    &mut code,
                    format_args!(
                        "{indent}                dst[offset..offset+4].copy_from_slice(&len.to_le_bytes());\n"
                    ),
                );
                push_fmt(
                    &mut code,
                    format_args!("{indent}                offset += 4;\n"),
                );
                push_fmt(
                    &mut code,
                    format_args!("{indent}                for item in {var}.iter() {{\n"),
                );
                let inner_encode =
                    Self::emit_union_value_encode(inner, "item", &format!("{indent}    "));
                code.push_str(&inner_encode);
                push_fmt(&mut code, format_args!("{indent}                }}\n"));
            }
            IdlType::Array { inner, size } => {
                push_fmt(
                    &mut code,
                    format_args!("{indent}                for i in 0..{size} {{\n"),
                );
                let inner_encode = Self::emit_union_value_encode(
                    inner,
                    &format!("&{var}[i]"),
                    &format!("{indent}    "),
                );
                code.push_str(&inner_encode);
                push_fmt(&mut code, format_args!("{indent}                }}\n"));
            }
            IdlType::Map { .. } => {
                push_fmt(
                    &mut code,
                    format_args!(
                        "{indent}                // complex type encoding: not yet supported\n"
                    ),
                );
            }
        }

        code
    }

    /// Emit decode code for a union value
    fn emit_union_value_decode(ty: &IdlType, indent: &str) -> String {
        let mut code = String::new();
        let alignment = Self::cdr2_alignment(ty);

        if alignment > 1 {
            push_fmt(
                &mut code,
                format_args!("{indent}                // Align for value\n"),
            );
            push_fmt(
                &mut code,
                format_args!(
                    "{indent}                let padding = ({alignment} - (offset % {alignment})) % {alignment};\n"
                ),
            );
            push_fmt(
                &mut code,
                format_args!("{indent}                offset += padding;\n\n"),
            );
        }

        match ty {
            IdlType::Primitive(p) => {
                let decode = Self::primitive_decode_inline(p);
                push_fmt(
                    &mut code,
                    format_args!("{indent}                let val = {decode};\n"),
                );
            }
            IdlType::Named(name) => {
                push_fmt(
                    &mut code,
                    format_args!(
                        "{indent}                let (val, n) = {name}::decode_cdr2_le(&src[offset..])?;\n"
                    ),
                );
                push_fmt(
                    &mut code,
                    format_args!("{indent}                offset += n;\n"),
                );
            }
            IdlType::Sequence { inner, .. } => {
                push_fmt(
                    &mut code,
                    format_args!("{indent}                // Decode sequence length\n"),
                );
                push_fmt(
                    &mut code,
                    format_args!(
                        "{indent}                let len = u32::from_le_bytes(src[offset..offset+4].try_into().unwrap()) as usize;\n"
                    ),
                );
                push_fmt(
                    &mut code,
                    format_args!("{indent}                offset += 4;\n"),
                );
                push_fmt(
                    &mut code,
                    format_args!(
                        "{indent}                let mut val = Vec::with_capacity(len);\n"
                    ),
                );
                push_fmt(
                    &mut code,
                    format_args!("{indent}                for _ in 0..len {{\n"),
                );
                let inner_decode = Self::emit_union_value_decode(inner, &format!("{indent}    "));
                // Extract just the val assignment from inner decode
                code.push_str(&inner_decode.replace("let val", "let item"));
                push_fmt(
                    &mut code,
                    format_args!("{indent}                    val.push(item);\n"),
                );
                push_fmt(&mut code, format_args!("{indent}                }}\n"));
            }
            _ => {
                push_fmt(
                    &mut code,
                    format_args!(
                        "{indent}                let val = Default::default(); // complex type decoding: not yet supported\n"
                    ),
                );
            }
        }

        code
    }

    /// Primitive encode inline (for union values)
    #[allow(clippy::match_same_arms)]
    fn primitive_encode_inline(p: &PrimitiveType, var: &str) -> String {
        match p {
            PrimitiveType::Boolean => format!(
                "if dst.len() < offset + 1 {{ return Err(CdrError::BufferTooSmall); }}\ndst[offset] = if *{var} {{ 1 }} else {{ 0 }};\noffset += 1;"
            ),
            PrimitiveType::Octet | PrimitiveType::UInt8 => format!(
                "if dst.len() < offset + 1 {{ return Err(CdrError::BufferTooSmall); }}\ndst[offset] = *{var};\noffset += 1;"
            ),
            PrimitiveType::Int8 | PrimitiveType::Char | PrimitiveType::WChar => format!(
                "if dst.len() < offset + 1 {{ return Err(CdrError::BufferTooSmall); }}\ndst[offset] = *{var} as u8;\noffset += 1;"
            ),
            PrimitiveType::Short
            | PrimitiveType::Int16
            | PrimitiveType::UnsignedShort
            | PrimitiveType::UInt16 => format!(
                "if dst.len() < offset + 2 {{ return Err(CdrError::BufferTooSmall); }}\ndst[offset..offset+2].copy_from_slice(&{var}.to_le_bytes());\noffset += 2;"
            ),
            PrimitiveType::Long
            | PrimitiveType::Int32
            | PrimitiveType::UnsignedLong
            | PrimitiveType::UInt32
            | PrimitiveType::Float => format!(
                "if dst.len() < offset + 4 {{ return Err(CdrError::BufferTooSmall); }}\ndst[offset..offset+4].copy_from_slice(&{var}.to_le_bytes());\noffset += 4;"
            ),
            PrimitiveType::LongLong
            | PrimitiveType::Int64
            | PrimitiveType::UnsignedLongLong
            | PrimitiveType::UInt64
            | PrimitiveType::Double
            | PrimitiveType::LongDouble => format!(
                "if dst.len() < offset + 8 {{ return Err(CdrError::BufferTooSmall); }}\ndst[offset..offset+8].copy_from_slice(&{var}.to_le_bytes());\noffset += 8;"
            ),
            PrimitiveType::String | PrimitiveType::WString => format!(
                "let bytes = {var}.as_bytes();\nlet len = bytes.len();\nlet len_u32 = u32::try_from(len).map_err(|_| CdrError::InvalidEncoding)?;\nif dst.len() < offset + 4 + len + 1 {{ return Err(CdrError::BufferTooSmall); }}\ndst[offset..offset+4].copy_from_slice(&len_u32.to_le_bytes());\noffset += 4;\ndst[offset..offset+len].copy_from_slice(bytes);\noffset += len;\ndst[offset] = 0;\noffset += 1;"
            ),
            _ => format!("// unsupported type: encode {var}"),
        }
    }

    /// Primitive decode inline (for union values)
    fn primitive_decode_inline(p: &PrimitiveType) -> String {
        match p {
            PrimitiveType::Boolean => {
                "{ if src.len() < offset + 1 { return Err(CdrError::UnexpectedEof); } let v = src[offset] != 0; offset += 1; v }".to_string()
            }
            PrimitiveType::Octet | PrimitiveType::UInt8 => {
                "{ if src.len() < offset + 1 { return Err(CdrError::UnexpectedEof); } let v = src[offset]; offset += 1; v }".to_string()
            }
            PrimitiveType::Int8 => {
                "{ if src.len() < offset + 1 { return Err(CdrError::UnexpectedEof); } let v = src[offset] as i8; offset += 1; v }".to_string()
            }
            PrimitiveType::Short | PrimitiveType::Int16 => {
                "{ if src.len() < offset + 2 { return Err(CdrError::UnexpectedEof); } let v = i16::from_le_bytes(src[offset..offset+2].try_into().unwrap()); offset += 2; v }".to_string()
            }
            PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => {
                "{ if src.len() < offset + 2 { return Err(CdrError::UnexpectedEof); } let v = u16::from_le_bytes(src[offset..offset+2].try_into().unwrap()); offset += 2; v }".to_string()
            }
            PrimitiveType::Long | PrimitiveType::Int32 => {
                "{ if src.len() < offset + 4 { return Err(CdrError::UnexpectedEof); } let v = i32::from_le_bytes(src[offset..offset+4].try_into().unwrap()); offset += 4; v }".to_string()
            }
            PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => {
                "{ if src.len() < offset + 4 { return Err(CdrError::UnexpectedEof); } let v = u32::from_le_bytes(src[offset..offset+4].try_into().unwrap()); offset += 4; v }".to_string()
            }
            PrimitiveType::LongLong | PrimitiveType::Int64 => {
                "{ if src.len() < offset + 8 { return Err(CdrError::UnexpectedEof); } let v = i64::from_le_bytes(src[offset..offset+8].try_into().unwrap()); offset += 8; v }".to_string()
            }
            PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => {
                "{ if src.len() < offset + 8 { return Err(CdrError::UnexpectedEof); } let v = u64::from_le_bytes(src[offset..offset+8].try_into().unwrap()); offset += 8; v }".to_string()
            }
            PrimitiveType::Float => {
                "{ if src.len() < offset + 4 { return Err(CdrError::UnexpectedEof); } let v = f32::from_le_bytes(src[offset..offset+4].try_into().unwrap()); offset += 4; v }".to_string()
            }
            PrimitiveType::Double | PrimitiveType::LongDouble => {
                "{ if src.len() < offset + 8 { return Err(CdrError::UnexpectedEof); } let v = f64::from_le_bytes(src[offset..offset+8].try_into().unwrap()); offset += 8; v }".to_string()
            }
            PrimitiveType::String | PrimitiveType::WString => {
                "{ if src.len() < offset + 4 { return Err(CdrError::UnexpectedEof); } let len = u32::from_le_bytes(src[offset..offset+4].try_into().unwrap()) as usize; offset += 4; if src.len() < offset + len + 1 { return Err(CdrError::UnexpectedEof); } let s = String::from_utf8_lossy(&src[offset..offset+len]).to_string(); offset += len + 1; s }".to_string()
            }
            PrimitiveType::Char | PrimitiveType::WChar => {
                "{ if src.len() < offset + 1 { return Err(CdrError::UnexpectedEof); } let v = src[offset] as char; offset += 1; v }".to_string()
            }
            _ => "Default::default()".to_string(),
        }
    }

    /// Estimate type size for `max_cdr2_size`
    fn estimate_type_size(ty: &IdlType) -> usize {
        #[allow(clippy::match_same_arms)]
        match ty {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Boolean
                | PrimitiveType::Octet
                | PrimitiveType::Int8
                | PrimitiveType::UInt8
                | PrimitiveType::Char => 1,
                PrimitiveType::Short
                | PrimitiveType::UnsignedShort
                | PrimitiveType::Int16
                | PrimitiveType::UInt16 => 2,
                PrimitiveType::Long
                | PrimitiveType::UnsignedLong
                | PrimitiveType::Int32
                | PrimitiveType::UInt32
                | PrimitiveType::Float => 4,
                PrimitiveType::LongLong
                | PrimitiveType::UnsignedLongLong
                | PrimitiveType::Int64
                | PrimitiveType::UInt64
                | PrimitiveType::Double
                | PrimitiveType::LongDouble => 8,
                PrimitiveType::String | PrimitiveType::WString => 256 + 4 + 1, // Conservative
                _ => 16,
            },
            IdlType::Named(_) => 64,          // Conservative for named types
            IdlType::Sequence { .. } => 1024, // Conservative
            IdlType::Array { inner, size } => Self::estimate_type_size(inner) * (*size as usize),
            IdlType::Map { .. } => 64,
        }
    }
}
