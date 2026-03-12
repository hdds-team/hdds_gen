// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! DDS trait generation for HDDS interoperability
//!
//! Generates `impl DDS for Type` with:
//! - `type_descriptor()` with FQN
//! - Type ID (FNV-1a hash of FQN)
//! - `XTypes` `CompleteTypeObject` (extensibility + member IDs)
//! - `compute_key()` for `@key` fields
//! - Field type mappings (IDL -> Rust -> `XTypes`)
//!
//! Note: `uninlined_format_args` allowed here due to extensive format!() usage
//! in code generation that would require significant refactoring.

#![allow(clippy::uninlined_format_args)]

use std::cell::RefCell;
use std::collections::HashSet;

use super::helpers;
use super::{push_fmt, RustGenerator};
use crate::ast::{Field, Struct};
use crate::types::{Annotation, IdlType, PrimitiveType};

thread_local! {
    /// Track types currently being emitted to detect recursive references
    /// (e.g., struct Node { child: Node; }) and break the cycle.
    // @audit-ok: thread_local! RefCell is inherently thread-safe (one instance per thread)
    static EMITTING_TYPES: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// How a `@key` field should be hashed in `compute_key()`.
enum KeyHashKind {
    /// String: iterate `as_bytes()`
    String,
    /// Numeric primitive: `to_le_bytes()` directly
    Numeric,
    /// `for b in (self.field as <cast>).to_le_bytes()` — used for bool(u8), char/enum(u32)
    Cast(&'static str),
    /// Complex type (struct, typedef, Fixed): CDR2-serialize then hash
    Cdr2,
}

/// Classify how a `@key` field type should be hashed.
fn classify_key_field(field_type: &IdlType) -> KeyHashKind {
    // String / WString / bounded string (sequence<char>)
    let is_string = matches!(
        field_type,
        IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString)
    ) || matches!(
        field_type,
        IdlType::Sequence { inner, .. }
            if matches!(**inner, IdlType::Primitive(PrimitiveType::Char | PrimitiveType::WChar))
    );
    if is_string {
        return KeyHashKind::String;
    }
    // Named type: check if it's an enum (cast u32) or struct/typedef (CDR2)
    if let IdlType::Named(name) = field_type {
        return if matches!(
            helpers::lookup_type_def(name),
            Some((helpers::TypeDef::Enum(_), _))
        ) {
            KeyHashKind::Cast("u32")
        } else {
            KeyHashKind::Cdr2
        };
    }
    match field_type {
        IdlType::Primitive(PrimitiveType::Boolean) => KeyHashKind::Cast("u8"),
        IdlType::Primitive(PrimitiveType::Char | PrimitiveType::WChar) => {
            KeyHashKind::Cast("u32")
        }
        IdlType::Primitive(PrimitiveType::Fixed { .. }) => KeyHashKind::Cdr2,
        _ => KeyHashKind::Numeric,
    }
}

/// Emit hash code for one `@key` field into `output`.
fn emit_key_field_hash(output: &mut String, field: &str, kind: &KeyHashKind) {
    push_fmt(output, format_args!("            // Hash @key field: {field}\n"));
    match kind {
        KeyHashKind::String => {
            push_fmt(output, format_args!(
                "            for &b in self.{field}.as_bytes() {{\n"
            ));
        }
        KeyHashKind::Numeric => {
            push_fmt(output, format_args!(
                "            for b in self.{field}.to_le_bytes() {{\n"
            ));
        }
        KeyHashKind::Cast(ty) => {
            push_fmt(output, format_args!(
                "            for b in (self.{field} as {ty}).to_le_bytes() {{\n"
            ));
        }
        KeyHashKind::Cdr2 => {
            output.push_str("            {\n");
            output.push_str("                use ::hdds::core::ser::Cdr2Encode;\n");
            output.push_str("                let mut _kbuf = [0u8; 4096];\n");
            push_fmt(output, format_args!(
                "                if let Ok(_kn) = self.{field}.encode_cdr2_le(&mut _kbuf) {{\n"
            ));
            output.push_str("                    for &b in &_kbuf[.._kn] {\n");
            output.push_str("                        hash ^= b as u64;\n");
            output.push_str("                        hash = hash.wrapping_mul(PRIME);\n");
            output.push_str("                    }\n");
            output.push_str("                }\n");
            output.push_str("            }\n\n");
            return;
        }
    }
    // Common suffix for non-CDR2 variants
    output.push_str("                hash ^= b as u64;\n");
    output.push_str("                hash = hash.wrapping_mul(PRIME);\n");
    output.push_str("            }\n\n");
}

impl RustGenerator {
    /// Generate DDS trait implementation for a struct
    ///
    /// # Arguments
    /// - `s`: The struct definition
    /// - `module_path`: Optional module path (e.g., `TemperatureData`) for FQN
    ///
    /// # Returns
    /// Generated Rust code implementing `::hdds::api::DDS`
    // codegen function - line count from template output
    #[allow(clippy::too_many_lines)]
    pub(super) fn emit_dds_trait_impl(s: &Struct, module_path: Option<&str>) -> String {
        let mut output = String::new();

        // Build FQN: "Module::Struct" or just "Struct"
        let fqn = module_path.map_or_else(
            || s.name.clone(),
            |module| format!("{}::{}", module, s.name),
        );

        // Calculate type_id (FNV-1a hash)
        let type_id = Self::compute_fnv1a_hash(&fqn);

        // Calculate struct size and alignment
        let (size_bytes, alignment, field_layouts) = Self::compute_struct_layout(&s.fields);

        push_fmt(
            &mut output,
            format_args!("    // === DDS Trait (auto-generated by hddsgen) ===\n"),
        );
        push_fmt(
            &mut output,
            format_args!("    impl ::hdds::api::DDS for {} {{\n", s.name),
        );

        // type_descriptor()
        output.push_str(
            "        fn type_descriptor() -> &'static ::hdds::core::types::TypeDescriptor {\n",
        );
        output.push_str("            static DESC: ::hdds::core::types::TypeDescriptor = ::hdds::core::types::TypeDescriptor {\n");
        push_fmt(
            &mut output,
            format_args!(
                "                type_id: {:#010X},  // FNV-1a hash of \"{}\"\n",
                type_id, fqn
            ),
        );
        push_fmt(
            &mut output,
            format_args!("                type_name: \"{}\",\n", fqn),
        );
        push_fmt(
            &mut output,
            format_args!("                size_bytes: {},\n", size_bytes),
        );
        push_fmt(
            &mut output,
            format_args!("                alignment: {},\n", alignment),
        );
        let is_variable = Self::has_variable_size_field(&s.fields);
        push_fmt(
            &mut output,
            format_args!("                is_variable_size: {},\n", is_variable),
        );
        output.push_str("                fields: &[\n");

        // Generate field layouts
        for (field_name, offset, field_type) in &field_layouts {
            let (size, align, prim_kind) = Self::field_type_info(field_type);
            push_fmt(
                &mut output,
                format_args!("                    ::hdds::core::types::FieldLayout {{\n"),
            );
            push_fmt(
                &mut output,
                format_args!("                        name: \"{}\",\n", field_name),
            );
            push_fmt(
                &mut output,
                format_args!("                        offset_bytes: {},\n", offset),
            );
            push_fmt(
                &mut output,
                format_args!(
                    "                        field_type: ::hdds::core::types::FieldType::Primitive(\n"
                ),
            );
            push_fmt(
                &mut output,
                format_args!(
                    "                            ::hdds::core::types::PrimitiveKind::{}\n",
                    prim_kind
                ),
            );
            output.push_str("                        ),\n");
            push_fmt(
                &mut output,
                format_args!("                        alignment: {},\n", align),
            );
            push_fmt(
                &mut output,
                format_args!("                        size_bytes: {},\n", size),
            );
            output.push_str("                        element_type: None,\n");
            output.push_str("                    },\n");
        }

        output.push_str("                ],\n");
        output.push_str("            };\n");
        output.push_str("            &DESC\n");
        output.push_str("        }\n\n");

        // encode_cdr2()
        output.push_str(
            "        fn encode_cdr2(&self, buf: &mut [u8]) -> ::hdds::api::Result<usize> {\n",
        );
        output.push_str("            use ::hdds::core::ser::Cdr2Encode;\n");
        output.push_str("            self.encode_cdr2_le(buf).map_err(Into::into)\n");
        output.push_str("        }\n\n");

        // decode_cdr2()
        output.push_str("        fn decode_cdr2(buf: &[u8]) -> ::hdds::api::Result<Self> {\n");
        output.push_str("            use ::hdds::core::ser::Cdr2Decode;\n");
        output.push_str(
            "            Self::decode_cdr2_le(buf).map(|(val, _)| val).map_err(Into::into)\n",
        );
        output.push_str("        }\n\n");

        // get_type_object() (XTypes)
        output.push_str(&Self::emit_xtypes_type_object(s, &fqn, &field_layouts));

        // compute_key() for @key fields
        output.push_str(&Self::emit_compute_key_method(s));

        // has_key() constant
        output.push_str(&Self::emit_has_key_method(s));

        output.push_str("    }\n    \n");
        output
    }

    /// Generate `XTypes` `CompleteTypeObject`
    ///
    /// For `@mutable`/`@appendable` structs we must:
    /// - set `StructTypeFlag::IS_MUTABLE` / `IS_APPENDABLE`
    /// - use hash-based `MemberId` consistent with `PL_CDR2` encoding
    fn emit_xtypes_type_object(
        s: &Struct,
        fqn: &str,
        field_layouts: &[(String, usize, IdlType)],
    ) -> String {
        let mut output = String::new();

        output.push_str(
            "        fn get_type_object() -> Option<::hdds::xtypes::CompleteTypeObject> {\n",
        );
        output.push_str("            Some(::hdds::xtypes::CompleteTypeObject::Struct(\n");
        output.push_str("                ::hdds::xtypes::CompleteStructType {\n");
        // Derive struct extensibility flags from IDL annotations
        if helpers::is_mutable_struct(s) {
            output.push_str(
                "                    struct_flags: ::hdds::xtypes::StructTypeFlag::IS_MUTABLE,\n",
            );
        } else {
            output.push_str(
                "                    struct_flags: ::hdds::xtypes::StructTypeFlag::IS_FINAL,\n",
            );
        }
        output.push_str("                    header: ::hdds::xtypes::CompleteStructHeader {\n");
        output.push_str("                        base_type: None,\n");
        push_fmt(
            &mut output,
            format_args!(
                "                        detail: ::hdds::xtypes::CompleteTypeDetail::new(\"{}\"),\n",
                fqn
            ),
        );
        output.push_str("                    },\n");
        output.push_str("                    member_seq: vec![\n");

        // Generate CompleteStructMember for each field
        for (idx, (field_name, _offset, field_type)) in field_layouts.iter().enumerate() {
            // For mutable structs, use hash-based MemberId to match PL_CDR2 EMHEADER encoding.
            // For final/appendable, use sequential MemberId per spec.
            let member_id_val = if helpers::is_mutable_struct(s) {
                let field = &s.fields[idx];
                Self::compute_member_id(s, idx, field)
            } else {
                // @audit-ok: safe cast - struct field index always << u32::MAX
                #[allow(clippy::cast_possible_truncation)]
                {
                    idx as u32
                }
            };

            let type_id_code = Self::emit_member_type_id(field_type);
            output.push_str("                        ::hdds::xtypes::CompleteStructMember {\n");
            output.push_str(
                "                            common: ::hdds::xtypes::CommonStructMember {\n",
            );
            push_fmt(
                &mut output,
                format_args!(
                    "                                member_id: {},\n",
                    member_id_val
                ),
            );
            output.push_str("                                member_flags: ::hdds::xtypes::MemberFlag::empty(),\n");
            push_fmt(
                &mut output,
                format_args!(
                    "                                member_type_id: {},\n",
                    type_id_code
                ),
            );
            output.push_str("                            },\n");
            push_fmt(
                &mut output,
                format_args!(
                    "                            detail: ::hdds::xtypes::CompleteMemberDetail::new(\"{}\"),\n",
                    field_name
                ),
            );
            output.push_str("                        },\n");
        }

        output.push_str("                    ],\n");
        output.push_str("                }\n");
        output.push_str("            ))\n");
        output.push_str("        }\n");
        output
    }

    /// Compute FNV-1a hash (32-bit) for type ID
    /// Same algorithm as hdds build.rs
    fn compute_fnv1a_hash(s: &str) -> u32 {
        let mut hash = 2_166_136_261_u32;
        for byte in s.bytes() {
            hash ^= u32::from(byte);
            hash = hash.wrapping_mul(16_777_619);
        }
        hash
    }

    /// Compute struct layout (size, alignment, field offsets)
    /// Returns: (`total_size`, `max_alignment`, `[(field_name, offset, type)]`)
    fn compute_struct_layout(fields: &[Field]) -> (usize, usize, Vec<(String, usize, IdlType)>) {
        let mut current_offset = 0usize;
        let mut max_alignment = 1usize;
        let mut layouts = Vec::new();

        for field in fields {
            let (size, alignment, _) = Self::field_type_info(&field.field_type);

            // Align offset to field alignment
            current_offset = Self::align_to(current_offset, alignment);
            max_alignment = max_alignment.max(alignment);

            layouts.push((field.name.clone(), current_offset, field.field_type.clone()));
            current_offset += size;
        }

        // Struct total size is aligned to max field alignment
        let total_size = Self::align_to(current_offset, max_alignment);
        (total_size, max_alignment, layouts)
    }

    /// Align offset to alignment boundary
    const fn align_to(offset: usize, alignment: usize) -> usize {
        (offset + alignment - 1) & !(alignment - 1)
    }

    /// Check if any field is variable-size (strings, sequences, maps, optional)
    fn has_variable_size_field(fields: &[Field]) -> bool {
        fields
            .iter()
            .any(|f| f.is_optional() || Self::is_variable_size_type(&f.field_type))
    }

    /// Check if a type is variable-size in CDR2
    fn is_variable_size_type(ty: &IdlType) -> bool {
        match ty {
            IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString)
            | IdlType::Sequence { .. }
            | IdlType::Map { .. } => true,
            IdlType::Array { inner, .. } => Self::is_variable_size_type(inner),
            _ => false,
        }
    }

    /// Get field type info: (size, alignment, `PrimitiveKind` name)
    const fn field_type_info(idl_type: &IdlType) -> (usize, usize, &'static str) {
        use crate::types::PrimitiveType;

        match idl_type {
            IdlType::Primitive(prim) => match prim {
                PrimitiveType::Octet
                | PrimitiveType::UInt8
                | PrimitiveType::Int8
                | PrimitiveType::Char
                | PrimitiveType::Boolean => (1, 1, "U8"),
                PrimitiveType::Short | PrimitiveType::Int16 => (2, 2, "I16"),
                PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => (2, 2, "U16"),
                PrimitiveType::Long | PrimitiveType::Int32 => (4, 4, "I32"),
                PrimitiveType::UnsignedLong
                | PrimitiveType::UInt32
                | PrimitiveType::Void
                | PrimitiveType::WChar => (4, 4, "U32"),
                PrimitiveType::LongLong | PrimitiveType::Int64 => (8, 8, "I64"),
                PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => (8, 8, "U64"),
                PrimitiveType::Float => (4, 4, "F32"),
                PrimitiveType::Double => (8, 8, "F64"),
                PrimitiveType::LongDouble => (16, 8, "F64"),
                PrimitiveType::String | PrimitiveType::WString => (24, 8, "String"),
                PrimitiveType::Fixed { .. } => (16, 8, "I64"),
            },
            IdlType::Sequence { inner, .. } => {
                // Bounded string (string<N>) has same layout as String
                if matches!(
                    **inner,
                    IdlType::Primitive(PrimitiveType::Char | PrimitiveType::WChar)
                ) {
                    return (24, 8, "String");
                }
                (24, 8, "U64") // Vec<T> = ptr + len + cap
            }
            IdlType::Array { inner, size } => {
                let (inner_size, inner_align, inner_kind) = Self::field_type_info(inner);
                (inner_size * (*size as usize), inner_align, inner_kind)
            }
            IdlType::Map { .. } => (48, 8, "U64"), // HashMap layout
            IdlType::Named(_) => (4, 4, "U32"),    // Enums are u32; nested structs vary
        }
    }

    /// Generate complete `TypeIdentifier` expression for a field type.
    ///
    /// For primitive types, returns `::hdds::xtypes::TypeIdentifier::TK_*`.
    /// For named types (structs/enums), generates an inline `TypeIdentifier::Inline(...)`
    /// embedding the full `CompleteTypeObject` so that `XTypes` auto-decode can
    /// resolve the type hierarchy without a registry.
    fn emit_member_type_id(idl_type: &IdlType) -> String {
        use crate::types::PrimitiveType;

        match idl_type {
            IdlType::Primitive(prim) => {
                let suffix = match prim {
                    PrimitiveType::Octet | PrimitiveType::UInt8 | PrimitiveType::Void => "TK_BYTE",
                    PrimitiveType::Char => "TK_CHAR8",
                    PrimitiveType::WChar => "TK_CHAR16",
                    PrimitiveType::Boolean => "TK_BOOLEAN",
                    PrimitiveType::Short | PrimitiveType::Int16 => "TK_INT16",
                    PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => "TK_UINT16",
                    PrimitiveType::Long | PrimitiveType::Int32 => "TK_INT32",
                    PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => "TK_UINT32",
                    PrimitiveType::LongLong
                    | PrimitiveType::Int64
                    | PrimitiveType::Fixed { .. } => "TK_INT64",
                    PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => "TK_UINT64",
                    PrimitiveType::Float => "TK_FLOAT32",
                    PrimitiveType::Double | PrimitiveType::LongDouble => "TK_FLOAT64",
                    PrimitiveType::Int8 => "TK_INT8",
                    PrimitiveType::String => "TK_STRING8",
                    PrimitiveType::WString => "TK_STRING16",
                };
                format!("::hdds::xtypes::TypeIdentifier::{}", suffix)
            }
            IdlType::Sequence { inner, .. } => {
                if matches!(
                    **inner,
                    IdlType::Primitive(PrimitiveType::Char | PrimitiveType::WChar)
                ) {
                    return "::hdds::xtypes::TypeIdentifier::TK_STRING8".to_string();
                }
                "::hdds::xtypes::TypeIdentifier::TK_UINT32".to_string()
            }
            IdlType::Array { inner, .. } => Self::emit_member_type_id(inner),
            IdlType::Map { .. } => "::hdds::xtypes::TypeIdentifier::TK_UINT32".to_string(),
            IdlType::Named(name) => Self::emit_inline_type_object(name),
        }
    }

    /// Generate an inline `TypeIdentifier::Inline(Box::new(...))` for a named type.
    ///
    /// Looks up the type definition in the thread-local type index and generates
    /// the appropriate `CompleteTypeObject` (Struct or Enumerated).
    /// Falls back to `TK_UINT32` if the type is not found (e.g., typedefs).
    fn emit_inline_type_object(name: &str) -> String {
        // Cycle detection: if we are already emitting this type (recursive struct),
        // return a placeholder to break infinite recursion.
        let is_cycle = EMITTING_TYPES.with(|set| set.borrow().contains(name));
        if is_cycle {
            return "::hdds::xtypes::TypeIdentifier::TK_UINT32".to_string();
        }

        let Some((type_def, module)) = helpers::lookup_type_def(name) else {
            // Unknown type (typedef, etc.) - fall back to u32 placeholder
            return "::hdds::xtypes::TypeIdentifier::TK_UINT32".to_string();
        };

        let fqn = module
            .as_ref()
            .map_or_else(|| name.to_string(), |m| format!("{}::{}", m, name));

        EMITTING_TYPES.with(|set| set.borrow_mut().insert(name.to_string()));

        let result = match type_def {
            helpers::TypeDef::Struct(s) => Self::emit_inline_struct_type_object(&s, &fqn),
            helpers::TypeDef::Enum(e) => Self::emit_inline_enum_type_object(&e, &fqn),
        };

        EMITTING_TYPES.with(|set| set.borrow_mut().remove(name));

        result
    }

    /// Generate inline `CompleteTypeObject::Struct(...)` for a named struct.
    fn emit_inline_struct_type_object(s: &Struct, fqn: &str) -> String {
        let mut out = String::new();
        out.push_str("::hdds::xtypes::TypeIdentifier::Inline(Box::new(\n");
        out.push_str(
            "                                    ::hdds::xtypes::CompleteTypeObject::Struct(\n",
        );
        out.push_str(
            "                                        ::hdds::xtypes::CompleteStructType {\n",
        );
        out.push_str("                                            struct_flags: ::hdds::xtypes::StructTypeFlag::IS_FINAL,\n");
        out.push_str("                                            header: ::hdds::xtypes::CompleteStructHeader {\n");
        out.push_str("                                                base_type: None,\n");
        push_fmt(&mut out, format_args!(
            "                                                detail: ::hdds::xtypes::CompleteTypeDetail::new(\"{}\"),\n", fqn
        ));
        out.push_str("                                            },\n");
        out.push_str("                                            member_seq: vec![\n");

        for (idx, field) in s.fields.iter().enumerate() {
            let inner_type_id = Self::emit_member_type_id(&field.field_type);
            out.push_str("                                                ::hdds::xtypes::CompleteStructMember {\n");
            out.push_str("                                                    common: ::hdds::xtypes::CommonStructMember {\n");
            push_fmt(
                &mut out,
                format_args!(
                    "                                                        member_id: {},\n",
                    idx
                ),
            );
            out.push_str("                                                        member_flags: ::hdds::xtypes::MemberFlag::empty(),\n");
            push_fmt(
                &mut out,
                format_args!(
                    "                                                        member_type_id: {},\n",
                    inner_type_id
                ),
            );
            out.push_str("                                                    },\n");
            push_fmt(&mut out, format_args!(
                "                                                    detail: ::hdds::xtypes::CompleteMemberDetail::new(\"{}\"),\n", field.name
            ));
            out.push_str("                                                },\n");
        }

        out.push_str("                                            ],\n");
        out.push_str("                                        }\n");
        out.push_str("                                    )\n");
        out.push_str("                                ))");
        out
    }

    /// Generate inline `CompleteTypeObject::Enumerated(...)` for a named enum.
    fn emit_inline_enum_type_object(e: &crate::ast::Enum, fqn: &str) -> String {
        let mut out = String::new();
        out.push_str("::hdds::xtypes::TypeIdentifier::Inline(Box::new(\n");
        out.push_str(
            "                                    ::hdds::xtypes::CompleteTypeObject::Enumerated(\n",
        );
        out.push_str(
            "                                        ::hdds::xtypes::CompleteEnumeratedType {\n",
        );
        out.push_str("                                            header: ::hdds::xtypes::CompleteEnumeratedHeader {\n");
        out.push_str("                                                bit_bound: 32,\n");
        push_fmt(&mut out, format_args!(
            "                                                detail: ::hdds::xtypes::CompleteTypeDetail::new(\"{}\"),\n", fqn
        ));
        out.push_str("                                            },\n");
        out.push_str("                                            literal_seq: vec![\n");

        for (idx, variant) in e.variants.iter().enumerate() {
            // @audit-ok: enum variant index always fits in i64
            let value = variant
                .value
                .unwrap_or_else(|| i64::try_from(idx).unwrap_or(0));
            out.push_str("                                                ::hdds::xtypes::CompleteEnumeratedLiteral {\n");
            out.push_str("                                                    common: ::hdds::xtypes::CommonEnumeratedLiteral {\n");
            push_fmt(
                &mut out,
                format_args!(
                    "                                                        value: {},\n",
                    value
                ),
            );
            out.push_str("                                                        flags: ::hdds::xtypes::EnumeratedLiteralFlag::empty(),\n");
            out.push_str("                                                    },\n");
            push_fmt(&mut out, format_args!(
                "                                                    detail: ::hdds::xtypes::CompleteMemberDetail::new(\"{}\"),\n", variant.name
            ));
            out.push_str("                                                },\n");
        }

        out.push_str("                                            ],\n");
        out.push_str("                                        }\n");
        out.push_str("                                    )\n");
        out.push_str("                                ))");
        out
    }

    /// Generate `compute_key()` method for `@key` fields
    ///
    /// Returns a 16-byte key hash computed from `@key` fields using FNV-1a.
    fn emit_compute_key_method(s: &Struct) -> String {
        let mut output = String::new();

        let key_fields: Vec<(&str, &IdlType)> = s
            .fields
            .iter()
            .filter(|f| f.annotations.iter().any(|a| matches!(a, Annotation::Key)))
            .map(|f| (f.name.as_str(), &f.field_type))
            .collect();

        output.push_str(
            "\n        /// Compute instance key hash from @key fields (FNV-1a, 16 bytes)\n",
        );
        output.push_str("        fn compute_key(&self) -> [u8; 16] {\n");

        if key_fields.is_empty() {
            output.push_str("            // No @key fields - return zeroed hash\n");
            output.push_str("            [0u8; 16]\n");
        } else {
            output.push_str("            // FNV-1a hash of @key fields\n");
            output.push_str("            let mut hash: u64 = 14695981039346656037;\n");
            output.push_str("            const PRIME: u64 = 1099511628211;\n\n");

            for (field, field_type) in &key_fields {
                let kind = classify_key_field(field_type);
                emit_key_field_hash(&mut output, field, &kind);
            }

            output.push_str("            // Expand to 16 bytes\n");
            output.push_str("            let mut key = [0u8; 16];\n");
            output.push_str("            key[..8].copy_from_slice(&hash.to_le_bytes());\n");
            output.push_str("            hash = hash.wrapping_mul(PRIME);\n");
            output.push_str("            key[8..].copy_from_slice(&hash.to_le_bytes());\n");
            output.push_str("            key\n");
        }

        output.push_str("        }\n");
        output
    }

    /// Generate `has_key()` constant for checking `@key` field presence
    fn emit_has_key_method(s: &Struct) -> String {
        let has_key = s
            .fields
            .iter()
            .any(|f| f.annotations.iter().any(|a| matches!(a, Annotation::Key)));

        let mut output = String::new();
        output.push_str("\n        /// Returns true if this type has @key fields\n");
        push_fmt(
            &mut output,
            format_args!(
                "        fn has_key() -> bool {{ {} }}\n",
                if has_key { "true" } else { "false" }
            ),
        );
        output
    }
}
