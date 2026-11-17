// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Micro code generator for `no_std` embedded targets
//!
//! Generates Rust code compatible with `hdds_micro` crate.
//! Uses inline `CdrEncoder`/`CdrDecoder` instead of traits.
//!
//! # Wire format
//!
//! XCDR1 (`PLAIN_CDR`, DDS-`XTypes` 1.3 7.4.1): `hdds_micro`'s cursors align
//! every primitive to its size, 8-byte values at 8, from the first byte of
//! their buffer. `encode` writes the body alone; the caller puts the
//! encapsulation header (`CDR_LE`, 0x0001) in front of it, outside the
//! encoder, so that the alignment origin stays the first byte of the body.
//! An `@appendable` type is serialized as FINAL (rule 29).
//!
//! # Supported IDL Constructs
//!
//! | Construct | Notes |
//! |-----------|-------|
//! | `struct` | Base members first (`base` field); APPENDABLE as FINAL; MUTABLE as a parameter list (rules 23-25) |
//! | `@optional` | `Option<T>`; a parameter (rule 19), or left out of a MUTABLE list when absent |
//! | `enum` | Serialized at its `@bit_bound` width (1, 2 or 4 bytes) |
//! | `union` | Discriminator at its type's width; MUTABLE as a parameter list (rule 28) |
//! | `module` | Mapped to Rust modules |
//! | `typedef` | Resolved to the type it names (no alias is emitted) |
//! | `sequence<T, N>` | `heapless::Vec<T, N>` (unbounded: `max_sequence_len`) |
//! | `string<N>` | `heapless::String<N>` (unbounded: `max_string_len`) |
//! | `wchar`, `wstring` | `u32` (one UTF-16 unit, 2 bytes); `heapless::String` (`wstring<N>`: `heapless::Vec<u32, N>` of code points), UTF-16 on the wire |
//! | `@key` | `compute_key()`: RTPS 2.5 9.6.4.8 `KeyHash` |
//!
//! # Refused Constructs (generation fails, naming the type)
//!
//! | Construct | Reason |
//! |-----------|--------|
//! | `map<K, V>` | Requires dynamic allocation |
//! | `bitset`, `bitmask`, `fixed` | Not mapped |
//!
//! `const` declarations are not emitted.
//!
//! Note: uninlined_format_args allowed here due to extensive `format!()` usage
//! in code generation that would require significant refactoring.

#![allow(clippy::uninlined_format_args)]
#![allow(clippy::format_push_string)]

use crate::ast::{Definition, Enum, IdlFile, Struct, Union};
use crate::codegen::CodeGenerator;
use crate::error::Result;
use crate::error::{ErrorKind, ParseError, Position};
use crate::types::{IdlType, PrimitiveType};

use super::keywords::rust_ident;

mod codec;
mod key_hash;

/// Configuration for micro code generation
#[derive(Debug, Clone)]
pub struct MicroConfig {
    /// Maximum length for `heapless::String<N>`
    pub max_string_len: usize,
    /// Maximum length for `heapless::Vec<T, N>`
    pub max_sequence_len: usize,
    /// Stack buffer size for encode/decode
    pub stack_buffer_size: usize,
}

impl Default for MicroConfig {
    fn default() -> Self {
        Self {
            max_string_len: 64,
            max_sequence_len: 32,
            stack_buffer_size: 256,
        }
    }
}

/// Generates `no_std` Rust code for `hdds_micro`
pub struct MicroGenerator {
    config: MicroConfig,
}

impl MicroGenerator {
    /// Creates a new micro generator with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: MicroConfig::default(),
        }
    }

    /// Creates a new micro generator with custom configuration.
    #[must_use]
    pub const fn with_config(config: MicroConfig) -> Self {
        Self { config }
    }

    fn push_fmt(output: &mut String, args: std::fmt::Arguments<'_>) {
        use std::fmt::Write;
        let _ = write!(output, "{}", args);
    }
}

impl Default for MicroGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl MicroGenerator {
    /// Check if the AST uses types that require heapless (strings or sequences)
    fn ast_uses_heapless(ast: &IdlFile) -> bool {
        fn check_type(t: &IdlType) -> bool {
            match t {
                IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString)
                | IdlType::BoundedString { .. }
                | IdlType::Sequence { .. } => true,
                IdlType::Array { inner, .. } => check_type(inner),
                _ => false,
            }
        }

