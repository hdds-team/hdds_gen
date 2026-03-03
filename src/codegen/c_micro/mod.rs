// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! C Micro code generator for embedded MCUs
//!
//! Generates header-only C code compatible with C89/C99.
//! Target: STM32, AVR, PIC, ESP32, any MCU with a C compiler.
//!
//! # Supported IDL Constructs
//!
//! | Construct | Status | Notes |
//! |-----------|--------|-------|
//! | `struct` | ✅ Supported | With fixed-size arrays for strings/sequences |
//! | `enum` | ✅ Supported | Serialized as `int32_t` |
//! | `union` | ✅ Supported | Discriminated unions |
//! | `module` | ✅ Supported | Mapped to C namespace prefix |
//! | `sequence<T, N>` | ✅ Supported | Fixed-size array + length field |
//! | `string<N>` | ✅ Supported | `char[N+1]` with null terminator |
//! | `@key` | ✅ Supported | `compute_key()` function generated |
//!
//! # Unsupported Constructs (Embedded C Limitations)
//!
//! | Construct | Reason |
//! |-----------|--------|
//! | `map<K, V>` | No dynamic allocation, no hash tables |
//! | `bitset` | Not implemented for embedded C |
//! | `bitmask` | Not implemented for embedded C |
//! | `typedef` | Silently ignored |
//! | `const` | Silently ignored |
//! | Unbounded `sequence<T>` | No dynamic memory (`malloc`) |
//! | Unbounded `string` | No dynamic memory (`malloc`) |
//! | `@optional` | No `std::optional` in C |
//! | `@extensibility(MUTABLE)` | DHEADER/EMHEADER not implemented |
//! | `wchar` / `wstring` | Limited wide char support on MCUs |
//!
//! Note: uninlined_format_args allowed here due to extensive `format_args!()` usage
//! in code generation that would require significant refactoring.

#![allow(clippy::uninlined_format_args)]

use crate::ast::{Definition, Enum, IdlFile, Module, Struct, Union, UnionLabel};
use crate::codegen::CodeGenerator;
use crate::error::Result;
use crate::types::{Annotation, IdlType, PrimitiveType};

use super::keywords::c_ident;

/// Configuration for C micro code generation
#[derive(Debug, Clone)]
pub struct CMicroConfig {
    /// Maximum length for char[] strings
    pub max_string_len: usize,
    /// Maximum length for sequences
    pub max_sequence_len: usize,
    /// Function prefix (e.g., "hdds_" or "")
    pub prefix: String,
    /// Include guard prefix
    pub guard_prefix: String,
}

impl Default for CMicroConfig {
    fn default() -> Self {
        Self {
            max_string_len: 64,
            max_sequence_len: 32,
            prefix: String::new(),
            guard_prefix: String::new(),
        }
    }
}

/// Generates header-only C code for embedded MCUs
pub struct CMicroGenerator {
    config: CMicroConfig,
}

