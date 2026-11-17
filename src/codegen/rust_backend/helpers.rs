// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Helper utilities for Rust code generation.
//!
//! Common functions for type mapping, formatting, and AST traversal.

#![allow(clippy::redundant_pub_crate)]

use super::RustGenerator;
use crate::ast::{Definition, IdlFile};
use crate::types::{Annotation, IdlType, PrimitiveType};
use std::fmt::Write;

/// CDR encoding version threaded through the codegen pipeline.
///
/// XCDR v1 and XCDR v2 share most of their serialization rules but diverge
/// on the alignment of 8-byte primitives (XCDR v1 aligns on 8, XCDR v2 caps
/// at 4 per OMG DDS-XTypes v1.3 Section 7.4.2). The codegen selects the
/// right alignment table plus the matching function names and inter-type
/// method invocations based on this enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CdrVersion {
    /// XCDR v1 per OMG DDS-XTypes v1.3 Section 7.4.1.
    Xcdr1,
    /// XCDR v2 per OMG DDS-XTypes v1.3 Section 7.4.2.
    Xcdr2,
}

/// XCDR versions the generator always emits for every named type (struct,
/// union, enum, bitset, Fixed).
///
/// Systematic dual emission avoids propagating a "target version" through
/// nested type invocations. Every generated type carries both
/// `encode_xcdr1_le` / `encode_xcdr2_le` inherent methods, so a caller
/// encoding in XCDR v1 context always finds the matching sub-type method
/// locally. The default wire representation (what `Cdr2Encode::encode_cdr2_le`
/// maps to) is selected per type by the `@data_representation` annotation via
/// [`primary_version`].
pub(super) const VERSIONS_TO_EMIT: &[CdrVersion] = &[CdrVersion::Xcdr1, CdrVersion::Xcdr2];

pub(crate) fn push_fmt(dst: &mut String, args: std::fmt::Arguments<'_>) {
    let _ = dst.write_fmt(args);
}

/// Read the `@data_representation("...")` annotation string from a list of
/// annotations. Returns `None` if absent.
///
/// Possible valid values per OMG `XTypes` v1.3 (checked at parse-time in
/// `validate/rules/helpers.rs:54`): `"XCDR1"`, `"XCDR2"`, `"PLAIN_CDR"`,
/// `"PLAIN_CDR2"`.
pub(super) fn data_representation_annotation(annotations: &[Annotation]) -> Option<String> {
    annotations.iter().find_map(|a| match a {
        Annotation::DataRepresentation(val) => Some(val.clone()),
        _ => None,
    })
}

/// Decide which version the legacy `Cdr2Encode` / `Cdr2Decode` trait impls
/// delegate to, given the type's `@data_representation` annotation.
///
/// Both `encode_xcdr1_le` and `encode_xcdr2_le` inherent methods are always
/// emitted (see [`VERSIONS_TO_EMIT`]); only the default wire representation
/// exposed through the `Cdr2Encode` trait shifts based on the annotation.
///
/// The match is exhaustive over the four values the parser validates at
/// `validate/rules/helpers.rs:54` (`"XCDR1"`, `"XCDR2"`, `"PLAIN_CDR"`,
/// `"PLAIN_CDR2"`). Any other string reaching this function indicates a
/// new accepted value was added to the parser without being wired here:
/// a `debug_assert!` fires in debug builds so the mismatch surfaces, and
/// release builds fall back to the spec-correct `Xcdr2` default.
///
/// - `"XCDR1"` / `"PLAIN_CDR"` -> `Xcdr1`
/// - `"XCDR2"` / `"PLAIN_CDR2"` -> `Xcdr2`
/// - `None` -> `Xcdr2` (no annotation = spec-correct default)
/// - any other string -> debug-assert + fall back to `Xcdr2`
pub(super) fn primary_version(repr: Option<&str>) -> CdrVersion {
    match repr {
        Some("XCDR1" | "PLAIN_CDR") => CdrVersion::Xcdr1,
        Some("XCDR2" | "PLAIN_CDR2") | None => CdrVersion::Xcdr2,
        Some(other) => {
            debug_assert!(
                false,
                "primary_version: unexpected @data_representation value {other:?}; \
                 parser accepts a value the codegen does not handle. \
                 Falling back to Xcdr2."
            );
            CdrVersion::Xcdr2
        }
    }
}