        fn check_def(def: &Definition) -> bool {
            match def {
                Definition::Struct(s) => s.fields.iter().any(|f| check_type(&f.field_type)),
                Definition::Module(m) => m.definitions.iter().any(check_def),
                _ => false,
            }
        }

        ast.definitions.iter().any(check_def)
    }
}
impl CodeGenerator for MicroGenerator {
    fn generate(&self, ast: &IdlFile) -> Result<String> {
        // References whose spelling a scope-less lookup would misread are
        // written as their absolute global name (IDL 4.2 7.5), and module
        // `@autoid` is carried by each type.
        let ast = &crate::scope::merge_reopened_modules(&crate::scope::for_codegen(ast));
        let mut output = String::new();

        // Check if we need heapless (strings or sequences)
        let needs_heapless = Self::ast_uses_heapless(ast);

        // Header
        output.push_str("// Generated by hddsgen gen micro\n");
        output.push_str("// DO NOT EDIT\n\n");
        output.push_str("#![allow(dead_code, unused_imports)]\n\n");

        // Use hdds-micro crate for CDR (no inline code)
        output.push_str("// Re-export CDR types from hdds-micro\n");
        output.push_str("pub use hdds_micro::cdr::{CdrEncoder, CdrDecoder};\n");
        output.push_str("pub use hdds_micro::Error as CdrError;\n");
        output.push_str(&crate::codegen::rust_paths::root_module());

        if needs_heapless {
            output.push_str("pub use heapless;\n");
        }
        output.push('\n');

        let index = super::key_holder::TypeIndex::new(ast);
        if key_hash::file_needs_writer(ast, &index) {
            output.push_str(&key_hash::writer_module());
        }

        self.generate_definitions(&mut output, &ast.definitions, &index, "")?;
        Ok(output)
    }
}

impl MicroGenerator {
    fn generate_definitions(
        &self,
        output: &mut String,
        defs: &[Definition],
        index: &super::key_holder::TypeIndex,
        indent: &str,
    ) -> Result<()> {
        for def in defs {
            let code = match def {
                Definition::Module(m) => {
                    Self::push_fmt(output, format_args!("{indent}pub mod {} {{\n", m.name));
                    Self::push_fmt(output, format_args!("{indent}    use super::*;\n\n"));
                    self.generate_definitions(
                        output,
                        &m.definitions,
                        index,
                        &format!("{indent}    "),
                    )?;
                    Self::push_fmt(output, format_args!("{indent}}}\n\n"));
                    continue;
                }
                Definition::Struct(s) => self.generate_struct(s, index)?,
                Definition::Enum(e) => Self::generate_enum(e),
                Definition::Union(u) => self.generate_union(u, index)?,
                _ => continue,
            };
            for line in code.lines() {
                if line.is_empty() {
                    output.push('\n');
                } else {
                    Self::push_fmt(output, format_args!("{indent}{line}\n"));
                }
            }
        }
        Ok(())
    }

    fn refuse(reason: &str) -> ParseError {
        ParseError::new(
            ErrorKind::Other,
            Position::new(0, 0),
            format!("micro backend: {reason}"),
        )
    }

    /// Generate struct definition and encode/decode methods
    fn generate_struct(&self, s: &Struct, index: &super::key_holder::TypeIndex) -> Result<String> {
        if let Some(reason) = codec::struct_refusal(index, &self.config, s) {
            return Err(Self::refuse(&reason));
        }
        let g = codec::MGen::new(index, &self.config);
        let mut output = String::new();

        Self::push_fmt(
            &mut output,
            format_args!("#[derive(Debug, Clone, PartialEq)]\n"),
        );
        Self::push_fmt(&mut output, format_args!("pub struct {} {{\n", s.name));
        if let Some(base) = s
            .base_struct
            .as_deref()
            .map(crate::codegen::rust_paths::path)
        {
            Self::push_fmt(&mut output, format_args!("    pub base: {base},\n"));
        }
        for field in &s.fields {
            let escaped = rust_ident(&field.name);
            let rust_type = if field.is_optional() {
                format!("Option<{}>", g.rust_type(&field.field_type))
            } else {
                g.rust_type(&field.field_type)
            };
            Self::push_fmt(
                &mut output,
                format_args!("    pub {}: {},\n", escaped, rust_type),
            );
        }
        output.push_str("}\n\n");

        // Largest serialization (XCDR1 alignment, every bound at its maximum).
        let max_size = g
            .struct_max_end(s, 0, 0)
            .ok_or_else(|| Self::refuse(&format!("struct `{}` reaches itself", s.name)))?;
        Self::push_fmt(
            &mut output,
            format_args!(
                "impl {} {{\n    pub const ENCODED_SIZE_MAX: usize = {};\n\n",
                s.name, max_size
            ),
        );
        output.push_str(&codec::struct_methods(index, &self.config, s));
        output.push('\n');

        // Generate compute_key method (instance KeyHash, RTPS 2.5 9.6.4.8)
        output.push_str(&key_hash::compute_key_method(s, index));

        output.push_str("}\n\n");
        Ok(output)
    }