impl CMicroGenerator {
    /// Creates a new C micro generator with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: CMicroConfig::default(),
        }
    }

    /// Creates a new C micro generator with custom configuration.
    #[must_use]
    pub const fn with_config(config: CMicroConfig) -> Self {
        Self { config }
    }

    fn push_fmt(output: &mut String, args: std::fmt::Arguments<'_>) {
        use std::fmt::Write;
        let _ = write!(output, "{}", args);
    }

    /// Check if a type is a bounded string (string<N> -> sequence<char, N>)
    fn is_bounded_string(idl_type: &IdlType) -> Option<u32> {
        if let IdlType::Sequence { inner, bound } = idl_type {
            if **inner == IdlType::Primitive(PrimitiveType::Char) {
                return *bound;
            }
        }
        None
    }

    /// Check if a type is a bounded wstring (wstring<N> -> sequence<wchar, N>)
    fn is_bounded_wstring(idl_type: &IdlType) -> Option<u32> {
        if let IdlType::Sequence { inner, bound } = idl_type {
            if **inner == IdlType::Primitive(PrimitiveType::WChar) {
                return *bound;
            }
        }
        None
    }

    /// Convert IDL type name to C type
    fn type_to_c(&self, idl_type: &IdlType) -> String {
        // Check for bounded string first (string<N> -> char[N])
        if let Some(bound) = Self::is_bounded_string(idl_type) {
            return format!("char[{}]", bound);
        }
        // Check for bounded wstring (wstring<N> -> uint16_t[N])
        if let Some(bound) = Self::is_bounded_wstring(idl_type) {
            return format!("uint16_t[{}]", bound);
        }

        match idl_type {
            IdlType::Primitive(p) => self.primitive_to_c(p),
            IdlType::Named(name) => name.clone(),
            IdlType::Sequence { inner, bound } => {
                let inner_c = self.type_to_c(inner);
                // @audit-ok: safe cast - config values are bounded by design (default: 32)
                #[allow(clippy::cast_possible_truncation)]
                let cap = bound.unwrap_or(self.config.max_sequence_len as u32);
                // Generate inline struct for sequence
                format!("struct {{ {} data[{}]; uint32_t count; }}", inner_c, cap)
            }
            IdlType::Array { inner, size } => {
                let inner_c = self.type_to_c(inner);
                format!("{}[{}]", inner_c, size)
            }
            IdlType::Map { .. } => "/* Map not supported */".to_string(),
        }
    }

    fn primitive_to_c(&self, p: &PrimitiveType) -> String {
        match p {
            PrimitiveType::Boolean | PrimitiveType::Octet | PrimitiveType::UInt8 => {
                "uint8_t".to_string()
            }
            PrimitiveType::Char => "char".to_string(),
            PrimitiveType::WChar | PrimitiveType::UInt16 | PrimitiveType::UnsignedShort => {
                "uint16_t".to_string()
            }
            PrimitiveType::Int8 => "int8_t".to_string(),
            PrimitiveType::Int16 | PrimitiveType::Short => "int16_t".to_string(),
            PrimitiveType::UInt32 | PrimitiveType::UnsignedLong => "uint32_t".to_string(),
            PrimitiveType::Int32 | PrimitiveType::Long => "int32_t".to_string(),
            PrimitiveType::UInt64 | PrimitiveType::UnsignedLongLong => "uint64_t".to_string(),
            PrimitiveType::Int64 | PrimitiveType::LongLong | PrimitiveType::Fixed { .. } => {
                "int64_t".to_string() // Fixed is approximate
            }
            PrimitiveType::Float => "float".to_string(),
            PrimitiveType::Double | PrimitiveType::LongDouble => "double".to_string(),
            PrimitiveType::String | PrimitiveType::WString => {
                format!("char[{}]", self.config.max_string_len)
            }
            PrimitiveType::Void => "void".to_string(),
        }
    }

    /// Get CDR write function for a primitive type
    const fn primitive_write_call(p: &PrimitiveType) -> &'static str {
        match p {
            PrimitiveType::Boolean => "hdds_cdr_write_bool",
            PrimitiveType::Char | PrimitiveType::Octet | PrimitiveType::UInt8 => {
                "hdds_cdr_write_u8"
            }
            PrimitiveType::Int8 => "hdds_cdr_write_i8",
            PrimitiveType::UInt16 | PrimitiveType::UnsignedShort | PrimitiveType::WChar => {
                "hdds_cdr_write_u16"
            }
            PrimitiveType::Int16 | PrimitiveType::Short => "hdds_cdr_write_i16",
            PrimitiveType::UInt32 | PrimitiveType::UnsignedLong => "hdds_cdr_write_u32",
            PrimitiveType::Int32 | PrimitiveType::Long => "hdds_cdr_write_i32",
            PrimitiveType::UInt64 | PrimitiveType::UnsignedLongLong => "hdds_cdr_write_u64",
            PrimitiveType::Int64 | PrimitiveType::LongLong => "hdds_cdr_write_i64",
            PrimitiveType::Float => "hdds_cdr_write_f32",
            PrimitiveType::Double | PrimitiveType::LongDouble => "hdds_cdr_write_f64",
            PrimitiveType::String | PrimitiveType::WString => "hdds_cdr_write_string",
            _ => "/* unsupported */",
        }
    }

    /// Get CDR read function for a primitive type
    const fn primitive_read_call(p: &PrimitiveType) -> &'static str {
        match p {
            PrimitiveType::Boolean => "hdds_cdr_read_bool",
            PrimitiveType::Char | PrimitiveType::Octet | PrimitiveType::UInt8 => "hdds_cdr_read_u8",
            PrimitiveType::Int8 => "hdds_cdr_read_i8",
            PrimitiveType::UInt16 | PrimitiveType::UnsignedShort | PrimitiveType::WChar => {
                "hdds_cdr_read_u16"
            }
            PrimitiveType::Int16 | PrimitiveType::Short => "hdds_cdr_read_i16",
            PrimitiveType::UInt32 | PrimitiveType::UnsignedLong => "hdds_cdr_read_u32",
            PrimitiveType::Int32 | PrimitiveType::Long => "hdds_cdr_read_i32",
            PrimitiveType::UInt64 | PrimitiveType::UnsignedLongLong => "hdds_cdr_read_u64",
            PrimitiveType::Int64 | PrimitiveType::LongLong => "hdds_cdr_read_i64",
            PrimitiveType::Float => "hdds_cdr_read_f32",
            PrimitiveType::Double | PrimitiveType::LongDouble => "hdds_cdr_read_f64",
            _ => "/* unsupported */",
        }
    }

    /// Convert name to lowercase C identifier
    fn to_c_ident(name: &str) -> String {
        let mut result = String::new();
        for (i, c) in name.chars().enumerate() {
            if c.is_uppercase() {
                if i > 0 {
                    result.push('_');
                }
                result.push(c.to_ascii_lowercase());
            } else {
                result.push(c);
            }
        }
        result
    }

    /// Get byte size of primitive type for key hashing
    const fn get_primitive_size(t: &IdlType) -> &'static str {
        match t {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Boolean
                | PrimitiveType::Char
                | PrimitiveType::Octet
                | PrimitiveType::Int8
                | PrimitiveType::UInt8 => "1",
                PrimitiveType::Short
                | PrimitiveType::Int16
                | PrimitiveType::UnsignedShort
                | PrimitiveType::UInt16 => "2",
                PrimitiveType::Long
                | PrimitiveType::Int32
                | PrimitiveType::UnsignedLong
                | PrimitiveType::UInt32
                | PrimitiveType::Float
                | PrimitiveType::WChar => "4",
                PrimitiveType::LongLong
                | PrimitiveType::Int64
                | PrimitiveType::UnsignedLongLong
                | PrimitiveType::UInt64
                | PrimitiveType::Double
                | PrimitiveType::LongDouble => "8",
                PrimitiveType::Fixed { .. } => "16",
                // Strings are handled specially (iterate bytes)
                PrimitiveType::String | PrimitiveType::WString | PrimitiveType::Void => "0",
            },
            // Complex types not supported as @key fields in c-micro
            _ => "0",
        }
    }
}

