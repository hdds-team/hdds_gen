// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Helper utilities for Rust code generation.
//!
//! Common functions for type mapping, formatting, and AST traversal.

#![allow(clippy::redundant_pub_crate)]

use super::RustGenerator;
use crate::ast::{Definition, Enum, Field, IdlFile, Struct};
use crate::types::{Annotation, AutoIdKind, IdlType, PrimitiveType};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
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

thread_local! {
    // @audit-ok: thread_local! guarantees thread-safety, RefCell is safe here
    static MUTABLE_TYPES: RefCell<HashSet<String>> = RefCell::new(HashSet::new());

    /// Index of all type definitions (structs + enums) for XTypes inline resolution.
    /// Maps unqualified name -> (definition, optional module name).
    // @audit-ok: thread_local! RefCell is inherently thread-safe (one instance per thread)
    static TYPE_DEFS: RefCell<HashMap<String, (TypeDef, Option<String>)>> =
        RefCell::new(HashMap::new());
}

/// Lightweight clone of struct/enum AST nodes for `XTypes` inline `TypeObject` generation.
#[derive(Clone)]
pub(super) enum TypeDef {
    Struct(Struct),
    Enum(Enum),
}

pub(super) fn set_mutable_types(set: HashSet<String>) {
    MUTABLE_TYPES.with(|cell| *cell.borrow_mut() = set);
}

/// Populate the type definition index from the AST.
pub(super) fn set_type_defs(ast: &IdlFile) {
    fn walk(
        defs: &[Definition],
        module: Option<&str>,
        acc: &mut HashMap<String, (TypeDef, Option<String>)>,
    ) {
        for def in defs {
            match def {
                Definition::Struct(s) => {
                    acc.insert(
                        s.name.clone(),
                        (TypeDef::Struct(s.clone()), module.map(String::from)),
                    );
                }
                Definition::Enum(e) => {
                    acc.insert(
                        e.name.clone(),
                        (TypeDef::Enum(e.clone()), module.map(String::from)),
                    );
                }
                Definition::Module(m) => walk(&m.definitions, Some(&m.name), acc),
                _ => {}
            }
        }
    }
    let mut map = HashMap::new();
    walk(&ast.definitions, None, &mut map);
    TYPE_DEFS.with(|cell| *cell.borrow_mut() = map);
}

/// Look up a type definition by unqualified name.
pub(super) fn lookup_type_def(name: &str) -> Option<(TypeDef, Option<String>)> {
    TYPE_DEFS.with(|cell| cell.borrow().get(name).cloned())
}

pub(super) fn is_named_mutable(name: &str) -> bool {
    MUTABLE_TYPES.with(|cell| cell.borrow().contains(name))
}

pub(super) fn collect_mutable_types(ast: &IdlFile) -> HashSet<String> {
    fn walk(defs: &[Definition], acc: &mut HashSet<String>) {
        for def in defs {
            match def {
                Definition::Struct(s) => {
                    let is_mut = matches!(
                        s.extensibility,
                        Some(
                            crate::types::ExtensibilityKind::Mutable
                                | crate::types::ExtensibilityKind::Appendable
                        )
                    ) || s.annotations.iter().any(|a| {
                        matches!(
                            a,
                            Annotation::Extensibility(
                                crate::types::ExtensibilityKind::Mutable
                                    | crate::types::ExtensibilityKind::Appendable
                            ) | Annotation::Mutable
                                | Annotation::Appendable
                        )
                    });
                    if is_mut {
                        acc.insert(s.name.clone());
                    }
                }
                Definition::Module(m) => walk(&m.definitions, acc),
                _ => {}
            }
        }
    }

    let mut set = HashSet::new();
    walk(&ast.definitions, &mut set);
    set
}

/// Read the `@data_representation("...")` annotation string from a list of
/// annotations. Returns `None` if absent.
///
/// Possible valid values per OMG XTypes v1.3 (checked at parse-time in
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
        Some("XCDR1") | Some("PLAIN_CDR") => CdrVersion::Xcdr1,
        Some("XCDR2") | Some("PLAIN_CDR2") | None => CdrVersion::Xcdr2,
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
pub(super) fn xcdr_method_suffix(version: CdrVersion) -> &'static str {
    match version {
        CdrVersion::Xcdr1 => "xcdr1",
        CdrVersion::Xcdr2 => "xcdr2",
    }
}

pub(super) fn is_mutable_struct(s: &Struct) -> bool {
    s.extensibility.is_some()
        && matches!(
            s.extensibility,
            Some(crate::types::ExtensibilityKind::Mutable)
        )
        || s.annotations.iter().any(|a| {
            matches!(
                a,
                Annotation::Extensibility(crate::types::ExtensibilityKind::Mutable)
                    | Annotation::Mutable
            )
        })
}