    /// Generate enum definition
    fn generate_enum(e: &Enum) -> String {
        let mut output = String::new();

        Self::push_fmt(
            &mut output,
            format_args!("#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]\n"),
        );
        let values = e.resolved_values();
        // A negative discriminant needs a signed representation.
        let repr = if values.iter().any(|v| *v < 0) {
            "i32"
        } else {
            "u32"
        };
        Self::push_fmt(&mut output, format_args!("#[repr({repr})]\n"));
        Self::push_fmt(&mut output, format_args!("pub enum {} {{\n", e.name));

        for (i, (variant, val)) in e.variants.iter().zip(values).enumerate() {
            // First variant gets #[default] attribute
            let default_attr = if i == 0 { "    #[default]\n" } else { "" };
            Self::push_fmt(
                &mut output,
                format_args!("{}    {} = {},\n", default_attr, variant.name, val),
            );
        }
        output.push_str("}\n\n");

        Self::push_fmt(&mut output, format_args!("impl {} {{\n", e.name));
        output.push_str(&codec::enum_methods(e));
        output.push_str("}\n\n");

        output
    }

    /// Generate union definition and encode/decode methods
    fn generate_union(&self, u: &Union, index: &super::key_holder::TypeIndex) -> Result<String> {
        if let Some(reason) = codec::union_refusal(index, &self.config, u) {
            return Err(Self::refuse(&reason));
        }
        let g = codec::MGen::new(index, &self.config);
        let mut output = String::new();

        // Union as Rust enum with tuple variants
        Self::push_fmt(
            &mut output,
            format_args!("#[derive(Debug, Clone, PartialEq)]\n"),
        );
        Self::push_fmt(&mut output, format_args!("pub enum {} {{\n", u.name));

        for (idx, case) in u.cases.iter().enumerate() {
            let variant_name = Self::union_variant_name(&case.field.name, idx);
            let field_type = g.rust_type(&case.field.field_type);
            Self::push_fmt(
                &mut output,
                format_args!("    {}({}),\n", variant_name, field_type),
            );
        }
        output.push_str("}\n\n");

        Self::push_fmt(&mut output, format_args!("impl {} {{\n", u.name));
        output.push_str(&codec::union_methods(
            index,
            &self.config,
            u,
            &Self::union_variant_name,
        ));
        output.push_str(&codec::union_default(
            index,
            &self.config,
            u,
            &Self::union_variant_name,
        ));
        output.push_str("}\n\n");
        output.push_str(&key_hash::union_method(u, index));

        Ok(output)
    }

    fn union_variant_name(field_name: &str, idx: usize) -> String {
        // Convert to CamelCase: foo_bar -> FooBar
        let mut result = String::new();
        let mut capitalize_next = true;

        for c in field_name.chars() {
            if c == '_' {
                capitalize_next = true;
            } else if capitalize_next {
                result.push(c.to_ascii_uppercase());
                capitalize_next = false;
            } else {
                result.push(c);
            }
        }

        if result.chars().next().is_none_or(|c| !c.is_alphabetic()) {
            format!("Case{idx}")
        } else {
            result
        }
    }

    /// Rust variant of a union case, for the `KeyHolder` emitter.
    pub(super) fn union_variant_name_for_key(field_name: &str, idx: usize) -> String {
        Self::union_variant_name(field_name, idx)
    }
}