impl Default for CMicroGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGenerator for CMicroGenerator {
    fn generate(&self, ast: &IdlFile) -> Result<String> {
        let mut output = String::new();

        // Header
        output.push_str("/**\n");
        output.push_str(" * @file generated_types.h\n");
        output.push_str(" * @brief Auto-generated by hdds_gen --target c-micro\n");
        output.push_str(" * @note DO NOT EDIT\n");
        output.push_str(" */\n\n");

        output.push_str("#ifndef GENERATED_TYPES_H\n");
        output.push_str("#define GENERATED_TYPES_H\n\n");

        output.push_str("#include <stdint.h>\n");
        output.push_str("#include <string.h>\n");
        output.push_str("#include \"hdds_micro_cdr.h\"\n\n");

        output.push_str("#ifdef __cplusplus\n");
        output.push_str("extern \"C\" {\n");
        output.push_str("#endif\n\n");

        // Forward declarations
        output.push_str("/* Forward declarations */\n");
        for def in &ast.definitions {
            match def {
                Definition::Struct(s) => {
                    Self::push_fmt(
                        &mut output,
                        format_args!("typedef struct {} {};\n", s.name, s.name),
                    );
                }
                Definition::Enum(e) => {
                    Self::push_fmt(
                        &mut output,
                        format_args!("typedef enum {} {};\n", e.name, e.name),
                    );
                }
                Definition::Union(u) => {
                    Self::push_fmt(
                        &mut output,
                        format_args!("typedef struct {} {};\n", u.name, u.name),
                    );
                }
                Definition::Module(m) => {
                    Self::emit_forward_decls(&mut output, m);
                }
                _ => {}
            }
        }
        output.push('\n');

        // Generate definitions
        for def in &ast.definitions {
            match def {
                Definition::Struct(s) => output.push_str(&self.generate_struct(s)),
                Definition::Enum(e) => output.push_str(&Self::generate_enum(e)),
                Definition::Union(u) => output.push_str(&self.generate_union(u)),
                Definition::Module(m) => output.push_str(&self.generate_module(m)),
                _ => {}
            }
        }

        output.push_str("#ifdef __cplusplus\n");
        output.push_str("}\n");
        output.push_str("#endif\n\n");
        output.push_str("#endif /* GENERATED_TYPES_H */\n");

        Ok(output)
    }
}

impl CMicroGenerator {
    fn emit_forward_decls(output: &mut String, m: &Module) {
        for def in &m.definitions {
            match def {
                Definition::Struct(s) => {
                    Self::push_fmt(
                        output,
                        format_args!(
                            "typedef struct {}_{} {}_{};\n",
                            m.name, s.name, m.name, s.name
                        ),
                    );
                }
                Definition::Enum(e) => {
                    Self::push_fmt(
                        output,
                        format_args!(
                            "typedef enum {}_{} {}_{};\n",
                            m.name, e.name, m.name, e.name
                        ),
                    );
                }
                Definition::Module(inner) => {
                    Self::emit_forward_decls(output, inner);
                }
                _ => {}
            }
        }
    }

    fn generate_module(&self, m: &Module) -> String {
        let mut output = String::new();
        Self::push_fmt(&mut output, format_args!("/* Module: {} */\n\n", m.name));

        for def in &m.definitions {
            match def {
                Definition::Struct(s) => {
                    // Prefix struct name with module name
                    let mut prefixed = s.clone();
                    prefixed.name = format!("{}_{}", m.name, s.name);
                    output.push_str(&self.generate_struct(&prefixed));
                }
                Definition::Enum(e) => {
                    let mut prefixed = e.clone();
                    prefixed.name = format!("{}_{}", m.name, e.name);
                    output.push_str(&Self::generate_enum(&prefixed));
                }
                Definition::Union(u) => {
                    let mut prefixed = u.clone();
                    prefixed.name = format!("{}_{}", m.name, u.name);
                    output.push_str(&self.generate_union(&prefixed));
                }
                Definition::Module(inner) => {
                    output.push_str(&self.generate_module(inner));
                }
                _ => {}
            }
        }

        output
    }