/// Returns the `xcdr1` / `xcdr2` fragment used in generated function names
/// such as `encode_xcdr1_le` or `max_xcdr2_size`.
pub(super) const fn xcdr_method_suffix(version: CdrVersion) -> &'static str {
    match version {
        CdrVersion::Xcdr1 => "xcdr1",
        CdrVersion::Xcdr2 => "xcdr2",
    }
}

impl RustGenerator {
    pub(super) fn type_to_rust(idl_type: &IdlType) -> String {
        match idl_type {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Void => "()".to_string(),
                PrimitiveType::Boolean => "bool".to_string(),
                PrimitiveType::Char | PrimitiveType::WChar => "char".to_string(),
                PrimitiveType::Octet | PrimitiveType::UInt8 => "u8".to_string(),
                PrimitiveType::Short | PrimitiveType::Int16 => "i16".to_string(),
                PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => "u16".to_string(),
                PrimitiveType::Long | PrimitiveType::Int32 => "i32".to_string(),
                PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => "u32".to_string(),
                PrimitiveType::LongLong | PrimitiveType::Int64 => "i64".to_string(),
                PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => "u64".to_string(),
                PrimitiveType::Float => "f32".to_string(),
                PrimitiveType::Double | PrimitiveType::LongDouble => "f64".to_string(),
                PrimitiveType::String | PrimitiveType::WString => "String".to_string(),
                PrimitiveType::Int8 => "i8".to_string(),
                PrimitiveType::Fixed { digits, scale } => format!("Fixed<{digits}, {scale}>"),
            },
            IdlType::Named(name) => crate::codegen::rust_paths::path(name),
            IdlType::BoundedString { .. } => "String".to_string(),
            IdlType::Sequence { inner, .. } => {
                let inner_type = Self::type_to_rust(inner);
                format!("Vec<{inner_type}>")
            }
            IdlType::Map { key, value, .. } => {
                let key = Self::type_to_rust(key);
                let value = Self::type_to_rust(value);
                format!("std::collections::HashMap<{key}, {value}>")
            }
            IdlType::Array { inner, size } => {
                let inner = Self::type_to_rust(inner);
                format!("[{inner}; {size}]")
            }
        }
    }

    /// Emit `impl Cdr2Encode` / `impl Cdr2Decode` for type `name`, routed
    /// to the inherent methods of the version the type's
    /// `@data_representation` selects (`primary`; XCDR2 by default).
    ///
    /// The `_at` forms go through the inherent `_at` methods, which align
    /// against the start of the buffer they are handed (the codec's
    /// origin), so a value encoded at an offset lands where the enclosing
    /// stream expects it.
    pub(super) fn emit_cdr_trait_delegator(name: &str, primary: CdrVersion) -> String {
        let suffix = super::helpers::xcdr_method_suffix(primary);
        format!(
            "impl Cdr2Encode for {name} {{\n\
             \u{20}   fn encode_cdr2_le(&self, dst: &mut [u8]) -> Result<usize, CdrError> {{\n\
             \u{20}       self.encode_{suffix}_le(dst)\n\
             \u{20}   }}\n\
             \n\
             \u{20}   fn max_cdr2_size(&self) -> usize {{\n\
             \u{20}       self.max_{suffix}_size()\n\
             \u{20}   }}\n\
             \n\
             \u{20}   fn encode_cdr2_le_at(\n\
             \u{20}       &self,\n\
             \u{20}       dst: &mut [u8],\n\
             \u{20}       offset: &mut usize,\n\
             \u{20}   ) -> Result<(), CdrError> {{\n\
             \u{20}       self.encode_{suffix}_le_at(dst, offset)\n\
             \u{20}   }}\n\
             }}\n\
             \n\
             impl Cdr2Decode for {name} {{\n\
             \u{20}   fn decode_cdr2_le(src: &[u8]) -> Result<(Self, usize), CdrError> {{\n\
             \u{20}       Self::decode_{suffix}_le(src)\n\
             \u{20}   }}\n\
             \n\
             \u{20}   fn decode_cdr2_le_at(\n\
             \u{20}       src: &[u8],\n\
             \u{20}       offset: &mut usize,\n\
             \u{20}   ) -> Result<Self, CdrError> {{\n\
             \u{20}       Self::decode_{suffix}_le_at(src, offset)\n\
             \u{20}   }}\n\
             }}\n\
             \n"
        )
    }

    /// Emit an `encode_{suffix}_le_at` that forwards to the inherent
    /// `encode_{suffix}_le` on a sub-slice (enumerations and bitsets, whose
    /// standalone encoding needs no alignment).
    pub(super) fn emit_encode_at_wrapper(suffix: &str, indent: &str) -> String {
        format!(
            "{indent}    pub fn encode_{suffix}_le_at(\n\
             {indent}        &self,\n\
             {indent}        dst: &mut [u8],\n\
             {indent}        offset: &mut usize,\n\
             {indent}    ) -> Result<(), CdrError> {{\n\
             {indent}        let len = self.encode_{suffix}_le(&mut dst[*offset..])?;\n\
             {indent}        *offset += len;\n\
             {indent}        Ok(())\n\
             {indent}    }}\n"
        )
    }

    /// Emit a `decode_{suffix}_le_at` that forwards to the inherent
    /// `decode_{suffix}_le` on a sub-slice (see `emit_encode_at_wrapper`).
    pub(super) fn emit_decode_at_wrapper(suffix: &str, indent: &str) -> String {
        format!(
            "{indent}    pub fn decode_{suffix}_le_at(\n\
             {indent}        src: &[u8],\n\
             {indent}        offset: &mut usize,\n\
             {indent}    ) -> Result<Self, CdrError> {{\n\
             {indent}        let (value, used) = Self::decode_{suffix}_le(&src[*offset..])?;\n\
             {indent}        *offset += used;\n\
             {indent}        Ok(value)\n\
             {indent}    }}\n"
        )
    }
}

