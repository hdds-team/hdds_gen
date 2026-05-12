// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Decode helpers for container types (sequences, arrays, maps).
//!
//! Generates CDR2 deserialization code for complex IDL containers.

#![allow(clippy::uninlined_format_args)]

use super::{push_fmt, CdrVersion, RustGenerator};
use crate::types::{IdlType, PrimitiveType};

impl RustGenerator {
    #[allow(clippy::too_many_lines, clippy::branches_sharing_code)]
    pub(super) fn append_decode_sequence(
        dst: &mut String,
        field_name: &str,
        inner: &IdlType,
        version: CdrVersion,
    ) {
        let suffix = super::helpers::xcdr_method_suffix(version);
        push_fmt(
            dst,
            format_args!("        // Decode sequence field '{field_name}'\n"),
        );
        Self::decode_buffer_check(dst, "        ", "4");
        dst.push_str(
            "        let count = {\n            let mut __hdds_tmp = [0u8; 4];\n            __hdds_tmp.copy_from_slice(&src[offset..offset+4]);\n            u32::from_le_bytes(__hdds_tmp) as usize\n        };\n",
        );
        dst.push_str("        offset += 4;\n");

        let rust_type = Self::type_to_rust(inner);
        push_fmt(
            dst,
            format_args!("        let mut {field_name} = Vec::with_capacity(count);\n"),
        );

        if let IdlType::Primitive(PrimitiveType::Fixed { digits, scale }) = inner {
            push_fmt(dst, format_args!("        for _ in 0..count {{\n"));
            Self::decode_buffer_check(dst, "            ", "16");
            push_fmt(
                dst,
                format_args!(
                    "            let raw = {{\n                let mut __hdds_tmp = [0u8; 16];\n                __hdds_tmp.copy_from_slice(&src[offset..offset+16]);\n                i128::from_le_bytes(__hdds_tmp)\n            }};\n"
                ),
            );
            push_fmt(
                dst,
                format_args!(
                    "            {field_name}.push(Fixed::<{digits}, {scale}>::from_raw(raw));\n"
                ),
            );
            dst.push_str("            offset += 16;\n");
        } else if matches!(inner, IdlType::Primitive(PrimitiveType::WChar)) {
            push_fmt(dst, format_args!("        for _ in 0..count {{\n"));
            Self::decode_buffer_check(dst, "            ", "4");
            dst.push_str(
                "            let scalar = {\n                let mut __hdds_tmp = [0u8; 4];\n                __hdds_tmp.copy_from_slice(&src[offset..offset+4]);\n                u32::from_le_bytes(__hdds_tmp)\n            };\n",
            );
            dst.push_str(
                "            let value = char::from_u32(scalar).ok_or(CdrError::InvalidEncoding)?;\n",
            );
            push_fmt(dst, format_args!("            {field_name}.push(value);\n"));
            dst.push_str("            offset += 4;\n");
        } else if let Some(elem_size) = Self::cdr2_fixed_size(inner) {
            push_fmt(dst, format_args!("        for _ in 0..count {{\n"));
            let combined = elem_size.to_string();
            Self::decode_buffer_check(dst, "            ", &combined);
            push_fmt(
                dst,
                format_args!(
                    "            let elem = {{\n                let mut __hdds_tmp = [0u8; {elem_size}];\n                __hdds_tmp.copy_from_slice(&src[offset..offset+{elem_size}]);\n                {rust_type}::from_le_bytes(__hdds_tmp)\n            }};\n"
                ),
            );
            push_fmt(dst, format_args!("            {field_name}.push(elem);\n"));
            push_fmt(dst, format_args!("            offset += {elem_size};\n"));
        } else {
            dst.push_str("        for _ in 0..count {\n");
            if matches!(
                inner,
                IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString)
            ) || super::helpers::is_bounded_string(inner)
            {
                // Handle unbounded strings AND bounded strings (string<N> -> Sequence<Char, N>)
                Self::decode_string_into(dst, "            ", "value");
                push_fmt(dst, format_args!("            {field_name}.push(value);\n"));
            } else if matches!(inner, IdlType::Primitive(PrimitiveType::WChar)) {
                Self::decode_buffer_check(dst, "            ", "4");
                dst.push_str(
                    "            let scalar = {\n                let mut __hdds_tmp = [0u8; 4];\n                __hdds_tmp.copy_from_slice(&src[offset..offset+4]);\n                u32::from_le_bytes(__hdds_tmp)\n            };\n",
                );
                dst.push_str(
                    "            let value = char::from_u32(scalar).ok_or(CdrError::InvalidEncoding)?;\n",
                );
                push_fmt(dst, format_args!("            {field_name}.push(value);\n"));
                dst.push_str("            offset += 4;\n");
            } else if let IdlType::Named(name) = inner {
                if super::helpers::is_named_mutable(name) {
                    dst.push_str("            let elem_len = u32::from_le_bytes(src[offset..offset+4].try_into().unwrap()) as usize;\n");
                    dst.push_str("            offset += 4;\n");
                    let rust_type = Self::type_to_rust(inner);
                    // PL_CDR2 mutable element: clamp `used` to `elem_len`
                    // boundary per EMHEADER semantics. Offset-aware decode via
                    // `_at` + pre/post delta to compute `used`.
                    dst.push_str("            let __hdds_pre = offset;\n");
                    push_fmt(
                        dst,
                        format_args!(
                            "            let elem = <{rust_type}>::decode_{suffix}_le_at(src, &mut offset)?;\n"
                        ),
                    );
                    dst.push_str("            let used = offset - __hdds_pre;\n");
                    dst.push_str("            let advance = usize::min(elem_len, used);\n");
                    dst.push_str("            offset = __hdds_pre + advance;\n");
                    push_fmt(dst, format_args!("            {field_name}.push(elem);\n"));
                } else {
                    let rust_type = Self::type_to_rust(inner);
                    push_fmt(
                        dst,
                        format_args!(
                            "            let elem = <{rust_type}>::decode_{suffix}_le_at(src, &mut offset)?;\n"
                        ),
                    );
                    push_fmt(dst, format_args!("            {field_name}.push(elem);\n"));
                }
            } else {
                // Non-named types (sequences, arrays, maps) - decode inline via `_at`.
                let rust_type = Self::type_to_rust(inner);
                push_fmt(
                    dst,
                    format_args!(
                        "            let elem = <{rust_type}>::decode_{suffix}_le_at(src, &mut offset)?;\n"
                    ),
                );
                push_fmt(dst, format_args!("            {field_name}.push(elem);\n"));
            }
        }
        dst.push_str("        }\n\n");
    }

    pub(super) fn append_decode_array(
        dst: &mut String,
        field_name: &str,
        inner: &IdlType,
        size: u32,
        version: CdrVersion,
    ) {
        push_fmt(
            dst,
            format_args!("        // Decode array field '{field_name}' [{size}]\n"),
        );

        if let Some(elem_size) = Self::is_primitive_like(inner) {
            Self::append_decode_array_primitive(dst, field_name, inner, size, elem_size);
        } else {
            Self::append_decode_array_dynamic(dst, field_name, inner, size, version);
        }
    }

    fn append_decode_array_primitive(
        dst: &mut String,
        field_name: &str,
        inner: &IdlType,
        size: u32,
        elem_size: usize,
    ) {
        let total_size = elem_size * (size as usize);
        let total_expr = total_size.to_string();
        Self::decode_buffer_check(dst, "        ", &total_expr);

        let rust_type = Self::type_to_rust(inner);
        push_fmt(
            dst,
            format_args!("        let mut {field_name} = Vec::with_capacity({size});\n"),
        );
        push_fmt(dst, format_args!("        for _ in 0..{size} {{\n"));
        push_fmt(
            dst,
            format_args!(
                "            let elem = {{\n                let mut __hdds_tmp = [0u8; {elem_size}];\n                __hdds_tmp.copy_from_slice(&src[offset..offset+{elem_size}]);\n                {rust_type}::from_le_bytes(__hdds_tmp)\n            }};\n"
            ),
        );
        push_fmt(dst, format_args!("            {field_name}.push(elem);\n"));
        push_fmt(dst, format_args!("            offset += {elem_size};\n"));
        dst.push_str("        }\n");
        push_fmt(
            dst,
            format_args!(
                "        let {field_name}: [{rust_type}; {size}] = match {field_name}.try_into() {{ Ok(arr) => arr, Err(_) => return Err(CdrError::InvalidEncoding) }};\n\n"
            ),
        );
    }

    fn append_decode_array_dynamic(
        dst: &mut String,
        field_name: &str,
        inner: &IdlType,
        size: u32,
        version: CdrVersion,
    ) {
        let suffix = super::helpers::xcdr_method_suffix(version);
        push_fmt(
            dst,
            format_args!("        let mut {field_name} = Vec::with_capacity({size});\n"),
        );
        push_fmt(dst, format_args!("        for _ in 0..{size} {{\n"));
        if let IdlType::Primitive(PrimitiveType::Fixed { digits, scale }) = inner {
            Self::decode_buffer_check(dst, "            ", "16");
            dst.push_str(
                "            let raw = {\n                let mut __hdds_tmp = [0u8; 16];\n                __hdds_tmp.copy_from_slice(&src[offset..offset+16]);\n                i128::from_le_bytes(__hdds_tmp)\n            };\n",
            );
            push_fmt(
                dst,
                format_args!(
                    "            {field_name}.push(Fixed::<{digits}, {scale}>::from_raw(raw));\n"
                ),
            );
            dst.push_str("            offset += 16;\n");
        } else if matches!(inner, IdlType::Primitive(PrimitiveType::WChar)) {
            Self::decode_buffer_check(dst, "            ", "4");
            dst.push_str(
                "            let scalar = {\n                let mut __hdds_tmp = [0u8; 4];\n                __hdds_tmp.copy_from_slice(&src[offset..offset+4]);\n                u32::from_le_bytes(__hdds_tmp)\n            };\n",
            );
            dst.push_str(
                "            let value = char::from_u32(scalar).ok_or(CdrError::InvalidEncoding)?;\n",
            );
            push_fmt(dst, format_args!("            {field_name}.push(value);\n"));
            dst.push_str("            offset += 4;\n");
        } else if matches!(
            inner,
            IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString)
        ) || super::helpers::is_bounded_string(inner)
        {
            // Handle unbounded strings AND bounded strings (string<N> -> Sequence<Char, N>)
            Self::decode_string_into(dst, "            ", "value");
            push_fmt(dst, format_args!("            {field_name}.push(value);\n"));
        } else {
            let rust_type = Self::type_to_rust(inner);
            push_fmt(
                dst,
                format_args!(
                    "            let elem = <{rust_type}>::decode_{suffix}_le_at(src, &mut offset)?;\n"
                ),
            );
            push_fmt(dst, format_args!("            {field_name}.push(elem);\n"));
        }
        dst.push_str("        }\n");
        push_fmt(
            dst,
            format_args!(
                "        let {field_name} = {field_name}.try_into().expect(\"array length verified\");\n\n"
            ),
        );
    }

    pub(super) fn append_decode_map(
        dst: &mut String,
        field_name: &str,
        key: &IdlType,
        value: &IdlType,
        version: CdrVersion,
    ) {
        Self::append_decode_map_prelude(dst, field_name);

        let key_size = Self::is_primitive_like(key);
        let value_size = Self::is_primitive_like(value);

        let key_type = Self::type_to_rust(key);
        let value_type = Self::type_to_rust(value);

        if let (Some(k), Some(v)) = (key_size, value_size) {
            Self::append_decode_map_primitives(dst, field_name, &key_type, k, &value_type, v);
        } else {
            Self::append_decode_map_generic(
                dst, field_name, key, key_size, value, value_size, version,
            );
        }
    }

    fn append_decode_map_prelude(dst: &mut String, field_name: &str) {
        push_fmt(
            dst,
            format_args!("        // Decode map field '{field_name}'\n"),
        );
        Self::decode_buffer_check(dst, "        ", "4");
        dst.push_str(
            "        let count = {\n            let mut __hdds_tmp = [0u8; 4];\n            __hdds_tmp.copy_from_slice(&src[offset..offset+4]);\n            u32::from_le_bytes(__hdds_tmp) as usize\n        };\n",
        );
        dst.push_str("        offset += 4;\n");
        push_fmt(
            dst,
            format_args!(
                "        let mut {field_name} = std::collections::HashMap::with_capacity(count);\n"
            ),
        );
    }

    fn append_decode_map_primitives(
        dst: &mut String,
        field_name: &str,
        key_type: &str,
        key_size: usize,
        value_type: &str,
        value_size: usize,
    ) {
        dst.push_str("        for _ in 0..count {\n");
        let combined = (key_size + value_size).to_string();
        Self::decode_buffer_check(dst, "            ", &combined);
        push_fmt(
            dst,
            format_args!(
                "            let k = {{\n                let mut __hdds_tmp = [0u8; {key_size}];\n                __hdds_tmp.copy_from_slice(&src[offset..offset+{key_size}]);\n                {key_type}::from_le_bytes(__hdds_tmp)\n            }};\n"
            ),
        );
        push_fmt(dst, format_args!("            offset += {key_size};\n"));
        push_fmt(
            dst,
            format_args!(
                "            let v = {{\n                let mut __hdds_tmp = [0u8; {value_size}];\n                __hdds_tmp.copy_from_slice(&src[offset..offset+{value_size}]);\n                {value_type}::from_le_bytes(__hdds_tmp)\n            }};\n"
            ),
        );
        push_fmt(dst, format_args!("            offset += {value_size};\n"));
        push_fmt(
            dst,
            format_args!("            {field_name}.insert(k, v);\n"),
        );
        dst.push_str("        }\n\n");
    }

    fn append_decode_map_generic(
        dst: &mut String,
        field_name: &str,
        key: &IdlType,
        key_size: Option<usize>,
        value: &IdlType,
        value_size: Option<usize>,
        version: CdrVersion,
    ) {
        dst.push_str("        for _ in 0..count {\n");
        Self::append_decode_map_component(dst, "            ", "key_tmp", key, key_size, version);
        Self::append_decode_map_component(
            dst,
            "            ",
            "value_tmp",
            value,
            value_size,
            version,
        );
        push_fmt(
            dst,
            format_args!("            {field_name}.insert(key_tmp, value_tmp);\n"),
        );
        dst.push_str("        }\n\n");
    }

    fn append_decode_map_component(
        dst: &mut String,
        indent: &str,
        var_name: &str,
        ty: &IdlType,
        fixed_size: Option<usize>,
        version: CdrVersion,
    ) {
        if let Some(size) = fixed_size {
            Self::append_decode_map_component_fixed(dst, indent, var_name, ty, size, version);
            return;
        }

        Self::append_decode_map_component_dynamic(dst, indent, var_name, ty, version);
    }

    fn append_decode_map_component_fixed(
        dst: &mut String,
        indent: &str,
        var_name: &str,
        ty: &IdlType,
        size: usize,
        version: CdrVersion,
    ) {
        let suffix = super::helpers::xcdr_method_suffix(version);
        let size_expr = size.to_string();
        Self::decode_buffer_check(dst, indent, &size_expr);
        match ty {
            IdlType::Primitive(PrimitiveType::Boolean) => {
                push_fmt(
                    dst,
                    format_args!("{indent}let {var_name} = src[offset] != 0;\n"),
                );
                push_fmt(dst, format_args!("{indent}offset += 1;\n"));
            }
            IdlType::Primitive(PrimitiveType::Char) => {
                push_fmt(
                    dst,
                    format_args!("{indent}let scalar = u32::from(src[offset]);\n"),
                );
                push_fmt(
                    dst,
                    format_args!(
                        "{indent}let {var_name} = char::from_u32(scalar).ok_or(CdrError::InvalidEncoding)?;\n"
                    ),
                );
                push_fmt(dst, format_args!("{indent}offset += 1;\n"));
            }
            IdlType::Primitive(PrimitiveType::WChar) => {
                push_fmt(
                    dst,
                    format_args!(
                        "{indent}let scalar = {{\n{indent}    let mut __hdds_tmp = [0u8; 4];\n{indent}    __hdds_tmp.copy_from_slice(&src[offset..offset+4]);\n{indent}    u32::from_le_bytes(__hdds_tmp)\n{indent}}};\n"
                    ),
                );
                push_fmt(
                    dst,
                    format_args!(
                        "{indent}let {var_name} = char::from_u32(scalar).ok_or(CdrError::InvalidEncoding)?;\n"
                    ),
                );
                push_fmt(dst, format_args!("{indent}offset += 4;\n"));
            }
            IdlType::Primitive(_) => {
                let type_name = Self::type_to_rust(ty);
                push_fmt(
                    dst,
                    format_args!(
                        "{indent}let {var_name} = {{\n{indent}    let mut __hdds_tmp = [0u8; {size}];\n{indent}    __hdds_tmp.copy_from_slice(&src[offset..offset+{size}]);\n{indent}    {type_name}::from_le_bytes(__hdds_tmp)\n{indent}}};\n"
                    ),
                );
                push_fmt(dst, format_args!("{indent}offset += {size};\n"));
            }
            _ => {
                let type_name = Self::type_to_rust(ty);
                push_fmt(
                    dst,
                    format_args!(
                        "{indent}let {var_name} = <{type_name}>::decode_{suffix}_le_at(src, &mut offset)?;\n"
                    ),
                );
            }
        }
    }

    fn append_decode_map_component_dynamic(
        dst: &mut String,
        indent: &str,
        var_name: &str,
        ty: &IdlType,
        version: CdrVersion,
    ) {
        let suffix = super::helpers::xcdr_method_suffix(version);
        if let IdlType::Primitive(PrimitiveType::Fixed { digits, scale }) = ty {
            Self::decode_buffer_check(dst, indent, "16");
            push_fmt(
                dst,
                format_args!(
                    "{indent}let raw = {{\n{indent}    let mut __hdds_tmp = [0u8; 16];\n{indent}    __hdds_tmp.copy_from_slice(&src[offset..offset+16]);\n{indent}    i128::from_le_bytes(__hdds_tmp)\n{indent}}};\n"
                ),
            );
            push_fmt(
                dst,
                format_args!(
                    "{indent}let {var_name} = Fixed::<{digits}, {scale}>::from_raw(raw);\n"
                ),
            );
            push_fmt(dst, format_args!("{indent}offset += 16;\n"));
        } else if matches!(ty, IdlType::Primitive(PrimitiveType::WChar)) {
            Self::decode_buffer_check(dst, indent, "4");
            push_fmt(
                dst,
                format_args!(
                    "{indent}let scalar = {{\n{indent}    let mut __hdds_tmp = [0u8; 4];\n{indent}    __hdds_tmp.copy_from_slice(&src[offset..offset+4]);\n{indent}    u32::from_le_bytes(__hdds_tmp)\n{indent}}};\n"
                ),
            );
            push_fmt(
                dst,
                format_args!(
                    "{indent}let {var_name} = char::from_u32(scalar).ok_or(CdrError::InvalidEncoding)?;\n"
                ),
            );
            push_fmt(dst, format_args!("{indent}offset += 4;\n"));
        } else if matches!(
            ty,
            IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString)
        ) || super::helpers::is_bounded_string(ty)
        {
            // Handle unbounded strings AND bounded strings (string<N> -> Sequence<Char, N>)
            Self::decode_string_into(dst, indent, var_name);
        } else {
            let type_name = Self::type_to_rust(ty);
            push_fmt(
                dst,
                format_args!(
                    "{indent}let {var_name} = <{type_name}>::decode_{suffix}_le_at(src, &mut offset)?;\n"
                ),
            );
        }
    }

    pub(super) fn decode_string_into(dst: &mut String, indent: &str, target_var: &str) {
        Self::decode_buffer_check(dst, indent, "4");
        push_fmt(
            dst,
            format_args!(
                "{indent}let len = {{\n{indent}    let mut __hdds_tmp = [0u8; 4];\n{indent}    __hdds_tmp.copy_from_slice(&src[offset..offset+4]);\n{indent}    u32::from_le_bytes(__hdds_tmp) as usize\n{indent}}};\n"
            ),
        );
        push_fmt(dst, format_args!("{indent}offset += 4;\n"));
        Self::decode_buffer_check(dst, indent, "len");
        push_fmt(
            dst,
            format_args!("{indent}// CDR: len includes null terminator\n"),
        );
        push_fmt(
            dst,
            format_args!("{indent}let str_len = len.saturating_sub(1);\n"),
        );
        push_fmt(
            dst,
            format_args!("{indent}let bytes = &src[offset..offset+str_len];\n"),
        );
        push_fmt(
            dst,
            format_args!("{indent}let {target_var} = String::from_utf8(bytes.to_vec()).map_err(|_| CdrError::InvalidEncoding)?;\n"),
        );
        push_fmt(
            dst,
            format_args!("{indent}offset += len; // len includes null terminator\n"),
        );
    }
}