    fn generate_enum(e: &Enum) -> String {
        let mut output = String::new();
        let func_prefix = Self::to_c_ident(&e.name);

        // Enum definition
        Self::push_fmt(&mut output, format_args!("/* Enum: {} */\n", e.name));
        Self::push_fmt(&mut output, format_args!("enum {} {{\n", e.name));

        for (i, variant) in e.variants.iter().enumerate() {
            // @audit-ok: safe cast - enum variant index always << i64::MAX
            #[allow(clippy::cast_possible_wrap)]
            let val = variant.value.unwrap_or(i as i64);
            Self::push_fmt(
                &mut output,
                format_args!(
                    "    {}_{} = {},\n",
                    e.name.to_uppercase(),
                    variant.name.to_uppercase(),
                    val
                ),
            );
        }

        output.push_str("};\n\n");

        // Encode function
        Self::push_fmt(
            &mut output,
            format_args!(
                "static inline int32_t {}_encode(const {}* self, hdds_cdr_t* cdr) {{\n",
                func_prefix, e.name
            ),
        );
        output.push_str("    return hdds_cdr_write_u32(cdr, (uint32_t)*self);\n");
        output.push_str("}\n\n");

        // Decode function
        Self::push_fmt(
            &mut output,
            format_args!(
                "static inline int32_t {}_decode({}* self, hdds_cdr_t* cdr) {{\n",
                func_prefix, e.name
            ),
        );
        output.push_str("    uint32_t v;\n");
        output.push_str("    int32_t rc = hdds_cdr_read_u32(cdr, &v);\n");
        output.push_str("    if (rc != HDDS_CDR_OK) { return rc; }\n");

        // Validate enum value
        output.push_str("    switch (v) {\n");
        for (i, variant) in e.variants.iter().enumerate() {
            // @audit-ok: safe cast - enum variant index always << i64::MAX
            #[allow(clippy::cast_possible_wrap)]
            let val = variant.value.unwrap_or(i as i64);
            Self::push_fmt(&mut output, format_args!("        case {}: break;\n", val));
        }
        output.push_str("        default: return HDDS_CDR_ERR_INVALID;\n");
        output.push_str("    }\n");

        Self::push_fmt(&mut output, format_args!("    *self = ({})v;\n", e.name));
        output.push_str("    return HDDS_CDR_OK;\n");
        output.push_str("}\n\n");

        output
    }

    fn generate_struct(&self, s: &Struct) -> String {
        let mut output = String::new();
        let func_prefix = Self::to_c_ident(&s.name);

        // Struct definition
        Self::push_fmt(&mut output, format_args!("/* Struct: {} */\n", s.name));
        Self::push_fmt(&mut output, format_args!("struct {} {{\n", s.name));

        for field in &s.fields {
            let escaped = c_ident(&field.name);
            // Handle bounded strings first (string<N>)
            if let Some(bound) = Self::is_bounded_string(&field.field_type) {
                Self::push_fmt(
                    &mut output,
                    format_args!("    char {}[{}];\n", escaped, bound),
                );
            }
            // Handle bounded wstrings (wstring<N>)
            else if let Some(bound) = Self::is_bounded_wstring(&field.field_type) {
                Self::push_fmt(
                    &mut output,
                    format_args!("    uint16_t {}[{}];\n", escaped, bound),
                );
            }
            // Handle arrays specially (type goes after name)
            else if let IdlType::Array { inner, size } = &field.field_type {
                let inner_c = self.type_to_c(inner);
                Self::push_fmt(
                    &mut output,
                    format_args!("    {} {}[{}];\n", inner_c, escaped, size),
                );
            } else if let IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString) =
                &field.field_type
            {
                // Unbounded string field with default buffer size
                let bound = self.config.max_string_len;
                Self::push_fmt(
                    &mut output,
                    format_args!("    char {}[{}];\n", escaped, bound),
                );
            } else if let IdlType::Sequence { inner, bound } = &field.field_type {
                // Inline sequence struct
                let inner_c = self.type_to_c(inner);
                // @audit-ok: safe cast - config values are bounded by design (default: 32)
                #[allow(clippy::cast_possible_truncation)]
                let cap = bound.unwrap_or(self.config.max_sequence_len as u32);
                Self::push_fmt(
                    &mut output,
                    format_args!(
                        "    struct {{ {} data[{}]; uint32_t count; }} {};\n",
                        inner_c, cap, escaped
                    ),
                );
            } else {
                let c_type = self.type_to_c(&field.field_type);
                Self::push_fmt(&mut output, format_args!("    {} {};\n", c_type, escaped));
            }
        }
        output.push_str("};\n\n");

        // Calculate max encoded size
        let max_size = self.calculate_max_size(s);
        Self::push_fmt(
            &mut output,
            format_args!(
                "#define {}_ENCODED_SIZE_MAX {}\n\n",
                s.name.to_uppercase(),
                max_size
            ),
        );

        // Encode function
        output.push_str(&self.generate_struct_encode(s, &func_prefix));

        // Decode function
        output.push_str(&self.generate_struct_decode(s, &func_prefix));

        // Compute key function
        output.push_str(&Self::generate_compute_key(s, &func_prefix));