pub(super) fn uses_fixed(ast: &IdlFile) -> bool {
    fn type_has_fixed(t: &IdlType) -> bool {
        match t {
            IdlType::Primitive(PrimitiveType::Fixed { .. }) => true,
            IdlType::Sequence { inner, .. } | IdlType::Array { inner, .. } => type_has_fixed(inner),
            IdlType::Map { key, value, .. } => type_has_fixed(key) || type_has_fixed(value),
            _ => false,
        }
    }

    for def in &ast.definitions {
        match def {
            Definition::Struct(s) => {
                if s.fields.iter().any(|f| type_has_fixed(&f.field_type)) {
                    return true;
                }
            }
            Definition::Typedef(t) => {
                if type_has_fixed(&t.base_type) {
                    return true;
                }
            }
            Definition::Union(u) => {
                if u.cases.iter().any(|c| type_has_fixed(&c.field.field_type)) {
                    return true;
                }
            }
            Definition::Module(m) => {
                let sub = IdlFile {
                    definitions: m.definitions.clone(),
                };
                if uses_fixed(&sub) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Convert `SCREAMING_SNAKE_CASE` or `snake_case` to `PascalCase` for Rust enum variants.
///
/// Examples:
/// - `RED` -> `Red`
/// - `GREEN_LIGHT` -> `GreenLight`
/// - `ALREADY_Pascal` -> `AlreadyPascal`
pub(super) fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                let mut result = String::new();
                result.push_str(&first.to_uppercase().to_string());
                result.push_str(&chars.as_str().to_lowercase());
                result
            })
        })
        .collect()
}