/// Compact `PL_CDR2` mutable struct:
/// - Marked `@mutable` (or extensibility Mutable)
/// - All fields are non-optional primitives (no sequences/maps/arrays/named)
///
/// `FastDDS` encodes these as a flat sequence of `EMHEADER1` + payload without
/// an inner `DHEADER`. Sequences of such structs then add a `DHEADER` per element.
pub(super) fn is_compact_mutable_struct(s: &Struct) -> bool {
    if !is_mutable_struct(s) {
        return false;
    }

    s.fields
        .iter()
        .all(|f| !f.is_optional() && matches!(f.field_type, IdlType::Primitive(_)))
}

/// Check if a type is a bounded string (string<N> -> Sequence<Char, Some(N)>)
/// or bounded wstring (wstring<N> -> Sequence<`WChar`, Some(N)>)
pub(super) fn is_bounded_string(ty: &IdlType) -> bool {
    matches!(
        ty,
        IdlType::Sequence {
            inner,
            bound: Some(_),
        } if matches!(**inner, IdlType::Primitive(PrimitiveType::Char | PrimitiveType::WChar))
    )
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
            IdlType::Named(name) => name.clone(),
            IdlType::Sequence { inner, .. } => {
                // Check if this is a bounded string (string<N> -> sequence<char, N>)
                // or bounded wstring (wstring<N> -> sequence<wchar, N>)
                if matches!(
                    **inner,
                    IdlType::Primitive(PrimitiveType::Char | PrimitiveType::WChar)
                ) {
                    return "String".to_string();
                }
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

    /// XCDR v1 alignment table per OMG DDS-XTypes v1.3 (formal/2020-02-04).
    ///
    /// Reference: Section 7.4.1.1.1 "Primitive types", Table 31 (doc page 122).
    ///
    /// Returns alignment in bytes (1, 2, 4, or 8). Primitives align to their
    /// natural size, up to 8. Sequences, maps, and named type references align
    /// to 4 via their `uint32` length prefix.
    ///
    /// | Primitive type                                       | Alignment |
    /// | ---------------------------------------------------- | --------- |
    /// | `octet`, `bool`, `char`, `int8`, `uint8`             |  1        |
    /// | `short`, `ushort`, `int16`, `uint16`                 |  2        |
    /// | `long`, `ulong`, `int32`, `uint32`, `float`, `wchar` |  4        |
    /// | `string`, `wstring`, `fixed`                         |  4        |
    /// | `longlong`, `ulonglong`, `int64`, `uint64`           |  8        |
    /// | `double`, `long double`                              |  8        |
    pub(super) fn xcdr1_alignment(idl_type: &IdlType) -> usize {
        match idl_type {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Void
                | PrimitiveType::Octet
                | PrimitiveType::UInt8
                | PrimitiveType::Int8
                | PrimitiveType::Boolean
                | PrimitiveType::Char => 1,
                PrimitiveType::Short
                | PrimitiveType::UnsignedShort
                | PrimitiveType::Int16
                | PrimitiveType::UInt16 => 2,
                PrimitiveType::Long
                | PrimitiveType::UnsignedLong
                | PrimitiveType::Int32
                | PrimitiveType::UInt32
                | PrimitiveType::Float
                | PrimitiveType::WChar
                | PrimitiveType::String
                | PrimitiveType::WString
                | PrimitiveType::Fixed { .. } => 4,
                PrimitiveType::LongLong
                | PrimitiveType::UnsignedLongLong
                | PrimitiveType::Int64
                | PrimitiveType::UInt64
                | PrimitiveType::Double
                | PrimitiveType::LongDouble => 8,
            },
            IdlType::Array { inner, .. } => Self::xcdr1_alignment(inner),
            IdlType::Sequence { .. } | IdlType::Map { .. } | IdlType::Named(_) => 4,
        }
    }

    /// XCDR v2 alignment table per OMG DDS-XTypes v1.3 (formal/2020-02-04).
    ///
    /// References:
    /// - Section 7.4.2 "Extended CDR Representation (encoding version 2)",
    ///   doc page 129: "INT64, UINT64, FLOAT64, and FLOAT128 are serialized
    ///   into the CDR buffer at offsets that are aligned to 4 rather than 8
    ///   as was the case in PLAIN_CDR."
    /// - Section 7.4.3.2 "XCDR Stream State", `maxalign` variable (doc p.130).
    /// - Section 7.4.3.2.2 / Table 37 (doc p.132): `MAXALIGN(VERSION2) = 4`,
    ///   effective alignment = `MIN(type.alignment, XCDR.maxalign)`.
    ///
    /// Differs from [`Self::xcdr1_alignment`] only on 8-byte primitives:
    /// `longlong`, `ulonglong`, `int64`, `uint64`, `double`, `long double`
    /// align to **4** instead of 8.
    pub(super) fn xcdr2_alignment(idl_type: &IdlType) -> usize {
        match idl_type {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Void
                | PrimitiveType::Octet
                | PrimitiveType::UInt8
                | PrimitiveType::Int8
                | PrimitiveType::Boolean
                | PrimitiveType::Char => 1,
                PrimitiveType::Short
                | PrimitiveType::UnsignedShort
                | PrimitiveType::Int16
                | PrimitiveType::UInt16 => 2,
                PrimitiveType::Long
                | PrimitiveType::UnsignedLong
                | PrimitiveType::Int32
                | PrimitiveType::UInt32
                | PrimitiveType::Float
                | PrimitiveType::WChar
                | PrimitiveType::String
                | PrimitiveType::WString
                | PrimitiveType::Fixed { .. } => 4,
                PrimitiveType::LongLong
                | PrimitiveType::UnsignedLongLong
                | PrimitiveType::Int64
                | PrimitiveType::UInt64
                | PrimitiveType::Double
                | PrimitiveType::LongDouble => 4,
            },
            IdlType::Array { inner, .. } => Self::xcdr2_alignment(inner),
            IdlType::Sequence { .. } | IdlType::Map { .. } | IdlType::Named(_) => 4,
        }
    }

    /// Version-aware dispatcher. Every `emit_*` helper in `encode.rs`,
    /// `decode.rs`, `unions.rs`, `encode_containers.rs`, and
    /// `decode_containers.rs` receives a [`CdrVersion`] parameter and funnels
    /// through this dispatcher instead of calling the version-specific
    /// tables directly, so flipping the caller's target version reroutes the
    /// whole alignment chain.
    pub(super) fn xcdr_alignment(idl_type: &IdlType, version: CdrVersion) -> usize {
        match version {
            CdrVersion::Xcdr1 => Self::xcdr1_alignment(idl_type),
            CdrVersion::Xcdr2 => Self::xcdr2_alignment(idl_type),
        }
    }

    /// Emit `impl Cdr2Encode` / `impl Cdr2Decode` trait implementations that
    /// delegate to the per-version inherent methods emitted for type `name`.
    ///
    /// The delegator preserves the crate's existing `Cdr2Encode` /
    /// `Cdr2Decode` API while the inherent methods expose both XCDR v1 and
    /// XCDR v2 encoders. The target of the delegation is the type's
    /// `@data_representation` annotation (`@XCDR1` -> Xcdr1, otherwise Xcdr2).
    ///
    /// `encode_cdr2_le_at` is a REQUIRED trait method per DDS-XTypes v1.3
    /// §7.4.3.4.1 Tab.15 (offset-propagating entry point). The body is a
    /// trivial wrapper that calls `encode_cdr2_le` on a sub-slice and
    /// advances the global cursor by the returned length. This routes to
    /// the inherent `encode_xcdrN_le`, which is the actual encoder body.
    /// The inherent `encode_xcdrN_le_at` emitted by
    /// `RustGenerator::emit_encode_at_wrapper` is a parallel trivial
    /// wrapper over the same `encode_xcdrN_le` inherent — so calling
    /// either trait method or inherent `_at` ultimately reaches the same
    /// encoder body via a one-step sub-buffer slice.
    ///
    /// Inner-field call sites in struct/union/container templates use the
    /// offset-aware inherent `encode_xcdrN_le_at` so the global cursor
    /// propagates uniformly through nested types at the call-site level.
    /// Sub-buffer slicing is now confined to two trivial wrappers (the
    /// trait delegator emitted here and the inherent `_at` wrapper) plus
    /// `Fixed<D, S>`'s manually-inlined version of the same pattern.
    ///
    /// The inherent encoder bodies (`encode_xcdrN_le`) still compute
    /// alignment from a local `offset` cursor that starts at 0; lifting
    /// those bodies onto the global cursor (making the body operate on
    /// `*offset` directly) is a deeper refactor that closes the residual
    /// F01 surface for non-aligned outer offsets and is tracked separately.
    ///
    /// This is shared between struct and union codegen (2.2-a emits for
    /// structs, 2.2-c will emit for unions).
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
             \u{20}       let len = self.encode_{suffix}_le(&mut dst[*offset..])?;\n\
             \u{20}       *offset += len;\n\
             \u{20}       Ok(())\n\
             \u{20}   }}\n\
             }}\n\
             \n\
             impl Cdr2Decode for {name} {{\n\
             \u{20}   fn decode_cdr2_le(src: &[u8]) -> Result<(Self, usize), CdrError> {{\n\
             \u{20}       Self::decode_{suffix}_le(src)\n\
             \u{20}   }}\n\
             }}\n\
             \n"
        )
    }

    /// Calculate fixed size for primitives (None for variable-size types)
    pub(super) fn cdr2_fixed_size(idl_type: &IdlType) -> Option<usize> {
        match idl_type {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Octet
                | PrimitiveType::UInt8
                | PrimitiveType::Int8
                | PrimitiveType::Boolean
                | PrimitiveType::Char => Some(1),
                PrimitiveType::Short
                | PrimitiveType::UnsignedShort
                | PrimitiveType::Int16
                | PrimitiveType::UInt16 => Some(2),
                PrimitiveType::Long
                | PrimitiveType::UnsignedLong
                | PrimitiveType::Int32
                | PrimitiveType::UInt32
                | PrimitiveType::Float => Some(4),
                PrimitiveType::LongLong
                | PrimitiveType::UnsignedLongLong
                | PrimitiveType::Int64
                | PrimitiveType::UInt64
                | PrimitiveType::Double
                | PrimitiveType::LongDouble => Some(8),
                PrimitiveType::Fixed { .. } => Some(16),
                _ => None, // String, WString are variable size
            },
            IdlType::Array { inner, size } => {
                Self::cdr2_fixed_size(inner).map(|element_size| element_size * (*size as usize))
            }
            _ => None, // Sequences, Maps, Named types are variable size
        }
    }

    /// Check if a type is a single-byte type that can be directly memcpy'd
    /// (only u8, i8, bool - NOT floats, i32, etc.)
    pub(super) const fn is_byte_copyable(idl_type: &IdlType) -> bool {
        matches!(
            idl_type,
            IdlType::Primitive(
                PrimitiveType::Octet
                    | PrimitiveType::UInt8
                    | PrimitiveType::Int8
                    | PrimitiveType::Boolean
            )
        )
    }

    /// Check if a type is likely a primitive or primitive typedef (supports `to_le_bytes()`)
    /// Used for generating encoding code for arrays with named element types.
    pub(super) fn is_primitive_like(idl_type: &IdlType) -> Option<usize> {
        match idl_type {
            IdlType::Primitive(PrimitiveType::Fixed { .. }) => None,
            IdlType::Primitive(_) => Self::cdr2_fixed_size(idl_type),
            IdlType::Named(name) => match name.as_str() {
                "I8" | "Int8" | "U8" | "UInt8" | "Octet" => Some(1),
                "I16" | "Int16" | "Short" | "U16" | "UInt16" | "UShort" => Some(2),
                "I32" | "Int32" | "Long" | "U32" | "UInt32" | "ULong" | "F32" | "Float" => Some(4),
                "I64" | "Int64" | "LongLong" | "U64" | "UInt64" | "ULongLong" | "F64"
                | "Double" => Some(8),
                _ => None,
            },
            _ => None,
        }
    }

    /// Compute `MemberId` for mutable structs.
    ///
    /// Priority:
    /// - `@id` on the field
    /// - `@autoid(SEQUENTIAL)` on the struct -> use declaration order
    /// - default/`@autoid(HASH)` -> FNV-1a 32-bit on field name & `0x0FFF_FFFF`
    ///
    /// Algorithm: FNV-1a 32-bit (`XTypes` ss7.3.1.2.1.2), masked to 28 bits.
    pub(crate) fn compute_member_id(s: &Struct, idx: usize, field: &Field) -> u32 {
        for ann in &field.annotations {
            if let Annotation::Id(id) = ann {
                return *id;
            }
        }

        let autoid_seq = s
            .annotations
            .iter()
            .any(|a| matches!(a, Annotation::AutoId(AutoIdKind::Sequential)));
        if autoid_seq {
            // @audit-ok: safe cast - field index in struct always << u32::MAX
            #[allow(clippy::cast_possible_truncation)]
            return idx as u32;
        }

        Self::fnv1a_member_id(&field.name)
    }

    /// FNV-1a 32-bit hash masked to 28 bits, per `XTypes` ss7.3.1.2.1.2.
    pub(crate) fn fnv1a_member_id(name: &str) -> u32 {
        let mut h: u32 = 0x811c_9dc5; // FNV offset basis
        for b in name.as_bytes() {
            h ^= u32::from(*b);
            h = h.wrapping_mul(0x0100_0193); // FNV prime
        }
        h & 0x0fff_ffff
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