        output
    }

    fn generate_struct_encode(&self, s: &Struct, func_prefix: &str) -> String {
        let mut output = String::new();

        Self::push_fmt(
            &mut output,
            format_args!(
                "static inline int32_t {}_encode(const {}* self, hdds_cdr_t* cdr) {{\n",
                func_prefix, s.name
            ),
        );
        output.push_str("    int32_t rc;\n");
        output.push_str("    if (self == NULL || cdr == NULL) { return HDDS_CDR_ERR_NULL; }\n");

        for field in &s.fields {
            let escaped = c_ident(&field.name);
            output.push_str(&self.generate_field_encode(&escaped, &field.field_type, "self->"));
        }

        output.push_str("    return HDDS_CDR_OK;\n");
        output.push_str("}\n\n");

        output
    }

    #[allow(clippy::too_many_lines)] // Code generation for many IDL type variants
    #[allow(clippy::branches_sharing_code)] // Each branch is self-contained for clarity
    fn generate_field_encode(&self, name: &str, t: &IdlType, prefix: &str) -> String {
        let mut output = String::new();

        // Handle bounded string (string<N>) - encoded as CDR string
        if Self::is_bounded_string(t).is_some() || Self::is_bounded_wstring(t).is_some() {
            Self::push_fmt(
                &mut output,
                format_args!("    rc = hdds_cdr_write_string(cdr, {}{});\n", prefix, name),
            );
            output.push_str("    if (rc != HDDS_CDR_OK) { return rc; }\n");
            return output;
        }

        match t {
            IdlType::Primitive(p) => {
                if matches!(p, PrimitiveType::String | PrimitiveType::WString) {
                    Self::push_fmt(
                        &mut output,
                        format_args!("    rc = hdds_cdr_write_string(cdr, {}{});\n", prefix, name),
                    );
                    output.push_str("    if (rc != HDDS_CDR_OK) { return rc; }\n");
                } else {
                    let write_fn = Self::primitive_write_call(p);
                    Self::push_fmt(
                        &mut output,
                        format_args!("    rc = {}(cdr, {}{});\n", write_fn, prefix, name),
                    );
                    output.push_str("    if (rc != HDDS_CDR_OK) { return rc; }\n");
                }
            }
            IdlType::Named(type_name) => {
                let type_prefix = Self::to_c_ident(type_name);
                Self::push_fmt(
                    &mut output,
                    format_args!(
                        "    rc = {}_encode(&{}{}, cdr);\n",
                        type_prefix, prefix, name
                    ),
                );
                output.push_str("    if (rc != HDDS_CDR_OK) { return rc; }\n");
            }
            IdlType::Sequence { inner, bound } => {
                // @audit-ok: safe cast - config values are bounded by design (default: 32)
                #[allow(clippy::cast_possible_truncation)]
                let cap = bound.unwrap_or(self.config.max_sequence_len as u32);
                Self::push_fmt(
                    &mut output,
                    format_args!(
                        "    rc = hdds_cdr_write_seq_len(cdr, {}{}.count);\n",
                        prefix, name
                    ),
                );
                output.push_str("    if (rc != HDDS_CDR_OK) { return rc; }\n");

                Self::push_fmt(
                    &mut output,
                    format_args!(
                        "    for (uint32_t i = 0; i < {}{}.count && i < {}u; ++i) {{\n",
                        prefix, name, cap
                    ),
                );

                match &**inner {
                    IdlType::Primitive(p) => {
                        let write_fn = Self::primitive_write_call(p);
                        Self::push_fmt(
                            &mut output,
                            format_args!(
                                "        rc = {}(cdr, {}{}.data[i]);\n",
                                write_fn, prefix, name
                            ),
                        );
                    }
                    IdlType::Named(type_name) => {
                        let type_prefix = Self::to_c_ident(type_name);
                        Self::push_fmt(
                            &mut output,
                            format_args!(
                                "        rc = {}_encode(&{}{}.data[i], cdr);\n",
                                type_prefix, prefix, name
                            ),
                        );
                    }
                    _ => {}
                }
                output.push_str("        if (rc != HDDS_CDR_OK) { return rc; }\n");
                output.push_str("    }\n");
            }
            IdlType::Array { inner, size } => {
                Self::push_fmt(
                    &mut output,
                    format_args!("    for (uint32_t i = 0; i < {}u; ++i) {{\n", size),
                );

                match &**inner {
                    IdlType::Primitive(p) => {
                        let write_fn = Self::primitive_write_call(p);
                        Self::push_fmt(
                            &mut output,
                            format_args!(
                                "        rc = {}(cdr, {}{}[i]);\n",
                                write_fn, prefix, name
                            ),
                        );
                    }
                    IdlType::Named(type_name) => {
                        let type_prefix = Self::to_c_ident(type_name);
                        Self::push_fmt(
                            &mut output,
                            format_args!(
                                "        rc = {}_encode(&{}{}[i], cdr);\n",
                                type_prefix, prefix, name
                            ),
                        );
                    }
                    _ => {}
                }
                output.push_str("        if (rc != HDDS_CDR_OK) { return rc; }\n");
                output.push_str("    }\n");
            }
            IdlType::Map { .. } => {
                output.push_str("    /* Map encoding not supported */\n");
            }
        }

        output
    }

    fn generate_struct_decode(&self, s: &Struct, func_prefix: &str) -> String {
        let mut output = String::new();

        Self::push_fmt(
            &mut output,
            format_args!(
                "static inline int32_t {}_decode({}* self, hdds_cdr_t* cdr) {{\n",
                func_prefix, s.name
            ),
        );
        output.push_str("    int32_t rc;\n");
        output.push_str("    if (self == NULL || cdr == NULL) { return HDDS_CDR_ERR_NULL; }\n");

        for field in &s.fields {
            let escaped = c_ident(&field.name);
            output.push_str(&self.generate_field_decode(&escaped, &field.field_type, "self->"));
        }

        output.push_str("    return HDDS_CDR_OK;\n");
        output.push_str("}\n\n");

        output
    }

    #[allow(clippy::too_many_lines)] // Code generation for many IDL type variants
    #[allow(clippy::branches_sharing_code)] // Each branch is self-contained for clarity
    fn generate_field_decode(&self, name: &str, t: &IdlType, prefix: &str) -> String {
        let mut output = String::new();

        // Handle bounded string (string<N>) - decoded as CDR string
        if let Some(bound) = Self::is_bounded_string(t) {
            Self::push_fmt(
                &mut output,
                format_args!(
                    "    rc = hdds_cdr_read_string(cdr, {}{}, {});\n",
                    prefix, name, bound
                ),
            );
            output.push_str("    if (rc != HDDS_CDR_OK) { return rc; }\n");
            return output;
        }
        // Handle bounded wstring (wstring<N>) - decoded as CDR string
        if let Some(bound) = Self::is_bounded_wstring(t) {
            // For wstring we'd need a different function, for now treat as regular string
            Self::push_fmt(
                &mut output,
                format_args!(
                    "    rc = hdds_cdr_read_string(cdr, (char*){}{}, {});\n",
                    prefix,
                    name,
                    bound * 2
                ),
            );
            output.push_str("    if (rc != HDDS_CDR_OK) { return rc; }\n");
            return output;
        }

        match t {
            IdlType::Primitive(p) => {
                if matches!(p, PrimitiveType::String | PrimitiveType::WString) {
                    let bound = self.config.max_string_len;
                    Self::push_fmt(
                        &mut output,
                        format_args!(
                            "    rc = hdds_cdr_read_string(cdr, {}{}, {});\n",
                            prefix, name, bound
                        ),
                    );
                    output.push_str("    if (rc != HDDS_CDR_OK) { return rc; }\n");
                } else {
                    let read_fn = Self::primitive_read_call(p);
                    Self::push_fmt(
                        &mut output,
                        format_args!("    rc = {}(cdr, &{}{});\n", read_fn, prefix, name),
                    );
                    output.push_str("    if (rc != HDDS_CDR_OK) { return rc; }\n");
                }
            }
            IdlType::Named(type_name) => {
                let type_prefix = Self::to_c_ident(type_name);
                Self::push_fmt(
                    &mut output,
                    format_args!(
                        "    rc = {}_decode(&{}{}, cdr);\n",
                        type_prefix, prefix, name
                    ),
                );
                output.push_str("    if (rc != HDDS_CDR_OK) { return rc; }\n");
            }
            IdlType::Sequence { inner, bound } => {
                // @audit-ok: safe cast - config values are bounded by design (default: 32)
                #[allow(clippy::cast_possible_truncation)]
                let cap = bound.unwrap_or(self.config.max_sequence_len as u32);
                Self::push_fmt(
                    &mut output,
                    format_args!(
                        "    rc = hdds_cdr_read_seq_len(cdr, &{}{}.count);\n",
                        prefix, name
                    ),
                );
                output.push_str("    if (rc != HDDS_CDR_OK) { return rc; }\n");
                Self::push_fmt(
                    &mut output,
                    format_args!(
                        "    if ({}{}.count > {}u) {{ return HDDS_CDR_ERR_OVERFLOW; }}\n",
                        prefix, name, cap
                    ),
                );

                Self::push_fmt(
                    &mut output,
                    format_args!(
                        "    for (uint32_t i = 0; i < {}{}.count; ++i) {{\n",
                        prefix, name
                    ),
                );

                match &**inner {
                    IdlType::Primitive(p) => {
                        let read_fn = Self::primitive_read_call(p);
                        Self::push_fmt(
                            &mut output,
                            format_args!(
                                "        rc = {}(cdr, &{}{}.data[i]);\n",
                                read_fn, prefix, name
                            ),
                        );
                    }
                    IdlType::Named(type_name) => {
                        let type_prefix = Self::to_c_ident(type_name);
                        Self::push_fmt(
                            &mut output,
                            format_args!(
                                "        rc = {}_decode(&{}{}.data[i], cdr);\n",
                                type_prefix, prefix, name
                            ),
                        );
                    }
                    _ => {}
                }
                output.push_str("        if (rc != HDDS_CDR_OK) { return rc; }\n");
                output.push_str("    }\n");
            }
            IdlType::Array { inner, size } => {
                Self::push_fmt(
                    &mut output,
                    format_args!("    for (uint32_t i = 0; i < {}u; ++i) {{\n", size),
                );

                match &**inner {
                    IdlType::Primitive(p) => {
                        let read_fn = Self::primitive_read_call(p);
                        Self::push_fmt(
                            &mut output,
                            format_args!(
                                "        rc = {}(cdr, &{}{}[i]);\n",
                                read_fn, prefix, name
                            ),
                        );
                    }
                    IdlType::Named(type_name) => {
                        let type_prefix = Self::to_c_ident(type_name);
                        Self::push_fmt(
                            &mut output,
                            format_args!(
                                "        rc = {}_decode(&{}{}[i], cdr);\n",
                                type_prefix, prefix, name
                            ),
                        );
                    }
                    _ => {}
                }
                output.push_str("        if (rc != HDDS_CDR_OK) { return rc; }\n");
                output.push_str("    }\n");
            }
            IdlType::Map { .. } => {
                output.push_str("    /* Map decoding not supported */\n");
            }
        }

        output
    }

    /// Generate `compute_key` function for DDS key hash computation.
    /// Uses FNV-1a hash inline (no libc dependencies for embedded).
    #[allow(clippy::branches_sharing_code)] // Code gen: each branch is self-contained
    fn generate_compute_key(s: &Struct, func_prefix: &str) -> String {
        let mut output = String::new();

        Self::push_fmt(
            &mut output,
            format_args!(
                "static inline int {}_compute_key(const {}* value, uint8_t key_hash[16]) {{\n",
                func_prefix, s.name
            ),
        );
        output.push_str("    if (value == (void*)0 || key_hash == (void*)0) { return -1; }\n");

        let key_fields: Vec<_> = s
            .fields
            .iter()
            .filter(|f| f.annotations.iter().any(|a| matches!(a, Annotation::Key)))
            .collect();

        if key_fields.is_empty() {
            output.push_str("    /* No @key fields - clear hash */\n");
            output.push_str("    {\n");
            output.push_str("        uint32_t i;\n");
            output.push_str("        for (i = 0; i < 16; ++i) { key_hash[i] = 0; }\n");
            output.push_str("    }\n");
            output.push_str("    return 0; /* No key */\n");
        } else {
            output.push_str("    /* FNV-1a hash of @key fields */\n");
            output.push_str("    uint64_t hash = 14695981039346656037ULL;\n");

            for field in &key_fields {
                let escaped = c_ident(&field.name);
                output.push_str("    {\n");

                // Check if it's a string type (bounded or unbounded)
                let is_string = Self::is_bounded_string(&field.field_type).is_some()
                    || matches!(
                        &field.field_type,
                        IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString)
                    );

                if is_string {
                    // For strings: iterate each byte until null terminator
                    Self::push_fmt(
                        &mut output,
                        format_args!("        const char* str = value->{};\n", escaped),
                    );
                    output.push_str("        while (*str) {\n");
                    output.push_str("            hash ^= (uint8_t)*str;\n");
                    output.push_str("            hash *= 1099511628211ULL;\n");
                    output.push_str("            ++str;\n");
                    output.push_str("        }\n");
                } else {
                    // For numeric types: access byte by byte via pointer cast
                    Self::push_fmt(
                        &mut output,
                        format_args!(
                            "        const uint8_t* ptr = (const uint8_t*)&value->{};\n",
                            escaped
                        ),
                    );
                    let size = Self::get_primitive_size(&field.field_type);
                    output.push_str("        uint32_t i;\n");
                    Self::push_fmt(
                        &mut output,
                        format_args!("        for (i = 0; i < {}; ++i) {{\n", size),
                    );
                    output.push_str("            hash ^= ptr[i];\n");
                    output.push_str("            hash *= 1099511628211ULL;\n");
                    output.push_str("        }\n");
                }

                output.push_str("    }\n");
            }

            // Expand 8-byte hash to 16 bytes (no memcpy for embedded)
            output.push_str("    /* Expand to 16 bytes */\n");
            output.push_str("    {\n");
            output.push_str("        uint32_t i;\n");
            output.push_str("        const uint8_t* src = (const uint8_t*)&hash;\n");
            output.push_str("        for (i = 0; i < 8; ++i) { key_hash[i] = src[i]; }\n");
            output.push_str("        hash *= 1099511628211ULL;\n");
            output.push_str("        for (i = 0; i < 8; ++i) { key_hash[8 + i] = src[i]; }\n");
            output.push_str("    }\n");
            output.push_str("    return 1; /* Has key */\n");
        }

        output.push_str("}\n\n");
        output
    }

    #[allow(clippy::too_many_lines)] // Code generation for union encode/decode/max_size
    fn generate_union(&self, u: &Union) -> String {
        let mut output = String::new();
        let func_prefix = Self::to_c_ident(&u.name);

        // Discriminator type
        let disc_c_type = self.type_to_c(&u.discriminator);

        // Union definition (as struct with discriminator + union)
        Self::push_fmt(&mut output, format_args!("/* Union: {} */\n", u.name));
        Self::push_fmt(&mut output, format_args!("struct {} {{\n", u.name));
        Self::push_fmt(&mut output, format_args!("    {} _d;\n", disc_c_type));
        output.push_str("    union {\n");

        for case in &u.cases {
            let escaped = c_ident(&case.field.name);
            let c_type = self.type_to_c(&case.field.field_type);
            if let IdlType::Array { inner, size } = &case.field.field_type {
                let inner_c = self.type_to_c(inner);
                Self::push_fmt(
                    &mut output,
                    format_args!("        {} {}[{}];\n", inner_c, escaped, size),
                );
            } else {
                Self::push_fmt(
                    &mut output,
                    format_args!("        {} {};\n", c_type, escaped),
                );
            }
        }

        output.push_str("    } _u;\n");
        output.push_str("};\n\n");

        // Encode function
        Self::push_fmt(
            &mut output,
            format_args!(
                "static inline int32_t {}_encode(const {}* self, hdds_cdr_t* cdr) {{\n",
                func_prefix, u.name
            ),
        );
        output.push_str("    int32_t rc;\n");
        output.push_str("    if (self == NULL || cdr == NULL) { return HDDS_CDR_ERR_NULL; }\n");

        // Encode discriminator
        if let IdlType::Named(disc_name) = &u.discriminator {
            let disc_prefix = Self::to_c_ident(disc_name);
            Self::push_fmt(
                &mut output,
                format_args!("    rc = {}_encode(&self->_d, cdr);\n", disc_prefix),
            );
        } else {
            output.push_str("    rc = hdds_cdr_write_u32(cdr, (uint32_t)self->_d);\n");
        }
        output.push_str("    if (rc != HDDS_CDR_OK) { return rc; }\n");

        // Switch on discriminator
        output.push_str("    switch (self->_d) {\n");
        for case in &u.cases {
            for label in &case.labels {
                match label {
                    UnionLabel::Value(v) => {
                        if let IdlType::Named(enum_name) = &u.discriminator {
                            Self::push_fmt(
                                &mut output,
                                format_args!(
                                    "    case {}_{}: {{\n",
                                    enum_name.to_uppercase(),
                                    v.to_uppercase()
                                ),
                            );
                        } else {
                            Self::push_fmt(&mut output, format_args!("    case {}: {{\n", v));
                        }
                    }
                    UnionLabel::Default => {
                        output.push_str("    default: {\n");
                    }
                }
            }

            // Encode the field
            let escaped = c_ident(&case.field.name);
            output.push_str(&self.generate_field_encode(
                &escaped,
                &case.field.field_type,
                "self->_u.",
            ));
            output.push_str("        break;\n");
            output.push_str("    }\n");
        }
        output.push_str("    }\n");
        output.push_str("    return HDDS_CDR_OK;\n");
        output.push_str("}\n\n");

        // Decode function
        Self::push_fmt(
            &mut output,
            format_args!(
                "static inline int32_t {}_decode({}* self, hdds_cdr_t* cdr) {{\n",
                func_prefix, u.name
            ),
        );
        output.push_str("    int32_t rc;\n");
        output.push_str("    if (self == NULL || cdr == NULL) { return HDDS_CDR_ERR_NULL; }\n");

        // Decode discriminator
        if let IdlType::Named(disc_name) = &u.discriminator {
            let disc_prefix = Self::to_c_ident(disc_name);
            Self::push_fmt(
                &mut output,
                format_args!("    rc = {}_decode(&self->_d, cdr);\n", disc_prefix),
            );
        } else {
            output.push_str("    uint32_t disc_val;\n");
            output.push_str("    rc = hdds_cdr_read_u32(cdr, &disc_val);\n");
            Self::push_fmt(
                &mut output,
                format_args!("    self->_d = ({})disc_val;\n", disc_c_type),
            );
        }
        output.push_str("    if (rc != HDDS_CDR_OK) { return rc; }\n");

        // Switch on discriminator
        output.push_str("    switch (self->_d) {\n");
        for case in &u.cases {
            for label in &case.labels {
                match label {
                    UnionLabel::Value(v) => {
                        if let IdlType::Named(enum_name) = &u.discriminator {
                            Self::push_fmt(
                                &mut output,
                                format_args!(
                                    "    case {}_{}: {{\n",
                                    enum_name.to_uppercase(),
                                    v.to_uppercase()
                                ),
                            );
                        } else {
                            Self::push_fmt(&mut output, format_args!("    case {}: {{\n", v));
                        }
                    }
                    UnionLabel::Default => {
                        output.push_str("    default: {\n");
                    }
                }
            }

            // Decode the field
            let escaped = c_ident(&case.field.name);
            output.push_str(&self.generate_field_decode(
                &escaped,
                &case.field.field_type,
                "self->_u.",
            ));
            output.push_str("        break;\n");
            output.push_str("    }\n");
        }
        output.push_str("    }\n");
        output.push_str("    return HDDS_CDR_OK;\n");
        output.push_str("}\n\n");

        output
    }

    fn calculate_max_size(&self, s: &Struct) -> usize {
        let mut size = 0usize;
        for field in &s.fields {
            size += self.type_max_size(&field.field_type);
            // Add padding (worst case: 8-byte alignment)
            size = (size + 7) & !7;
        }
        size
    }

    fn type_max_size(&self, t: &IdlType) -> usize {
        match t {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Boolean
                | PrimitiveType::Char
                | PrimitiveType::Octet
                | PrimitiveType::UInt8
                | PrimitiveType::Int8 => 1,
                PrimitiveType::UInt16
                | PrimitiveType::Int16
                | PrimitiveType::Short
                | PrimitiveType::UnsignedShort
                | PrimitiveType::WChar => 2,
                PrimitiveType::UInt32
                | PrimitiveType::Int32
                | PrimitiveType::Long
                | PrimitiveType::UnsignedLong
                | PrimitiveType::Float => 4,
                PrimitiveType::String | PrimitiveType::WString => {
                    4 + self.config.max_string_len + 1 // length + chars + null
                }
                // 8-byte primitives (UInt64, Int64, Double, etc.) and others default to 8
                _ => 8,
            },
            IdlType::Named(_) => 64, // Estimate for nested types
            IdlType::Sequence { inner, bound } => {
                // @audit-ok: safe cast - config values are bounded by design (default: 32)
                #[allow(clippy::cast_possible_truncation)]
                let cap = bound.unwrap_or(self.config.max_sequence_len as u32) as usize;
                4 + cap * self.type_max_size(inner)
            }
            IdlType::Array { inner, size } => *size as usize * self.type_max_size(inner),
            IdlType::Map { .. } => 0,
        }
    }
}
