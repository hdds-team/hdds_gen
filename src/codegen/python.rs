// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Python code generator
//!
//! Generates idiomatic Python 3 (dataclasses + type hints).
//!
//! Note: uninlined_format_args allowed here due to extensive format!() usage
//! in code generation that would require significant refactoring.

#![allow(clippy::uninlined_format_args)]

use crate::ast::{
    Bitmask, Bitset, Const, Definition, Enum, Field, IdlFile, Struct, Typedef, Union,
};
use crate::codegen::keywords::python_ident;
use crate::codegen::CodeGenerator;
use crate::error::Result;
use crate::types::{Annotation, AutoIdKind, ExtensibilityKind, IdlType, PrimitiveType};
use std::collections::HashMap;
use std::fmt::Write;

fn push_fmt(dst: &mut String, args: std::fmt::Arguments<'_>) {
    let _ = dst.write_fmt(args);
}

/// Check if a struct is MUTABLE extensibility
fn is_mutable_struct(s: &Struct) -> bool {
    matches!(s.extensibility, Some(ExtensibilityKind::Mutable))
        || s.annotations.iter().any(|a| {
            matches!(
                a,
                Annotation::Extensibility(ExtensibilityKind::Mutable) | Annotation::Mutable
            )
        })
}

/// Check if a type is a bounded string (string<N> parsed as Sequence<Char, N>)
fn is_bounded_string(ty: &IdlType) -> bool {
    matches!(
        ty,
        IdlType::Sequence {
            inner,
            bound: Some(_),
        } if matches!(**inner, IdlType::Primitive(PrimitiveType::Char | PrimitiveType::WChar))
    )
}

/// Check if a struct is APPENDABLE extensibility
fn is_appendable_struct(s: &Struct) -> bool {
    matches!(s.extensibility, Some(ExtensibilityKind::Appendable))
        || s.annotations.iter().any(|a| {
            matches!(
                a,
                Annotation::Extensibility(ExtensibilityKind::Appendable) | Annotation::Appendable
            )
        })
}

/// Compute member ID for a field in a mutable struct.
/// Priority: @id annotation > @autoid(SEQUENTIAL) > FNV-1a hash (`XTypes` ss7.3.1.2.1.2)
fn compute_member_id(s: &Struct, idx: usize, field: &Field) -> u32 {
    // Check for explicit @id annotation
    for ann in &field.annotations {
        if let Annotation::Id(id) = ann {
            return *id;
        }
    }

    // Check for @autoid(SEQUENTIAL) on struct
    let autoid_seq = s
        .annotations
        .iter()
        .any(|a| matches!(a, Annotation::AutoId(AutoIdKind::Sequential)));
    if autoid_seq {
        #[allow(clippy::cast_possible_truncation)]
        return idx as u32;
    }

    // Default: FNV-1a 32-bit hash masked to 28 bits
    fnv1a_member_id(&field.name)
}

/// FNV-1a 32-bit hash masked to 28 bits (`XTypes` ss7.3.1.2.1.2)
fn fnv1a_member_id(name: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in name.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h & 0x0fff_ffff
}

/// Get LC (Length Code) value based on fixed size
const fn get_lc_for_size(size: Option<usize>) -> u32 {
    match size {
        Some(1) => 0,
        Some(2) => 1,
        Some(4) => 2,
        Some(8) => 3,
        _ => 5, // NEXTINT
    }
}

/// Index of type definitions for CDR2 generation
struct DefinitionIndex<'a> {
    structs: HashMap<String, &'a Struct>,
    enums: HashMap<String, &'a Enum>,
    typedefs: HashMap<String, &'a Typedef>,
    bitsets: HashMap<String, &'a Bitset>,
    bitmasks: HashMap<String, &'a Bitmask>,
}

impl<'a> DefinitionIndex<'a> {
    fn from_file(file: &'a IdlFile) -> Self {
        let mut idx = Self {
            structs: HashMap::new(),
            enums: HashMap::new(),
            typedefs: HashMap::new(),
            bitsets: HashMap::new(),
            bitmasks: HashMap::new(),
        };
        idx.collect(&file.definitions);
        idx
    }

    fn collect(&mut self, defs: &'a [Definition]) {
        for def in defs {
            match def {
                Definition::Module(m) => self.collect(&m.definitions),
                Definition::Struct(s) => {
                    self.structs.insert(s.name.clone(), s);
                }
                Definition::Enum(e) => {
                    self.enums.insert(e.name.clone(), e);
                }
                Definition::Typedef(t) => {
                    self.typedefs.insert(t.name.clone(), t);
                }
                Definition::Bitset(b) => {
                    self.bitsets.insert(b.name.clone(), b);
                }
                Definition::Bitmask(m) => {
                    self.bitmasks.insert(m.name.clone(), m);
                }
                _ => {}
            }
        }
    }
}

/// Python code generator producing dataclasses and serde helpers.
pub struct PythonGenerator {
    indent_level: usize,
}

impl PythonGenerator {
    /// Creates a new Python generator.
    #[must_use]
    pub const fn new() -> Self {
        // Indent by one level inside class bodies by default
        Self { indent_level: 1 }
    }

    fn indent(&self) -> String {
        "    ".repeat(self.indent_level)
    }

    fn last_ident(name: &str) -> String {
        // Flatten module qualifiers like A::B::Type -> Type
        name.rfind("::").map_or_else(
            || {
                name.rfind('.')
                    .map_or_else(|| name.to_string(), |pos| name[pos + 1..].to_string())
            },
            |pos| name[pos + 2..].to_string(),
        )
    }

    fn type_to_python(t: &IdlType) -> String {
        match t {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Void => "None".to_string(),
                PrimitiveType::Boolean => "bool".to_string(),
                PrimitiveType::Char
                | PrimitiveType::WChar
                | PrimitiveType::Octet
                | PrimitiveType::Short
                | PrimitiveType::UnsignedShort
                | PrimitiveType::Long
                | PrimitiveType::UnsignedLong
                | PrimitiveType::LongLong
                | PrimitiveType::UnsignedLongLong
                | PrimitiveType::Int8
                | PrimitiveType::Int16
                | PrimitiveType::Int32
                | PrimitiveType::Int64
                | PrimitiveType::UInt8
                | PrimitiveType::UInt16
                | PrimitiveType::UInt32
                | PrimitiveType::UInt64 => "int".to_string(),
                PrimitiveType::Float | PrimitiveType::Double | PrimitiveType::LongDouble => {
                    "float".to_string()
                }
                PrimitiveType::String | PrimitiveType::WString => "str".to_string(),
                PrimitiveType::Fixed { .. } => "float".to_string(), // MVP: decimal later
            },
            IdlType::Named(n) => Self::last_ident(n),
            IdlType::Sequence { .. } if is_bounded_string(t) => "str".to_string(),
            IdlType::Sequence { inner, .. } | IdlType::Array { inner, .. } => {
                format!("List[{}]", Self::type_to_python(inner))
            }
            IdlType::Map { key, value, .. } => {
                format!(
                    "Dict[{}, {}]",
                    Self::type_to_python(key),
                    Self::type_to_python(value)
                )
            }
        }
    }

    fn emit_header() -> String {
        let mut out = String::new();
        out.push_str("\"\"\"\n");
        push_fmt(
            &mut out,
            format_args!("Generated by hddsgen v{}\n", env!("HDDS_VERSION")),
        );
        out.push_str("DO NOT EDIT\n\n");
        out.push_str("Python target (dataclasses + typing + CDR2 serialization)\n");
        out.push_str("\"\"\"\n\n");

        // Imports
        out.push_str("from dataclasses import dataclass, field\n");
        out.push_str("from typing import List, Dict, Optional, Union as UnionType, Tuple\n");
        out.push_str("from enum import IntEnum\n");
        out.push_str("import struct\n");
        out.push_str("import hashlib\n\n");
        out
    }

    /// Get struct.pack format for a primitive type
    const fn primitive_to_struct_format(p: &PrimitiveType) -> Option<(&'static str, usize)> {
        match p {
            PrimitiveType::Boolean | PrimitiveType::Octet | PrimitiveType::UInt8 => Some(("B", 1)),
            PrimitiveType::Char | PrimitiveType::Int8 => Some(("b", 1)),
            PrimitiveType::Short | PrimitiveType::Int16 => Some(("<h", 2)),
            PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => Some(("<H", 2)),
            PrimitiveType::Long | PrimitiveType::Int32 => Some(("<i", 4)),
            PrimitiveType::UnsignedLong | PrimitiveType::UInt32 | PrimitiveType::WChar => {
                Some(("<I", 4))
            }
            PrimitiveType::LongLong | PrimitiveType::Int64 => Some(("<q", 8)),
            PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => Some(("<Q", 8)),
            PrimitiveType::Float => Some(("<f", 4)),
            PrimitiveType::Double | PrimitiveType::LongDouble => Some(("<d", 8)),
            PrimitiveType::String
            | PrimitiveType::WString
            | PrimitiveType::Fixed { .. }
            | PrimitiveType::Void => None,
        }
    }

    fn alignment_of(t: &IdlType, idx: &DefinitionIndex) -> usize {
        match t {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Boolean
                | PrimitiveType::Octet
                | PrimitiveType::Char
                | PrimitiveType::Int8
                | PrimitiveType::UInt8
                | PrimitiveType::Void => 1,
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
            IdlType::Sequence { .. } | IdlType::Map { .. } => 4,
            IdlType::Array { inner, .. } => Self::alignment_of(inner, idx),
            IdlType::Named(nm) => {
                let ident = Self::last_ident(nm);
                if idx.enums.contains_key(&ident) || idx.structs.contains_key(&ident) {
                    4
                } else if idx.bitsets.contains_key(&ident) || idx.bitmasks.contains_key(&ident) {
                    8
                } else if let Some(td) = idx.typedefs.get(&ident) {
                    Self::alignment_of(&td.base_type, idx)
                } else {
                    4
                }
            }
        }
    }

    fn emit_struct(&self, s: &Struct, idx: &DefinitionIndex) -> String {
        let mut out = String::new();

        // Docstring with original IDL signature
        let mut idl_sig = String::new();
        push_fmt(&mut idl_sig, format_args!("struct {} {{ ", s.name));
        for (i, f) in s.fields.iter().enumerate() {
            if i > 0 {
                idl_sig.push(' ');
            }
            push_fmt(
                &mut idl_sig,
                format_args!("{} {};", f.field_type.to_idl_string(), f.name),
            );
        }
        push_fmt(&mut idl_sig, format_args!(" }}"));

        // Class header (inheritance when present)
        if let Some(base) = &s.base_struct {
            push_fmt(
                &mut out,
                format_args!("@dataclass\nclass {}({}):\n", s.name, base),
            );
        } else {
            push_fmt(&mut out, format_args!("@dataclass\nclass {}:\n", s.name));
        }
        push_fmt(
            &mut out,
            format_args!("{}\"\"\"IDL: {}\"\"\"\n", self.indent(), idl_sig),
        );

        // Fields - MUST sort so fields without defaults come before fields with defaults
        // Python dataclasses require this ordering, otherwise we get:
        // "TypeError: non-default argument follows default argument"
        // All fields get defaults so dataclass ordering doesn't matter
        for f in &s.fields {
            let is_opt = f.is_optional();
            let mut ty = Self::type_to_python(&f.field_type);
            let default_suffix = if is_opt {
                ty = format!("Optional[{ty}]");
                " = None".to_string()
            } else {
                match &f.field_type {
                    IdlType::Sequence { .. } if is_bounded_string(&f.field_type) => {
                        " = \"\"".to_string()
                    }
                    IdlType::Sequence { .. } | IdlType::Array { .. } => {
                        " = field(default_factory=list)".to_string()
                    }
                    IdlType::Map { .. } => " = field(default_factory=dict)".to_string(),
                    IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString) => {
                        " = \"\"".to_string()
                    }
                    IdlType::Primitive(PrimitiveType::Boolean) => " = False".to_string(),
                    IdlType::Primitive(PrimitiveType::Float | PrimitiveType::Double | PrimitiveType::LongDouble) => {
                        " = 0.0".to_string()
                    }
                    IdlType::Primitive(PrimitiveType::Char | PrimitiveType::WChar) => {
                        " = 0".to_string()
                    }
                    IdlType::Primitive(_) => " = 0".to_string(),
                    IdlType::Named(nm) => {
                        let type_ident = Self::last_ident(nm);
                        #[allow(clippy::option_if_let_else)]
                        if let Some(td) = idx.typedefs.get(&type_ident) {
                            // Resolve typedef to get default
                            match &td.base_type {
                                IdlType::Sequence { .. } | IdlType::Array { .. } => {
                                    " = field(default_factory=list)".to_string()
                                }
                                IdlType::Map { .. } => " = field(default_factory=dict)".to_string(),
                                IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString) => {
                                    " = \"\"".to_string()
                                }
                                _ => " = 0".to_string(),
                            }
                        } else if idx.structs.contains_key(&type_ident) {
                            format!(" = field(default_factory={type_ident})")
                        } else {
                            // enum, bitmask, bitset
                            " = 0".to_string()
                        }
                    }
                }
            };

            let ident = python_ident(&f.name);
            push_fmt(
                &mut out,
                format_args!("{}{}: {}{}\n", self.indent(), ident, ty, default_suffix),
            );
        }

        if s.fields.is_empty() {
            push_fmt(&mut out, format_args!("{}pass\n", self.indent()));
        }
        out.push('\n');

        // Generate CDR2 encode method
        out.push_str(&self.emit_encode_method(s, idx));
        out.push('\n');

        // Generate CDR2 decode method
        out.push_str(&self.emit_decode_method(s, idx));
        out.push('\n');

        // Generate type_name() classmethod
        out.push_str(&self.emit_type_name_method(s));
        out.push('\n');

        // Generate compute_key() method for @key fields
        out.push_str(&self.emit_compute_key_method(s));
        out.push('\n');

        // Generate has_key() classmethod
        out.push_str(&self.emit_has_key_method(s));
        out.push('\n');

        out
    }

    fn emit_encode_method(&self, s: &Struct, idx: &DefinitionIndex) -> String {
        if is_mutable_struct(s) {
            return self.emit_encode_method_mutable(s, idx);
        }
        if is_appendable_struct(s) {
            return self.emit_encode_method_appendable(s, idx);
        }
        self.emit_encode_method_final(s, idx)
    }

    /// Emit encode method for FINAL structs (no DHEADER)
    fn emit_encode_method_final(&self, s: &Struct, idx: &DefinitionIndex) -> String {
        let mut out = String::new();
        let indent = self.indent();
        let indent2 = format!("{}    ", indent);
        let indent3 = format!("{}        ", indent);

        push_fmt(
            &mut out,
            format_args!("{}def encode_cdr2_le(self) -> bytes:\n", indent),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}\"\"\"Encode to CDR2 little-endian format (FINAL extensibility).\"\"\"\n",
                indent2
            ),
        );
        push_fmt(
            &mut out,
            format_args!("{}parts: List[bytes] = []\n", indent2),
        );
        push_fmt(&mut out, format_args!("{}offset = 0\n", indent2));

        for f in &s.fields {
            let field_id = python_ident(&f.name);
            if f.is_optional() {
                // Optional field: write presence flag first
                push_fmt(
                    &mut out,
                    format_args!("{}# @optional field '{}': presence flag\n", indent2, f.name),
                );
                push_fmt(
                    &mut out,
                    format_args!("{}if self.{} is None:\n", indent2, field_id),
                );
                push_fmt(
                    &mut out,
                    format_args!("{}parts.append(b'\\x00')  # absent\n", indent3),
                );
                push_fmt(&mut out, format_args!("{}offset += 1\n", indent3));
                push_fmt(&mut out, format_args!("{}else:\n", indent2));
                push_fmt(
                    &mut out,
                    format_args!("{}parts.append(b'\\x01')  # present\n", indent3),
                );
                push_fmt(&mut out, format_args!("{}offset += 1\n", indent3));
                // Encode the value with extra indentation
                let field_code = Self::emit_encode_field(&field_id, &f.field_type, idx, &indent3);
                out.push_str(&field_code);
            } else {
                out.push_str(&Self::emit_encode_field(
                    &field_id,
                    &f.field_type,
                    idx,
                    &indent2,
                ));
            }
        }

        push_fmt(
            &mut out,
            format_args!("{}return b''.join(parts)\n", indent2),
        );
        out
    }

    /// Emit encode method for APPENDABLE structs (DHEADER only)
    fn emit_encode_method_appendable(&self, s: &Struct, idx: &DefinitionIndex) -> String {
        let mut out = String::new();
        let indent = self.indent();
        let indent2 = format!("{}    ", indent);
        let indent3 = format!("{}        ", indent);

        push_fmt(
            &mut out,
            format_args!("{}def encode_cdr2_le(self) -> bytes:\n", indent),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}\"\"\"Encode to CDR2 little-endian format (APPENDABLE extensibility).\"\"\"\n",
                indent2
            ),
        );
        push_fmt(
            &mut out,
            format_args!("{}parts: List[bytes] = []\n", indent2),
        );
        push_fmt(
            &mut out,
            format_args!("{}offset = 4  # Reserve space for DHEADER\n", indent2),
        );

        for f in &s.fields {
            let field_id = python_ident(&f.name);
            if f.is_optional() {
                // Optional field: write presence flag first
                push_fmt(
                    &mut out,
                    format_args!("{}# @optional field '{}': presence flag\n", indent2, f.name),
                );
                push_fmt(
                    &mut out,
                    format_args!("{}if self.{} is None:\n", indent2, field_id),
                );
                push_fmt(
                    &mut out,
                    format_args!("{}parts.append(b'\\x00')  # absent\n", indent3),
                );
                push_fmt(&mut out, format_args!("{}offset += 1\n", indent3));
                push_fmt(&mut out, format_args!("{}else:\n", indent2));
                push_fmt(
                    &mut out,
                    format_args!("{}parts.append(b'\\x01')  # present\n", indent3),
                );
                push_fmt(&mut out, format_args!("{}offset += 1\n", indent3));
                // Encode the value with extra indentation
                let field_code = Self::emit_encode_field(&field_id, &f.field_type, idx, &indent3);
                out.push_str(&field_code);
            } else {
                out.push_str(&Self::emit_encode_field(
                    &field_id,
                    &f.field_type,
                    idx,
                    &indent2,
                ));
            }
        }

        push_fmt(
            &mut out,
            format_args!("{}payload = b''.join(parts)\n", indent2),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}dheader = struct.pack('<I', len(payload))  # DHEADER: payload size\n",
                indent2
            ),
        );
        push_fmt(
            &mut out,
            format_args!("{}return dheader + payload\n", indent2),
        );
        out
    }

    /// Emit encode method for MUTABLE structs (DHEADER + EMHEADER per field)
    #[allow(clippy::too_many_lines)]
    fn emit_encode_method_mutable(&self, s: &Struct, idx: &DefinitionIndex) -> String {
        let mut out = String::new();
        let indent = self.indent();
        let indent2 = format!("{}    ", indent);

        push_fmt(
            &mut out,
            format_args!("{}def encode_cdr2_le(self) -> bytes:\n", indent),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}\"\"\"Encode to CDR2 little-endian format (MUTABLE extensibility).\"\"\"\n",
                indent2
            ),
        );
        push_fmt(
            &mut out,
            format_args!("{}parts: List[bytes] = []\n", indent2),
        );
        push_fmt(
            &mut out,
            format_args!("{}offset = 4  # Reserve space for DHEADER\n", indent2),
        );

        for (field_idx, f) in s.fields.iter().enumerate() {
            let field_id = python_ident(&f.name);
            let member_id = compute_member_id(s, field_idx, f);
            let fixed_size = Self::primitive_fixed_size(&f.field_type);
            let lc = get_lc_for_size(fixed_size);

            // Comment for this field
            push_fmt(
                &mut out,
                format_args!(
                    "{}# Field '{}': member_id={:#010X}, LC={}\n",
                    indent2, f.name, member_id, lc
                ),
            );

            // Emit EMHEADER (M bit for @key / @must_understand)
            let mu_bit = if f.is_key() || f.is_must_understand() {
                "0x80000000 | "
            } else {
                ""
            };
            push_fmt(
                &mut out,
                format_args!(
                    "{}emheader = {}({} << 28) | ({:#010X} & 0x0FFFFFFF)\n",
                    indent2, mu_bit, lc, member_id
                ),
            );
            push_fmt(
                &mut out,
                format_args!("{}parts.append(struct.pack('<I', emheader))\n", indent2),
            );
            push_fmt(&mut out, format_args!("{}offset += 4\n", indent2));

            if lc == 5 {
                // NEXTINT: need to encode size after EMHEADER
                push_fmt(
                    &mut out,
                    format_args!("{}# NEXTINT: encode field size\n", indent2),
                );
                push_fmt(
                    &mut out,
                    format_args!("{}field_parts: List[bytes] = []\n", indent2),
                );
                push_fmt(&mut out, format_args!("{}field_offset = 0\n", indent2));

                // Encode field into field_parts
                out.push_str(&Self::emit_encode_field_to_var(
                    &field_id,
                    &f.field_type,
                    idx,
                    &indent2,
                    "field_parts",
                    "field_offset",
                ));

                push_fmt(
                    &mut out,
                    format_args!("{}field_data = b''.join(field_parts)\n", indent2),
                );
                push_fmt(
                    &mut out,
                    format_args!(
                        "{}parts.append(struct.pack('<I', len(field_data)))  # NEXTINT\n",
                        indent2
                    ),
                );
                push_fmt(&mut out, format_args!("{}offset += 4\n", indent2));
                push_fmt(
                    &mut out,
                    format_args!("{}parts.append(field_data)\n", indent2),
                );
                push_fmt(
                    &mut out,
                    format_args!("{}offset += len(field_data)\n", indent2),
                );
            } else {
                // Fixed size: encode directly after EMHEADER (no NEXTINT)
                out.push_str(&Self::emit_encode_field(
                    &field_id,
                    &f.field_type,
                    idx,
                    &indent2,
                ));
            }
        }

        push_fmt(
            &mut out,
            format_args!("{}payload = b''.join(parts)\n", indent2),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}dheader = struct.pack('<I', len(payload))  # DHEADER: payload size\n",
                indent2
            ),
        );
        push_fmt(
            &mut out,
            format_args!("{}return dheader + payload\n", indent2),
        );
        out
    }

    /// Get fixed size for primitives (None for variable-size types)
    fn primitive_fixed_size(ty: &IdlType) -> Option<usize> {
        match ty {
            IdlType::Primitive(p) => Self::primitive_to_struct_format(p).map(|(_, size)| size),
            _ => None,
        }
    }

    /// Emit encode field code to a specific variable (for MUTABLE NEXTINT case)
    fn emit_encode_field_to_var(
        name: &str,
        ty: &IdlType,
        idx: &DefinitionIndex,
        indent: &str,
        parts_var: &str,
        offset_var: &str,
    ) -> String {
        // Replace "parts" with parts_var and "offset" with offset_var in the output
        let standard_code = Self::emit_encode_field(name, ty, idx, indent);
        standard_code
            .replace("parts.append", &format!("{parts_var}.append"))
            .replace("offset +=", &format!("{offset_var} +="))
            .replace("offset %", &format!("{offset_var} %"))
    }

    #[allow(clippy::too_many_lines)]
    fn emit_encode_field(name: &str, ty: &IdlType, idx: &DefinitionIndex, indent: &str) -> String {
        let mut out = String::new();

        match ty {
            IdlType::Primitive(p) => {
                if let Some((fmt, size)) = Self::primitive_to_struct_format(p) {
                    let align = size.max(1);
                    push_fmt(
                        &mut out,
                        format_args!("{indent}# align to {align} and pack {name}\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}pad = ({align} - (offset % {align})) % {align}\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}parts.append(b'\\x00' * pad)\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += pad\n"));
                    push_fmt(
                        &mut out,
                        format_args!("{indent}parts.append(struct.pack('{fmt}', self.{name}))\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += {size}\n"));
                } else if matches!(p, PrimitiveType::String) {
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}# string: align to 4, write length (with NUL), then bytes\n"
                        ),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}pad = (4 - (offset % 4)) % 4\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}parts.append(b'\\x00' * pad)\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += pad\n"));
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_bytes = self.{name}.encode('utf-8') + b'\\x00'\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}parts.append(struct.pack('<I', len(_bytes)))\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                    push_fmt(&mut out, format_args!("{indent}parts.append(_bytes)\n"));
                    push_fmt(&mut out, format_args!("{indent}offset += len(_bytes)\n"));
                } else {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}# unsupported primitive type for {name}\n"),
                    );
                }
            }
            IdlType::Sequence { .. } if is_bounded_string(ty) => {
                // Bounded string<N>: encode as CDR2 string (length with NUL + bytes + NUL)
                push_fmt(
                    &mut out,
                    format_args!(
                        "{indent}# bounded string: align to 4, write length (with NUL), then bytes\n"
                    ),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}pad = (4 - (offset % 4)) % 4\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}parts.append(b'\\x00' * pad)\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += pad\n"));
                push_fmt(
                    &mut out,
                    format_args!("{indent}_bytes = self.{name}.encode('utf-8') + b'\\x00'\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}parts.append(struct.pack('<I', len(_bytes)))\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                push_fmt(&mut out, format_args!("{indent}parts.append(_bytes)\n"));
                push_fmt(&mut out, format_args!("{indent}offset += len(_bytes)\n"));
            }
            IdlType::Sequence { inner, .. } => {
                let _align = Self::alignment_of(inner, idx);
                push_fmt(
                    &mut out,
                    format_args!("{indent}# sequence: align to 4, write length, then elements\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}pad = (4 - (offset % 4)) % 4\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}parts.append(b'\\x00' * pad)\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += pad\n"));
                push_fmt(
                    &mut out,
                    format_args!("{indent}parts.append(struct.pack('<I', len(self.{name})))\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                push_fmt(
                    &mut out,
                    format_args!("{indent}for _elem in self.{name}:\n"),
                );
                // Emit element encoding
                out.push_str(&Self::emit_encode_element(
                    "_elem",
                    inner,
                    idx,
                    &format!("{indent}    "),
                ));
            }
            IdlType::Array { inner, size } => {
                let align = Self::alignment_of(inner, idx);
                push_fmt(
                    &mut out,
                    format_args!("{indent}# array: align to {align}, then {size} elements\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}pad = ({align} - (offset % {align})) % {align}\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}parts.append(b'\\x00' * pad)\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += pad\n"));
                push_fmt(
                    &mut out,
                    format_args!("{indent}for _elem in self.{name}:\n"),
                );
                out.push_str(&Self::emit_encode_element(
                    "_elem",
                    inner,
                    idx,
                    &format!("{indent}    "),
                ));
            }
            IdlType::Named(nm) => {
                let type_name = Self::last_ident(nm);
                if idx.structs.contains_key(&type_name) {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}# nested struct {type_name}\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_sub = self.{name}.encode_cdr2_le()\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}parts.append(_sub)\n"));
                    push_fmt(&mut out, format_args!("{indent}offset += len(_sub)\n"));
                } else if idx.enums.contains_key(&type_name) {
                    push_fmt(&mut out, format_args!("{indent}# enum as int32\n"));
                    push_fmt(
                        &mut out,
                        format_args!("{indent}pad = (4 - (offset % 4)) % 4\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}parts.append(b'\\x00' * pad)\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += pad\n"));
                    push_fmt(
                        &mut out,
                        format_args!("{indent}parts.append(struct.pack('<i', int(self.{name})))\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                } else if let Some(td) = idx.typedefs.get(&type_name) {
                    out.push_str(&Self::emit_encode_field(name, &td.base_type, idx, indent));
                } else {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}# unknown type {type_name} for {name}\n"),
                    );
                }
            }
            IdlType::Map { key, value, .. } => {
                push_fmt(
                    &mut out,
                    format_args!("{indent}# map: align to 4, write length, then key-value pairs\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}pad = (4 - (offset % 4)) % 4\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}parts.append(b'\\x00' * pad)\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += pad\n"));
                push_fmt(
                    &mut out,
                    format_args!("{indent}parts.append(struct.pack('<I', len(self.{name})))\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                push_fmt(
                    &mut out,
                    format_args!("{indent}for _key, _val in self.{name}.items():\n"),
                );
                // Emit key encoding
                out.push_str(&Self::emit_encode_element(
                    "_key",
                    key,
                    idx,
                    &format!("{indent}    "),
                ));
                // Emit value encoding
                out.push_str(&Self::emit_encode_element(
                    "_val",
                    value,
                    idx,
                    &format!("{indent}    "),
                ));
            }
        }
        out
    }

    fn emit_encode_element(var: &str, ty: &IdlType, idx: &DefinitionIndex, indent: &str) -> String {
        let mut out = String::new();
        match ty {
            IdlType::Primitive(p) => {
                if let Some((fmt, size)) = Self::primitive_to_struct_format(p) {
                    let align = size.max(1);
                    push_fmt(
                        &mut out,
                        format_args!("{indent}pad = ({align} - (offset % {align})) % {align}\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}parts.append(b'\\x00' * pad)\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += pad\n"));
                    push_fmt(
                        &mut out,
                        format_args!("{indent}parts.append(struct.pack('{fmt}', {var}))\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += {size}\n"));
                } else if matches!(p, PrimitiveType::String | PrimitiveType::WString) {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}pad = (4 - (offset % 4)) % 4\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}parts.append(b'\\x00' * pad)\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += pad\n"));
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_encoded = {var}.encode('utf-8') + b'\\x00'\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}parts.append(struct.pack('<I', len(_encoded)))\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                    push_fmt(
                        &mut out,
                        format_args!("{indent}parts.append(_encoded)\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}offset += len(_encoded)\n"),
                    );
                }
            }
            IdlType::Sequence { inner, .. } | IdlType::Array { inner, .. } => {
                // Nested sequence/array: write length prefix then recurse
                push_fmt(
                    &mut out,
                    format_args!("{indent}pad = (4 - (offset % 4)) % 4\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}parts.append(b'\\x00' * pad)\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += pad\n"));
                push_fmt(
                    &mut out,
                    format_args!("{indent}parts.append(struct.pack('<I', len({var})))\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                push_fmt(
                    &mut out,
                    format_args!("{indent}for _inner_elem in {var}:\n"),
                );
                let inner_indent = format!("{indent}    ");
                out.push_str(&Self::emit_encode_element(
                    "_inner_elem",
                    inner,
                    idx,
                    &inner_indent,
                ));
            }
            IdlType::Named(nm) => {
                let type_name = Self::last_ident(nm);
                if idx.structs.contains_key(&type_name) {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_sub = {var}.encode_cdr2_le()\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}parts.append(_sub)\n"));
                    push_fmt(&mut out, format_args!("{indent}offset += len(_sub)\n"));
                }
            }
            IdlType::Map { .. } => {}
        }
        out
    }

    fn emit_decode_method(&self, s: &Struct, idx: &DefinitionIndex) -> String {
        if is_mutable_struct(s) {
            return self.emit_decode_method_mutable(s, idx);
        }
        if is_appendable_struct(s) {
            return self.emit_decode_method_appendable(s, idx);
        }
        self.emit_decode_method_final(s, idx)
    }

    /// Emit decode method for FINAL structs (no DHEADER)
    fn emit_decode_method_final(&self, s: &Struct, idx: &DefinitionIndex) -> String {
        let mut out = String::new();
        let indent = self.indent();
        let indent2 = format!("{}    ", indent);
        let indent3 = format!("{}        ", indent);

        push_fmt(&mut out, format_args!("{}@classmethod\n", indent));
        push_fmt(
            &mut out,
            format_args!(
                "{}def decode_cdr2_le(cls, data: bytes) -> Tuple['{}', int]:\n",
                indent, s.name
            ),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}\"\"\"Decode from CDR2 little-endian format (FINAL extensibility). Returns (instance, bytes_read).\"\"\"\n",
                indent2
            ),
        );
        push_fmt(&mut out, format_args!("{}offset = 0\n", indent2));

        // Decode fields
        for f in &s.fields {
            let field_id = python_ident(&f.name);
            if f.is_optional() {
                // Optional field: read presence flag first
                push_fmt(
                    &mut out,
                    format_args!(
                        "{}# @optional field '{}': read presence flag\n",
                        indent2, f.name
                    ),
                );
                push_fmt(
                    &mut out,
                    format_args!("{}_has_{} = data[offset] != 0\n", indent2, field_id),
                );
                push_fmt(&mut out, format_args!("{}offset += 1\n", indent2));
                push_fmt(&mut out, format_args!("{}if _has_{}:\n", indent2, field_id));
                // Decode the value with extra indentation
                let field_code = Self::emit_decode_field(&field_id, &f.field_type, idx, &indent3);
                out.push_str(&field_code);
                push_fmt(&mut out, format_args!("{}else:\n", indent2));
                push_fmt(&mut out, format_args!("{}_{} = None\n", indent3, field_id));
            } else {
                out.push_str(&Self::emit_decode_field(
                    &field_id,
                    &f.field_type,
                    idx,
                    &indent2,
                ));
            }
        }

        // Create instance
        push_fmt(&mut out, format_args!("{}return cls(", indent2));
        for (i, f) in s.fields.iter().enumerate() {
            let field_id = python_ident(&f.name);
            if i > 0 {
                out.push_str(", ");
            }
            push_fmt(&mut out, format_args!("{}=_{}", field_id, field_id));
        }
        out.push_str("), offset\n");
        out
    }

    /// Emit decode method for APPENDABLE structs (DHEADER only)
    fn emit_decode_method_appendable(&self, s: &Struct, idx: &DefinitionIndex) -> String {
        let mut out = String::new();
        let indent = self.indent();
        let indent2 = format!("{}    ", indent);
        let indent3 = format!("{}        ", indent);

        push_fmt(&mut out, format_args!("{}@classmethod\n", indent));
        push_fmt(
            &mut out,
            format_args!(
                "{}def decode_cdr2_le(cls, data: bytes) -> Tuple['{}', int]:\n",
                indent, s.name
            ),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}\"\"\"Decode from CDR2 little-endian format (APPENDABLE extensibility). Returns (instance, bytes_read).\"\"\"\n",
                indent2
            ),
        );
        push_fmt(&mut out, format_args!("{}offset = 0\n", indent2));

        // Read DHEADER
        push_fmt(
            &mut out,
            format_args!("{}# Read DHEADER (payload size)\n", indent2),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}_dheader, = struct.unpack_from('<I', data, offset)\n",
                indent2
            ),
        );
        push_fmt(&mut out, format_args!("{}offset += 4\n", indent2));
        push_fmt(
            &mut out,
            format_args!("{}_payload_end = offset + _dheader\n", indent2),
        );

        // Decode fields
        for f in &s.fields {
            let field_id = python_ident(&f.name);
            if f.is_optional() {
                // Optional field: read presence flag first
                push_fmt(
                    &mut out,
                    format_args!(
                        "{}# @optional field '{}': read presence flag\n",
                        indent2, f.name
                    ),
                );
                push_fmt(
                    &mut out,
                    format_args!("{}_has_{} = data[offset] != 0\n", indent2, field_id),
                );
                push_fmt(&mut out, format_args!("{}offset += 1\n", indent2));
                push_fmt(&mut out, format_args!("{}if _has_{}:\n", indent2, field_id));
                // Decode the value with extra indentation
                let field_code = Self::emit_decode_field(&field_id, &f.field_type, idx, &indent3);
                out.push_str(&field_code);
                push_fmt(&mut out, format_args!("{}else:\n", indent2));
                push_fmt(&mut out, format_args!("{}_{} = None\n", indent3, field_id));
            } else {
                out.push_str(&Self::emit_decode_field(
                    &field_id,
                    &f.field_type,
                    idx,
                    &indent2,
                ));
            }
        }

        // Create instance
        push_fmt(&mut out, format_args!("{}return cls(", indent2));
        for (i, f) in s.fields.iter().enumerate() {
            let field_id = python_ident(&f.name);
            if i > 0 {
                out.push_str(", ");
            }
            push_fmt(&mut out, format_args!("{}=_{}", field_id, field_id));
        }
        out.push_str("), _payload_end\n");
        out
    }

    /// Emit decode method for MUTABLE structs (DHEADER + EMHEADER per field)
    #[allow(clippy::too_many_lines)]
    fn emit_decode_method_mutable(&self, s: &Struct, idx: &DefinitionIndex) -> String {
        let mut out = String::new();
        let indent = self.indent();
        let indent2 = format!("{}    ", indent);
        let indent3 = format!("{}        ", indent);

        push_fmt(&mut out, format_args!("{}@classmethod\n", indent));
        push_fmt(
            &mut out,
            format_args!(
                "{}def decode_cdr2_le(cls, data: bytes) -> Tuple['{}', int]:\n",
                indent, s.name
            ),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}\"\"\"Decode from CDR2 little-endian format (MUTABLE extensibility). Returns (instance, bytes_read).\"\"\"\n",
                indent2
            ),
        );
        push_fmt(&mut out, format_args!("{}offset = 0\n", indent2));

        // Read DHEADER
        push_fmt(
            &mut out,
            format_args!("{}# Read DHEADER (payload size)\n", indent2),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}_dheader, = struct.unpack_from('<I', data, offset)\n",
                indent2
            ),
        );
        push_fmt(&mut out, format_args!("{}offset += 4\n", indent2));
        push_fmt(
            &mut out,
            format_args!("{}_payload_end = offset + _dheader\n", indent2),
        );

        // Initialize field variables with default values
        push_fmt(
            &mut out,
            format_args!("{}# Initialize fields with default values\n", indent2),
        );
        for f in &s.fields {
            let field_id = python_ident(&f.name);
            let default = Self::get_default_value(&f.field_type);
            push_fmt(
                &mut out,
                format_args!("{}_{} = {}\n", indent2, field_id, default),
            );
        }

        // Read fields in loop
        push_fmt(
            &mut out,
            format_args!("{}# Decode fields by member ID\n", indent2),
        );
        push_fmt(
            &mut out,
            format_args!("{}while offset < _payload_end:\n", indent2),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}_emheader, = struct.unpack_from('<I', data, offset)\n",
                indent3
            ),
        );
        push_fmt(&mut out, format_args!("{}offset += 4\n", indent3));
        push_fmt(
            &mut out,
            format_args!("{}_lc = (_emheader >> 28) & 0xF\n", indent3),
        );
        push_fmt(
            &mut out,
            format_args!("{}_member_id = _emheader & 0x0FFFFFFF\n", indent3),
        );

        // Compute member size based on LC
        push_fmt(
            &mut out,
            format_args!("{}# Determine member size from LC\n", indent3),
        );
        push_fmt(&mut out, format_args!("{}if _lc == 0:\n", indent3));
        push_fmt(&mut out, format_args!("{}    _member_size = 1\n", indent3));
        push_fmt(&mut out, format_args!("{}elif _lc == 1:\n", indent3));
        push_fmt(&mut out, format_args!("{}    _member_size = 2\n", indent3));
        push_fmt(&mut out, format_args!("{}elif _lc == 2:\n", indent3));
        push_fmt(&mut out, format_args!("{}    _member_size = 4\n", indent3));
        push_fmt(&mut out, format_args!("{}elif _lc == 3:\n", indent3));
        push_fmt(&mut out, format_args!("{}    _member_size = 8\n", indent3));
        push_fmt(
            &mut out,
            format_args!("{}elif _lc == 5:  # NEXTINT\n", indent3),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}    _member_size, = struct.unpack_from('<I', data, offset)\n",
                indent3
            ),
        );
        push_fmt(&mut out, format_args!("{}    offset += 4\n", indent3));
        push_fmt(&mut out, format_args!("{}else:\n", indent3));
        push_fmt(
            &mut out,
            format_args!(
                "{}    raise ValueError(f\"Unknown LC value: {{_lc}}\")\n",
                indent3
            ),
        );

        // Dispatch based on member_id
        push_fmt(&mut out, format_args!("{}_field_start = offset\n", indent3));
        let mut first = true;
        for (field_idx, f) in s.fields.iter().enumerate() {
            let field_id = python_ident(&f.name);
            let member_id = compute_member_id(s, field_idx, f);
            let keyword = if first { "if" } else { "elif" };
            first = false;

            push_fmt(
                &mut out,
                format_args!(
                    "{}{} _member_id == {:#010X}:  # {}\n",
                    indent3, keyword, member_id, f.name
                ),
            );
            let indent4 = format!("{}    ", indent3);
            out.push_str(&Self::emit_decode_field(
                &field_id,
                &f.field_type,
                idx,
                &indent4,
            ));
        }

        // Skip unknown fields
        push_fmt(&mut out, format_args!("{}else:\n", indent3));
        push_fmt(
            &mut out,
            format_args!(
                "{}    offset = _field_start + _member_size  # Skip unknown field\n",
                indent3
            ),
        );

        // Create instance
        push_fmt(&mut out, format_args!("{}return cls(", indent2));
        for (i, f) in s.fields.iter().enumerate() {
            let field_id = python_ident(&f.name);
            if i > 0 {
                out.push_str(", ");
            }
            push_fmt(&mut out, format_args!("{}=_{}", field_id, field_id));
        }
        out.push_str("), _payload_end\n");
        out
    }

    /// Get default value for a type (for MUTABLE struct field initialization)
    const fn get_default_value(ty: &IdlType) -> &'static str {
        match ty {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Boolean => "False",
                PrimitiveType::String | PrimitiveType::WString => "\"\"",
                PrimitiveType::Float | PrimitiveType::Double | PrimitiveType::LongDouble => "0.0",
                _ => "0",
            },
            IdlType::Sequence { .. } | IdlType::Array { .. } => "[]",
            IdlType::Map { .. } => "{}",
            IdlType::Named(_) => "None",
        }
    }

    #[allow(clippy::too_many_lines)]
    fn emit_decode_field(name: &str, ty: &IdlType, idx: &DefinitionIndex, indent: &str) -> String {
        let mut out = String::new();

        match ty {
            IdlType::Primitive(p) => {
                if let Some((fmt, size)) = Self::primitive_to_struct_format(p) {
                    let align = size.max(1);
                    push_fmt(
                        &mut out,
                        format_args!("{indent}# align and unpack {name}\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}offset += ({align} - (offset % {align})) % {align}\n"
                        ),
                    );
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}_{name}, = struct.unpack_from('{fmt}', data, offset)\n"
                        ),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += {size}\n"));
                } else if matches!(p, PrimitiveType::String) {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}# string: align, read length, then bytes\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}offset += (4 - (offset % 4)) % 4\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_len, = struct.unpack_from('<I', data, offset)\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}_{name} = data[offset:offset+_len-1].decode('utf-8')\n"
                        ),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += _len\n"));
                } else {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_{name} = None  # unsupported type\n"),
                    );
                }
            }
            IdlType::Sequence { .. } if is_bounded_string(ty) => {
                // Bounded string<N>: decode as CDR2 string (length with NUL + bytes + NUL)
                push_fmt(
                    &mut out,
                    format_args!("{indent}# bounded string: align, read length, then bytes\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}offset += (4 - (offset % 4)) % 4\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}_len, = struct.unpack_from('<I', data, offset)\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                push_fmt(
                    &mut out,
                    format_args!("{indent}_{name} = data[offset:offset+_len-1].decode('utf-8')\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += _len\n"));
            }
            IdlType::Sequence { inner, .. } => {
                push_fmt(
                    &mut out,
                    format_args!("{indent}# sequence: align, read length, then elements\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}offset += (4 - (offset % 4)) % 4\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}_seq_len, = struct.unpack_from('<I', data, offset)\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                push_fmt(&mut out, format_args!("{indent}_{name} = []\n"));
                push_fmt(
                    &mut out,
                    format_args!("{indent}for _ in range(_seq_len):\n"),
                );
                out.push_str(&Self::emit_decode_element(
                    name,
                    inner,
                    idx,
                    &format!("{indent}    "),
                ));
            }
            IdlType::Array { inner, size } => {
                let align = Self::alignment_of(inner, idx);
                push_fmt(
                    &mut out,
                    format_args!("{indent}# array: align, then {size} elements\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}offset += ({align} - (offset % {align})) % {align}\n"),
                );
                push_fmt(&mut out, format_args!("{indent}_{name} = []\n"));
                push_fmt(&mut out, format_args!("{indent}for _ in range({size}):\n"));
                out.push_str(&Self::emit_decode_element(
                    name,
                    inner,
                    idx,
                    &format!("{indent}    "),
                ));
            }
            IdlType::Named(nm) => {
                let type_name = Self::last_ident(nm);
                if idx.structs.contains_key(&type_name) {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}# nested struct {type_name}\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}_{name}, _read = {type_name}.decode_cdr2_le(data[offset:])\n"
                        ),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += _read\n"));
                } else if idx.enums.contains_key(&type_name) {
                    push_fmt(&mut out, format_args!("{indent}# enum as int32\n"));
                    push_fmt(
                        &mut out,
                        format_args!("{indent}offset += (4 - (offset % 4)) % 4\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_val, = struct.unpack_from('<i', data, offset)\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_{name} = {type_name}(_val)\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                } else if let Some(td) = idx.typedefs.get(&type_name) {
                    out.push_str(&Self::emit_decode_field(name, &td.base_type, idx, indent));
                } else {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_{name} = None  # unknown type {type_name}\n"),
                    );
                }
            }
            IdlType::Map { key, value, .. } => {
                push_fmt(
                    &mut out,
                    format_args!("{indent}# map: align, read length, then key-value pairs\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}offset += (4 - (offset % 4)) % 4\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}_map_len, = struct.unpack_from('<I', data, offset)\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                push_fmt(&mut out, format_args!("{indent}_{name} = {{}}\n"));
                push_fmt(
                    &mut out,
                    format_args!("{indent}for _ in range(_map_len):\n"),
                );
                // Emit key decoding
                out.push_str(&Self::emit_decode_map_key_value(
                    key,
                    value,
                    name,
                    idx,
                    &format!("{indent}    "),
                ));
            }
        }
        out
    }

    fn emit_decode_element(
        list_name: &str,
        ty: &IdlType,
        idx: &DefinitionIndex,
        indent: &str,
    ) -> String {
        let mut out = String::new();
        match ty {
            IdlType::Primitive(p) => {
                if let Some((fmt, size)) = Self::primitive_to_struct_format(p) {
                    let align = size.max(1);
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}offset += ({align} - (offset % {align})) % {align}\n"
                        ),
                    );
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}_elem, = struct.unpack_from('{fmt}', data, offset)\n"
                        ),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += {size}\n"));
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_{list_name}.append(_elem)\n"),
                    );
                } else if matches!(p, PrimitiveType::String | PrimitiveType::WString) {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}offset += (4 - (offset % 4)) % 4\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}_slen, = struct.unpack_from('<I', data, offset)\n"
                        ),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}_elem = data[offset:offset+_slen-1].decode('utf-8')\n"
                        ),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += _slen\n"));
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_{list_name}.append(_elem)\n"),
                    );
                }
            }
            IdlType::Sequence { inner, .. } | IdlType::Array { inner, .. } => {
                // Nested sequence/array: read length, then recurse
                push_fmt(
                    &mut out,
                    format_args!("{indent}offset += (4 - (offset % 4)) % 4\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}_inner_len, = struct.unpack_from('<I', data, offset)\n"),
                );
                push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                push_fmt(
                    &mut out,
                    format_args!("{indent}_inner_list = []\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}for _ in range(_inner_len):\n"),
                );
                let inner_indent = format!("{indent}    ");
                out.push_str(&Self::emit_decode_element(
                    "inner_list",
                    inner,
                    idx,
                    &inner_indent,
                ));
                push_fmt(
                    &mut out,
                    format_args!("{indent}_{list_name}.append(_inner_list)\n"),
                );
            }
            IdlType::Named(nm) => {
                let type_name = Self::last_ident(nm);
                if idx.structs.contains_key(&type_name) {
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}_elem, _read = {type_name}.decode_cdr2_le(data[offset:])\n"
                        ),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += _read\n"));
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_{list_name}.append(_elem)\n"),
                    );
                }
            }
            IdlType::Map { .. } => {}
        }
        out
    }

    fn emit_decode_map_key_value(
        key_ty: &IdlType,
        val_ty: &IdlType,
        map_name: &str,
        idx: &DefinitionIndex,
        indent: &str,
    ) -> String {
        let mut out = String::new();
        // Decode key
        out.push_str(&Self::emit_decode_single_value("_k", key_ty, idx, indent));
        // Decode value
        out.push_str(&Self::emit_decode_single_value("_v", val_ty, idx, indent));
        // Insert into map
        push_fmt(&mut out, format_args!("{indent}_{map_name}[_k] = _v\n"));
        out
    }

    fn emit_decode_single_value(
        var_name: &str,
        ty: &IdlType,
        idx: &DefinitionIndex,
        indent: &str,
    ) -> String {
        let mut out = String::new();
        match ty {
            IdlType::Primitive(p) => {
                if let Some((fmt, size)) = Self::primitive_to_struct_format(p) {
                    let align = size.max(1);
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}offset += ({align} - (offset % {align})) % {align}\n"
                        ),
                    );
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}{var_name}, = struct.unpack_from('{fmt}', data, offset)\n"
                        ),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += {size}\n"));
                } else if matches!(p, PrimitiveType::String) {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}offset += (4 - (offset % 4)) % 4\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_slen, = struct.unpack_from('<I', data, offset)\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}{var_name} = data[offset:offset+_slen-1].decode('utf-8')\n"
                        ),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += _slen\n"));
                } else {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}{var_name} = None  # unsupported primitive\n"),
                    );
                }
            }
            IdlType::Named(nm) => {
                let type_name = Self::last_ident(nm);
                if idx.structs.contains_key(&type_name) {
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}{var_name}, _read = {type_name}.decode_cdr2_le(data[offset:])\n"
                        ),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += _read\n"));
                } else if idx.enums.contains_key(&type_name) {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}offset += (4 - (offset % 4)) % 4\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_val, = struct.unpack_from('<i', data, offset)\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}{var_name} = {type_name}(_val)\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                } else if let Some(td) = idx.typedefs.get(&type_name) {
                    out.push_str(&Self::emit_decode_single_value(
                        var_name,
                        &td.base_type,
                        idx,
                        indent,
                    ));
                } else {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}{var_name} = None  # unknown type {type_name}\n"),
                    );
                }
            }
            _ => {
                push_fmt(
                    &mut out,
                    format_args!(
                        "{indent}{var_name} = None  # complex type not supported in map\n"
                    ),
                );
            }
        }
        out
    }

    /// Generate `type_name()` classmethod
    fn emit_type_name_method(&self, s: &Struct) -> String {
        let mut out = String::new();
        let indent = self.indent();
        let indent2 = format!("{}    ", indent);

        push_fmt(&mut out, format_args!("{}@classmethod\n", indent));
        push_fmt(
            &mut out,
            format_args!("{}def type_name(cls) -> str:\n", indent),
        );
        push_fmt(
            &mut out,
            format_args!("{}\"\"\"Return the DDS type name.\"\"\"\n", indent2),
        );
        push_fmt(&mut out, format_args!("{}return \"{}\"\n", indent2, s.name));
        out
    }

    /// Generate `compute_key()` method for `@key` fields
    #[allow(clippy::too_many_lines)]
    fn emit_compute_key_method(&self, s: &Struct) -> String {
        let mut out = String::new();
        let indent = self.indent();
        let indent2 = format!("{}    ", indent);

        // Find @key fields
        let key_fields: Vec<std::borrow::Cow<'_, str>> = s
            .fields
            .iter()
            .filter(|f| f.annotations.iter().any(|a| matches!(a, Annotation::Key)))
            .map(|f| python_ident(&f.name))
            .collect();

        let has_key = !key_fields.is_empty();

        push_fmt(
            &mut out,
            format_args!("{}def compute_key(self) -> bytes:\n", indent),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}\"\"\"Compute instance key hash from @key fields (FNV-1a, 16 bytes).\"\"\"\n",
                indent2
            ),
        );

        if has_key {
            push_fmt(
                &mut out,
                format_args!("{}# FNV-1a hash of @key fields\n", indent2),
            );
            push_fmt(
                &mut out,
                format_args!("{}hash_val = 14695981039346656037\n", indent2),
            );
            push_fmt(&mut out, format_args!("{}prime = 1099511628211\n", indent2));

            for field in &key_fields {
                push_fmt(
                    &mut out,
                    format_args!("{}# Hash @key field: {}\n", indent2, field),
                );
                push_fmt(&mut out, format_args!("{}val = self.{}\n", indent2, field));
                push_fmt(
                    &mut out,
                    format_args!("{}if isinstance(val, int):\n", indent2),
                );
                push_fmt(
                    &mut out,
                    format_args!(
                        "{}    for b in val.to_bytes(8, 'little', signed=val < 0):\n",
                        indent2
                    ),
                );
                push_fmt(&mut out, format_args!("{}        hash_val ^= b\n", indent2));
                push_fmt(
                    &mut out,
                    format_args!(
                        "{}        hash_val = (hash_val * prime) & 0xFFFFFFFFFFFFFFFF\n",
                        indent2
                    ),
                );
                push_fmt(
                    &mut out,
                    format_args!("{}elif isinstance(val, (bytes, bytearray)):\n", indent2),
                );
                push_fmt(&mut out, format_args!("{}    for b in val:\n", indent2));
                push_fmt(&mut out, format_args!("{}        hash_val ^= b\n", indent2));
                push_fmt(
                    &mut out,
                    format_args!(
                        "{}        hash_val = (hash_val * prime) & 0xFFFFFFFFFFFFFFFF\n",
                        indent2
                    ),
                );
                push_fmt(
                    &mut out,
                    format_args!("{}elif isinstance(val, str):\n", indent2),
                );
                push_fmt(
                    &mut out,
                    format_args!("{}    for b in val.encode('utf-8'):\n", indent2),
                );
                push_fmt(&mut out, format_args!("{}        hash_val ^= b\n", indent2));
                push_fmt(
                    &mut out,
                    format_args!(
                        "{}        hash_val = (hash_val * prime) & 0xFFFFFFFFFFFFFFFF\n",
                        indent2
                    ),
                );
                push_fmt(
                    &mut out,
                    format_args!("{}elif isinstance(val, float):\n", indent2),
                );
                push_fmt(
                    &mut out,
                    format_args!("{}    import struct as st\n", indent2),
                );
                push_fmt(
                    &mut out,
                    format_args!("{}    for b in st.pack('<d', val):\n", indent2),
                );
                push_fmt(&mut out, format_args!("{}        hash_val ^= b\n", indent2));
                push_fmt(
                    &mut out,
                    format_args!(
                        "{}        hash_val = (hash_val * prime) & 0xFFFFFFFFFFFFFFFF\n",
                        indent2
                    ),
                );
            }

            push_fmt(&mut out, format_args!("{}# Expand to 16 bytes\n", indent2));
            push_fmt(
                &mut out,
                format_args!("{}key = hash_val.to_bytes(8, 'little')\n", indent2),
            );
            push_fmt(
                &mut out,
                format_args!(
                    "{}hash_val = (hash_val * prime) & 0xFFFFFFFFFFFFFFFF\n",
                    indent2
                ),
            );
            push_fmt(
                &mut out,
                format_args!("{}key += hash_val.to_bytes(8, 'little')\n", indent2),
            );
            push_fmt(&mut out, format_args!("{}return key\n", indent2));
        } else {
            push_fmt(
                &mut out,
                format_args!("{}# No @key fields - return zeroed hash\n", indent2),
            );
            push_fmt(&mut out, format_args!("{}return bytes(16)\n", indent2));
        }
        out
    }

    /// Generate `has_key()` classmethod
    fn emit_has_key_method(&self, s: &Struct) -> String {
        let mut out = String::new();
        let indent = self.indent();
        let indent2 = format!("{}    ", indent);

        let has_key = s
            .fields
            .iter()
            .any(|f| f.annotations.iter().any(|a| matches!(a, Annotation::Key)));

        push_fmt(&mut out, format_args!("{}@classmethod\n", indent));
        push_fmt(
            &mut out,
            format_args!("{}def has_key(cls) -> bool:\n", indent),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}\"\"\"Return True if this type has @key fields.\"\"\"\n",
                indent2
            ),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}return {}\n",
                indent2,
                if has_key { "True" } else { "False" }
            ),
        );
        out
    }

    fn emit_enum(&self, e: &Enum) -> String {
        let mut out = String::new();
        push_fmt(&mut out, format_args!("class {}(IntEnum):\n", e.name));
        if e.variants.is_empty() {
            push_fmt(&mut out, format_args!("{}pass\n\n", self.indent()));
            return out;
        }
        // IDL allows explicit values; otherwise start at 0 and increment
        let mut current: i64 = 0;
        for v in &e.variants {
            if let Some(val) = v.value {
                current = val;
            }
            push_fmt(
                &mut out,
                format_args!("{}{} = {}\n", self.indent(), v.name, current),
            );
            current = current.saturating_add(1);
        }
        out.push('\n');
        out
    }

    fn emit_typedef(t: &Typedef) -> String {
        let mut out = String::new();
        push_fmt(
            &mut out,
            format_args!("{} = {}\n\n", t.name, Self::type_to_python(&t.base_type)),
        );
        out
    }

    fn emit_const(c: &Const) -> String {
        // Best-effort: emit textual value as-is
        format!("{} = {}\n\n", c.name, c.value)
    }

    /// Qualify a union case label for Python.
    /// Numeric literals pass through; enum variant names get qualified as `EnumType.VARIANT`.
    fn label_to_python(disc: &IdlType, label: &str) -> String {
        let trimmed = label.trim();
        // If it's a numeric literal, return as-is
        if trimmed
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || c == '-')
        {
            return trimmed.to_string();
        }
        // For named types (enums), qualify with EnumType.VARIANT
        if let IdlType::Named(enum_name) = disc {
            if trimmed.contains('.') {
                trimmed.to_string()
            } else {
                let enum_ident = Self::last_ident(enum_name);
                format!("{enum_ident}.{trimmed}")
            }
        } else {
            trimmed.to_string()
        }
    }

    fn emit_union(&self, u: &Union, idx: &DefinitionIndex) -> String {
        use std::collections::HashSet;
        let mut out = String::new();
        // Collect unique case types
        let mut types: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for c in &u.cases {
            let t = Self::type_to_python(&c.field.field_type);
            if seen.insert(t.clone()) {
                types.push(t);
            }
        }
        let disc_ty = Self::type_to_python(&u.discriminator);
        let val_ann = if types.is_empty() {
            "object".to_string()
        } else if types.len() == 1 {
            types[0].clone()
        } else {
            format!("UnionType[{}]", types.join(", "))
        };
        push_fmt(&mut out, format_args!("@dataclass\nclass {}:\n", u.name));
        push_fmt(
            &mut out,
            format_args!("{}\"\"\"Tagged union for {}\"\"\"\n", self.indent(), u.name),
        );
        // Compute default discriminator value
        #[allow(clippy::option_if_let_else)]
        let disc_default = if let IdlType::Named(nm) = &u.discriminator {
            let enum_ident = Self::last_ident(nm);
            if let Some(e) = idx.enums.get(&enum_ident) {
                e.variants.first().map_or_else(
                    || " = 0".to_string(),
                    |first| format!(" = {enum_ident}.{}", first.name),
                )
            } else {
                " = 0".to_string()
            }
        } else {
            " = 0".to_string()
        };
        push_fmt(
            &mut out,
            format_args!("{}_discriminator: {}{}\n", self.indent(), disc_ty, disc_default),
        );
        push_fmt(
            &mut out,
            format_args!("{}_value: {} = None\n\n", self.indent(), val_ann),
        );

        // Emit simple properties per case (by field name)
        for c in &u.cases {
            let fname = python_ident(&c.field.name);
            let ann = Self::type_to_python(&c.field.field_type);
            push_fmt(
                &mut out,
                format_args!(
                    "{}@property\n{}def {}(self) -> Optional[{}]:\n",
                    self.indent(),
                    self.indent(),
                    fname,
                    ann
                ),
            );
            // Build discriminator check: if any label == value
            if c.labels.is_empty() {
                push_fmt(
                    &mut out,
                    format_args!("{}    return None\n\n", self.indent()),
                );
            } else {
                let conds: Vec<String> = c
                    .labels
                    .iter()
                    .map(|l| match l {
                        crate::ast::UnionLabel::Value(v) => {
                            let py_label = Self::label_to_python(&u.discriminator, v);
                            format!("self._discriminator == {py_label}")
                        }
                        crate::ast::UnionLabel::Default => "True".to_string(),
                    })
                    .collect();
                push_fmt(
                    &mut out,
                    format_args!(
                        "{}    return self._value if ({}) else None\n\n",
                        self.indent(),
                        conds.join(" or ")
                    ),
                );
            }
        }

        // Generate CDR2 encode method
        out.push_str(&self.emit_union_encode_method(u, idx));
        out.push('\n');

        // Generate CDR2 decode method
        out.push_str(&self.emit_union_decode_method(u, idx));
        out.push('\n');

        out
    }

    fn emit_union_encode_method(&self, u: &Union, idx: &DefinitionIndex) -> String {
        let mut out = String::new();
        let indent = self.indent();
        let indent2 = format!("{}    ", indent);

        push_fmt(
            &mut out,
            format_args!("{}def encode_cdr2_le(self) -> bytes:\n", indent),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}\"\"\"Encode to CDR2 little-endian format.\"\"\"\n",
                indent2
            ),
        );
        push_fmt(
            &mut out,
            format_args!("{}parts: List[bytes] = []\n", indent2),
        );
        push_fmt(&mut out, format_args!("{}offset = 0\n", indent2));

        // Encode discriminator as i32
        push_fmt(
            &mut out,
            format_args!("{}# Encode discriminator as i32\n", indent2),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}parts.append(struct.pack('<i', int(self._discriminator)))\n",
                indent2
            ),
        );
        push_fmt(&mut out, format_args!("{}offset += 4\n", indent2));

        // Encode value based on discriminator
        push_fmt(
            &mut out,
            format_args!("{}# Encode value based on discriminator\n", indent2),
        );

        let mut first = true;
        for c in &u.cases {
            let is_default = c
                .labels
                .iter()
                .any(|l| matches!(l, crate::ast::UnionLabel::Default));

            if is_default {
                push_fmt(&mut out, format_args!("{}else:\n", indent2));
            } else {
                let conds: Vec<String> = c
                    .labels
                    .iter()
                    .filter_map(|l| match l {
                        crate::ast::UnionLabel::Value(v) => {
                            let py_label = Self::label_to_python(&u.discriminator, v);
                            Some(format!("self._discriminator == {py_label}"))
                        }
                        crate::ast::UnionLabel::Default => None,
                    })
                    .collect();
                if conds.is_empty() {
                    continue;
                }
                let keyword = if first { "if" } else { "elif" };
                first = false;
                push_fmt(
                    &mut out,
                    format_args!("{}{} {}:\n", indent2, keyword, conds.join(" or ")),
                );
            }

            // Encode the value
            let indent3 = format!("{}    ", indent2);
            out.push_str(&Self::emit_union_encode_value(
                &c.field.field_type,
                idx,
                &indent3,
            ));
        }

        push_fmt(
            &mut out,
            format_args!("{}return b''.join(parts)\n", indent2),
        );
        out
    }

    #[allow(clippy::too_many_lines)]
    fn emit_union_encode_value(ty: &IdlType, idx: &DefinitionIndex, indent: &str) -> String {
        let mut out = String::new();
        match ty {
            IdlType::Primitive(p) => {
                if let Some((fmt, size)) = Self::primitive_to_struct_format(p) {
                    let align = size.max(1);
                    push_fmt(
                        &mut out,
                        format_args!("{indent}pad = ({align} - (offset % {align})) % {align}\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}parts.append(b'\\x00' * pad)\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += pad\n"));
                    push_fmt(
                        &mut out,
                        format_args!("{indent}parts.append(struct.pack('{fmt}', self._value))\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += {size}\n"));
                } else if matches!(p, PrimitiveType::String) {
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}# string: align to 4, write length (with NUL), then bytes\n"
                        ),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}pad = (4 - (offset % 4)) % 4\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}parts.append(b'\\x00' * pad)\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += pad\n"));
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_bytes = self._value.encode('utf-8') + b'\\x00'\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}parts.append(struct.pack('<I', len(_bytes)))\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                    push_fmt(&mut out, format_args!("{indent}parts.append(_bytes)\n"));
                    push_fmt(&mut out, format_args!("{indent}offset += len(_bytes)\n"));
                }
            }
            IdlType::Named(nm) => {
                let type_name = Self::last_ident(nm);
                if idx.structs.contains_key(&type_name) {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}# nested struct {type_name}\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_sub = self._value.encode_cdr2_le()\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}parts.append(_sub)\n"));
                    push_fmt(&mut out, format_args!("{indent}offset += len(_sub)\n"));
                } else if idx.enums.contains_key(&type_name) {
                    push_fmt(&mut out, format_args!("{indent}# enum as int32\n"));
                    push_fmt(
                        &mut out,
                        format_args!("{indent}pad = (4 - (offset % 4)) % 4\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}parts.append(b'\\x00' * pad)\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += pad\n"));
                    push_fmt(
                        &mut out,
                        format_args!("{indent}parts.append(struct.pack('<i', int(self._value)))\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                } else if let Some(td) = idx.typedefs.get(&type_name) {
                    out.push_str(&Self::emit_union_encode_value(&td.base_type, idx, indent));
                }
            }
            IdlType::Sequence { .. } | IdlType::Array { .. } | IdlType::Map { .. } => {
                push_fmt(
                    &mut out,
                    format_args!("{indent}# complex type: delegate to nested encode\n"),
                );
                push_fmt(
                    &mut out,
                    format_args!("{indent}_sub = self._value.encode_cdr2_le()\n"),
                );
                push_fmt(&mut out, format_args!("{indent}parts.append(_sub)\n"));
                push_fmt(&mut out, format_args!("{indent}offset += len(_sub)\n"));
            }
        }
        out
    }

    fn emit_union_decode_method(&self, u: &Union, idx: &DefinitionIndex) -> String {
        let mut out = String::new();
        let indent = self.indent();
        let indent2 = format!("{}    ", indent);

        push_fmt(&mut out, format_args!("{}@classmethod\n", indent));
        push_fmt(
            &mut out,
            format_args!(
                "{}def decode_cdr2_le(cls, data: bytes) -> Tuple['{}', int]:\n",
                indent, u.name
            ),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}\"\"\"Decode from CDR2 little-endian format. Returns (instance, bytes_read).\"\"\"\n",
                indent2
            ),
        );
        push_fmt(&mut out, format_args!("{}offset = 0\n", indent2));

        // Decode discriminator as i32
        push_fmt(
            &mut out,
            format_args!("{}# Decode discriminator as i32\n", indent2),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}_discriminator, = struct.unpack_from('<i', data, offset)\n",
                indent2
            ),
        );
        push_fmt(&mut out, format_args!("{}offset += 4\n", indent2));

        // Decode value based on discriminator
        push_fmt(
            &mut out,
            format_args!("{}# Decode value based on discriminator\n", indent2),
        );

        let mut first = true;
        for c in &u.cases {
            let is_default = c
                .labels
                .iter()
                .any(|l| matches!(l, crate::ast::UnionLabel::Default));

            if is_default {
                push_fmt(&mut out, format_args!("{}else:\n", indent2));
            } else {
                let conds: Vec<String> = c
                    .labels
                    .iter()
                    .filter_map(|l| match l {
                        crate::ast::UnionLabel::Value(v) => {
                            let py_label = Self::label_to_python(&u.discriminator, v);
                            Some(format!("_discriminator == {py_label}"))
                        }
                        crate::ast::UnionLabel::Default => None,
                    })
                    .collect();
                if conds.is_empty() {
                    continue;
                }
                let keyword = if first { "if" } else { "elif" };
                first = false;
                push_fmt(
                    &mut out,
                    format_args!("{}{} {}:\n", indent2, keyword, conds.join(" or ")),
                );
            }

            // Decode the value
            let indent3 = format!("{}    ", indent2);
            out.push_str(&Self::emit_union_decode_value(
                &c.field.field_type,
                idx,
                &indent3,
            ));
        }

        push_fmt(
            &mut out,
            format_args!(
                "{}return cls(_discriminator=_discriminator, _value=_value), offset\n",
                indent2
            ),
        );
        out
    }

    fn emit_union_decode_value(ty: &IdlType, idx: &DefinitionIndex, indent: &str) -> String {
        let mut out = String::new();
        match ty {
            IdlType::Primitive(p) => {
                if let Some((fmt, size)) = Self::primitive_to_struct_format(p) {
                    let align = size.max(1);
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}offset += ({align} - (offset % {align})) % {align}\n"
                        ),
                    );
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}_value, = struct.unpack_from('{fmt}', data, offset)\n"
                        ),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += {size}\n"));
                } else if matches!(p, PrimitiveType::String) {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}# string: align, read length, then bytes\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}offset += (4 - (offset % 4)) % 4\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_len, = struct.unpack_from('<I', data, offset)\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}_value = data[offset:offset+_len-1].decode('utf-8')\n"
                        ),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += _len\n"));
                } else {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_value = None  # unsupported primitive\n"),
                    );
                }
            }
            IdlType::Named(nm) => {
                let type_name = Self::last_ident(nm);
                if idx.structs.contains_key(&type_name) {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}# nested struct {type_name}\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!(
                            "{indent}_value, _read = {type_name}.decode_cdr2_le(data[offset:])\n"
                        ),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += _read\n"));
                } else if idx.enums.contains_key(&type_name) {
                    push_fmt(&mut out, format_args!("{indent}# enum as int32\n"));
                    push_fmt(
                        &mut out,
                        format_args!("{indent}offset += (4 - (offset % 4)) % 4\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_val, = struct.unpack_from('<i', data, offset)\n"),
                    );
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_value = {type_name}(_val)\n"),
                    );
                    push_fmt(&mut out, format_args!("{indent}offset += 4\n"));
                } else if let Some(td) = idx.typedefs.get(&type_name) {
                    out.push_str(&Self::emit_union_decode_value(&td.base_type, idx, indent));
                } else {
                    push_fmt(
                        &mut out,
                        format_args!("{indent}_value = None  # unknown type {type_name}\n"),
                    );
                }
            }
            IdlType::Sequence { .. } | IdlType::Array { .. } | IdlType::Map { .. } => {
                push_fmt(
                    &mut out,
                    format_args!(
                        "{indent}_value = None  # complex types not supported in union decode\n"
                    ),
                );
            }
        }
        out
    }

    #[allow(clippy::too_many_lines)]
    fn emit_bitset(&self, b: &Bitset) -> String {
        let mut out = String::new();
        let indent = self.indent();

        // Class declaration
        push_fmt(&mut out, format_args!("@dataclass\nclass {}:\n", b.name));
        push_fmt(
            &mut out,
            format_args!(
                "{}\"\"\"Bitset {} - stores packed bit fields in a 64-bit integer.\"\"\"\n",
                indent, b.name
            ),
        );
        push_fmt(&mut out, format_args!("{}bits: int = 0\n\n", indent));

        // Compute field positions
        let mut next_pos: u32 = 0;
        for f in &b.fields {
            let mut pos = None;
            for ann in &f.annotations {
                if let Annotation::Position(p) = ann {
                    pos = Some(*p);
                    break;
                }
            }
            let start = pos.unwrap_or_else(|| {
                let p = next_pos;
                next_pos += f.width;
                p
            });
            let width = f.width;
            let mask = (1u64 << width) - 1;

            let field_id = python_ident(&f.name);
            // Getter property
            push_fmt(&mut out, format_args!("{}@property\n", indent));
            push_fmt(
                &mut out,
                format_args!("{}def {}(self) -> int:\n", indent, field_id),
            );
            push_fmt(
                &mut out,
                format_args!(
                    "{}    \"\"\"Get {} (width={}, position={}).\"\"\"\n",
                    indent, f.name, width, start
                ),
            );
            push_fmt(
                &mut out,
                format_args!(
                    "{}    return (self.bits >> {}) & {}\n\n",
                    indent, start, mask
                ),
            );

            // Setter property
            push_fmt(&mut out, format_args!("{}@{}.setter\n", indent, field_id));
            push_fmt(
                &mut out,
                format_args!("{}def {}(self, v: int) -> None:\n", indent, field_id),
            );
            push_fmt(
                &mut out,
                format_args!("{}    mask = {} << {}\n", indent, mask, start),
            );
            push_fmt(
                &mut out,
                format_args!(
                    "{}    self.bits = (self.bits & ~mask) | ((v & {}) << {})\n\n",
                    indent, mask, start
                ),
            );
        }

        // CDR2 encode method
        push_fmt(
            &mut out,
            format_args!("{}def encode_cdr2_le(self) -> bytes:\n", indent),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}    \"\"\"Encode to CDR2 little-endian format (8 bytes).\"\"\"\n",
                indent
            ),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}    return struct.pack('<Q', self.bits & 0xFFFFFFFFFFFFFFFF)\n\n",
                indent
            ),
        );

        // CDR2 decode method
        push_fmt(&mut out, format_args!("{}@classmethod\n", indent));
        push_fmt(
            &mut out,
            format_args!(
                "{}def decode_cdr2_le(cls, data: bytes) -> Tuple['{}', int]:\n",
                indent, b.name
            ),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}    \"\"\"Decode from CDR2 little-endian format. Returns (instance, bytes_read).\"\"\"\n",
                indent
            ),
        );
        push_fmt(&mut out, format_args!("{}    # Align to 8 bytes\n", indent));
        push_fmt(
            &mut out,
            format_args!("{}    bits, = struct.unpack_from('<Q', data, 0)\n", indent),
        );
        push_fmt(
            &mut out,
            format_args!("{}    return cls(bits=bits), 8\n\n", indent),
        );

        out
    }

    #[allow(clippy::too_many_lines)]
    fn emit_bitmask(&self, m: &Bitmask) -> String {
        let mut out = String::new();
        let indent = self.indent();

        // Generate a class that wraps an int with flag constants
        push_fmt(&mut out, format_args!("class {}(int):\n", m.name));
        push_fmt(
            &mut out,
            format_args!(
                "{}\"\"\"Bitmask {} - integer with named flag constants.\"\"\"\n\n",
                indent, m.name
            ),
        );

        // Compute flag positions and emit constants
        let mut next_pos: u32 = 0;
        for flag in &m.flags {
            let mut pos = None;
            for ann in &flag.annotations {
                if let Annotation::Position(p) = ann {
                    pos = Some(*p);
                    break;
                }
            }
            let bit = pos.unwrap_or_else(|| {
                let p = next_pos;
                next_pos += 1;
                p
            });
            push_fmt(
                &mut out,
                format_args!("{}{} = 1 << {}\n", indent, flag.name.to_uppercase(), bit),
            );
        }
        out.push('\n');

        // Constructor to ensure proper type
        push_fmt(
            &mut out,
            format_args!("{}def __new__(cls, value: int = 0):\n", indent),
        );
        push_fmt(
            &mut out,
            format_args!("{}    return super().__new__(cls, value)\n\n", indent),
        );

        // Bitwise operations that return the same type
        push_fmt(
            &mut out,
            format_args!("{}def __or__(self, other: int) -> '{}':\n", indent, m.name),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}    return {}(int.__or__(self, other))\n\n",
                indent, m.name
            ),
        );

        push_fmt(
            &mut out,
            format_args!("{}def __and__(self, other: int) -> '{}':\n", indent, m.name),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}    return {}(int.__and__(self, other))\n\n",
                indent, m.name
            ),
        );

        push_fmt(
            &mut out,
            format_args!("{}def __xor__(self, other: int) -> '{}':\n", indent, m.name),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}    return {}(int.__xor__(self, other))\n\n",
                indent, m.name
            ),
        );

        push_fmt(
            &mut out,
            format_args!("{}def __invert__(self) -> '{}':\n", indent, m.name),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}    return {}(int.__invert__(self) & 0xFFFFFFFFFFFFFFFF)\n\n",
                indent, m.name
            ),
        );

        // CDR2 encode method
        push_fmt(
            &mut out,
            format_args!("{}def encode_cdr2_le(self) -> bytes:\n", indent),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}    \"\"\"Encode to CDR2 little-endian format (8 bytes).\"\"\"\n",
                indent
            ),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}    return struct.pack('<Q', int(self) & 0xFFFFFFFFFFFFFFFF)\n\n",
                indent
            ),
        );

        // CDR2 decode classmethod
        push_fmt(&mut out, format_args!("{}@classmethod\n", indent));
        push_fmt(
            &mut out,
            format_args!(
                "{}def decode_cdr2_le(cls, data: bytes) -> Tuple['{}', int]:\n",
                indent, m.name
            ),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}    \"\"\"Decode from CDR2 little-endian format. Returns (instance, bytes_read).\"\"\"\n",
                indent
            ),
        );
        push_fmt(
            &mut out,
            format_args!("{}    val, = struct.unpack_from('<Q', data, 0)\n", indent),
        );
        push_fmt(
            &mut out,
            format_args!("{}    return cls(val), 8\n\n", indent),
        );

        out
    }

    fn emit_definitions(&self, defs: &[Definition], idx: &DefinitionIndex, out: &mut String) {
        for def in defs {
            match def {
                Definition::Module(module) => self.emit_definitions(&module.definitions, idx, out),
                Definition::Struct(s) => out.push_str(&self.emit_struct(s, idx)),
                Definition::Enum(e) => out.push_str(&self.emit_enum(e)),
                Definition::Typedef(t) => out.push_str(&Self::emit_typedef(t)),
                Definition::Const(c) => out.push_str(&Self::emit_const(c)),
                Definition::Union(u) => out.push_str(&self.emit_union(u, idx)),
                Definition::Bitset(b) => out.push_str(&self.emit_bitset(b)),
                Definition::Bitmask(m) => out.push_str(&self.emit_bitmask(m)),
                Definition::AnnotationDecl(_) | Definition::ForwardDecl(_) => {}
                #[cfg(feature = "interfaces")]
                Definition::Interface(_) => {}
                #[cfg(feature = "interfaces")]
                Definition::Exception(_) => {}
            }
        }
    }
}

impl Default for PythonGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGenerator for PythonGenerator {
    fn generate(&self, ast: &IdlFile) -> Result<String> {
        let mut out = String::new();
        out.push_str(&Self::emit_header());

        let idx = DefinitionIndex::from_file(ast);
        self.emit_definitions(&ast.definitions, &idx, &mut out);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::pedantic)]
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::ast::{EnumVariant, Field};
    use std::error::Error;

    type TestResult<T> = std::result::Result<T, Box<dyn Error>>;

    #[test]
    fn python_generates_dataclass_and_enum() -> TestResult<()> {
        let mut file = IdlFile::new();
        // enum Color { RED, GREEN, BLUE };
        let mut e = Enum::new("Color");
        e.add_variant(EnumVariant::new("RED", None));
        e.add_variant(EnumVariant::new("GREEN", None));
        e.add_variant(EnumVariant::new("BLUE", None));
        file.add_definition(Definition::Enum(e));

        // struct Point { int32_t x; int32_t y; };
        let mut s = Struct::new("Point");
        s.add_field(Field::new("x", IdlType::Primitive(PrimitiveType::Int32)));
        s.add_field(Field::new("y", IdlType::Primitive(PrimitiveType::Int32)));
        file.add_definition(Definition::Struct(s));

        // struct Path { sequence<Point> points; string name; };
        let mut p = Struct::new("Path");
        p.add_field(Field::new(
            "points",
            IdlType::Sequence {
                inner: Box::new(IdlType::Named("Point".into())),
                bound: None,
            },
        ));
        p.add_field(Field::new(
            "name",
            IdlType::Primitive(PrimitiveType::String),
        ));
        file.add_definition(Definition::Struct(p));

        let pyg = PythonGenerator::new();
        let code = pyg.generate(&file)?;
        assert!(code.contains("from dataclasses import dataclass, field"));
        assert!(code.contains("from enum import IntEnum"));
        assert!(code.contains("class Color(IntEnum):"));
        assert!(code.contains("RED = 0"));
        assert!(code.contains("@dataclass\nclass Point:"));
        assert!(code.contains("x: int"));
        assert!(code.contains("y: int"));
        assert!(code.contains("@dataclass\nclass Path:"));
        assert!(code.contains("points: List[Point] = field(default_factory=list)"));
        assert!(code.contains("name: str = \"\""));
        Ok(())
    }

    #[test]
    fn python_generates_union() -> TestResult<()> {
        let mut file = IdlFile::new();
        let mut u = Union::new("Data", IdlType::Primitive(PrimitiveType::Int32));
        u.add_case(crate::ast::UnionCase {
            labels: vec![crate::ast::UnionLabel::Value("1".into())],
            field: crate::ast::Field::new(
                "integer_value",
                IdlType::Primitive(PrimitiveType::Int32),
            ),
        });
        u.add_case(crate::ast::UnionCase {
            labels: vec![crate::ast::UnionLabel::Value("2".into())],
            field: crate::ast::Field::new(
                "string_value",
                IdlType::Primitive(PrimitiveType::String),
            ),
        });
        file.add_definition(Definition::Union(u));
        let pyg = super::PythonGenerator::new();
        let code = pyg.generate(&file)?;
        assert!(code.contains("class Data:"));
        assert!(code.contains("_discriminator: int = 0"));
        assert!(code.contains("_value:"));
        assert!(code.contains("def integer_value(self) -> Optional[int]"));
        assert!(code.contains("def string_value(self) -> Optional[str]"));
        // CDR2 encode method
        assert!(code.contains("def encode_cdr2_le(self) -> bytes:"));
        assert!(code.contains("# Encode discriminator as i32"));
        assert!(code.contains("struct.pack('<i', int(self._discriminator))"));
        assert!(code.contains("if self._discriminator == 1:"));
        assert!(code.contains("elif self._discriminator == 2:"));
        // CDR2 decode method
        assert!(code.contains("def decode_cdr2_le(cls, data: bytes) -> Tuple['Data', int]:"));
        assert!(code.contains("# Decode discriminator as i32"));
        assert!(code.contains("_discriminator, = struct.unpack_from('<i', data, offset)"));
        assert!(code.contains("if _discriminator == 1:"));
        assert!(code.contains("elif _discriminator == 2:"));
        assert!(code.contains("return cls(_discriminator=_discriminator, _value=_value), offset"));
        Ok(())
    }

    #[test]
    fn python_generates_map() -> TestResult<()> {
        let mut file = IdlFile::new();
        // struct Config { map<string, int32> settings; };
        let mut s = Struct::new("Config");
        s.add_field(Field::new(
            "settings",
            IdlType::Map {
                key: Box::new(IdlType::Primitive(PrimitiveType::String)),
                value: Box::new(IdlType::Primitive(PrimitiveType::Int32)),
                bound: None,
            },
        ));
        file.add_definition(Definition::Struct(s));

        let pyg = PythonGenerator::new();
        let code = pyg.generate(&file)?;
        assert!(code.contains("settings: Dict[str, int] = field(default_factory=dict)"));
        // Map encoding should include length and loop
        assert!(code.contains("# map: align to 4, write length, then key-value pairs"));
        assert!(code.contains("for _key, _val in self.settings.items():"));
        // Map decoding should include length and loop
        assert!(code.contains("# map: align, read length, then key-value pairs"));
        assert!(code.contains("for _ in range(_map_len):"));
        Ok(())
    }

    #[test]
    fn python_generates_bitset() -> TestResult<()> {
        use crate::ast::BitfieldDecl;
        let mut file = IdlFile::new();
        // bitset Flags { bitfield<3> level; bitfield<1> active; };
        let mut b = Bitset::new("Flags");
        b.add_field(BitfieldDecl::new(3, "level"));
        b.add_field(BitfieldDecl::new(1, "active"));
        file.add_definition(Definition::Bitset(b));

        let pyg = PythonGenerator::new();
        let code = pyg.generate(&file)?;
        // Should have dataclass with bits field
        assert!(code.contains("@dataclass\nclass Flags:"));
        assert!(code.contains("bits: int = 0"));
        // Should have getter properties
        assert!(code.contains("@property"));
        assert!(code.contains("def level(self) -> int:"));
        assert!(code.contains("def active(self) -> int:"));
        // Should have setter properties
        assert!(code.contains("@level.setter"));
        assert!(code.contains("@active.setter"));
        // Should have CDR2 methods
        assert!(code.contains("def encode_cdr2_le(self) -> bytes:"));
        assert!(code.contains("def decode_cdr2_le(cls, data: bytes)"));
        Ok(())
    }

    #[test]
    fn python_generates_bitmask() -> TestResult<()> {
        use crate::ast::BitmaskFlag;
        let mut file = IdlFile::new();
        // bitmask Permissions { FLAG_READ; FLAG_WRITE; FLAG_EXEC; };
        let mut m = Bitmask::new("Permissions");
        m.add_flag(BitmaskFlag::new("FLAG_READ"));
        m.add_flag(BitmaskFlag::new("FLAG_WRITE"));
        m.add_flag(BitmaskFlag::new("FLAG_EXEC"));
        file.add_definition(Definition::Bitmask(m));

        let pyg = PythonGenerator::new();
        let code = pyg.generate(&file)?;
        // Should be a class inheriting from int
        assert!(code.contains("class Permissions(int):"));
        // Should have flag constants
        assert!(code.contains("FLAG_READ = 1 << 0"));
        assert!(code.contains("FLAG_WRITE = 1 << 1"));
        assert!(code.contains("FLAG_EXEC = 1 << 2"));
        // Should have bitwise operators
        assert!(code.contains("def __or__(self, other: int)"));
        assert!(code.contains("def __and__(self, other: int)"));
        assert!(code.contains("def __xor__(self, other: int)"));
        assert!(code.contains("def __invert__(self)"));
        // Should have CDR2 methods
        assert!(code.contains("def encode_cdr2_le(self) -> bytes:"));
        assert!(code.contains("def decode_cdr2_le(cls, data: bytes)"));
        Ok(())
    }

    #[test]
    fn python_generates_appendable_struct() -> TestResult<()> {
        let mut file = IdlFile::new();
        // @appendable struct Message { int32 id; string text; };
        let mut s = Struct::new("Message");
        s.extensibility = Some(ExtensibilityKind::Appendable);
        s.add_field(Field::new("id", IdlType::Primitive(PrimitiveType::Int32)));
        s.add_field(Field::new(
            "text",
            IdlType::Primitive(PrimitiveType::String),
        ));
        file.add_definition(Definition::Struct(s));

        let pyg = PythonGenerator::new();
        let code = pyg.generate(&file)?;

        // Check APPENDABLE encode: has DHEADER
        assert!(
            code.contains("APPENDABLE extensibility"),
            "Should document APPENDABLE"
        );
        assert!(
            code.contains("offset = 4  # Reserve space for DHEADER"),
            "Should reserve DHEADER space"
        );
        assert!(
            code.contains("dheader = struct.pack('<I', len(payload))"),
            "Should write DHEADER"
        );
        assert!(
            code.contains("return dheader + payload"),
            "Should prepend DHEADER"
        );

        // Check APPENDABLE decode: reads DHEADER
        assert!(
            code.contains("# Read DHEADER (payload size)"),
            "Should read DHEADER"
        );
        assert!(
            code.contains("_dheader, = struct.unpack_from('<I', data, offset)"),
            "Should unpack DHEADER"
        );
        assert!(
            code.contains("_payload_end = offset + _dheader"),
            "Should track payload end"
        );
        assert!(
            code.contains("), _payload_end"),
            "Should return payload_end as bytes_read"
        );
        Ok(())
    }

    #[test]
    fn python_generates_mutable_struct() -> TestResult<()> {
        let mut file = IdlFile::new();
        // @mutable struct Sensor { int32 id; float value; };
        let mut s = Struct::new("Sensor");
        s.extensibility = Some(ExtensibilityKind::Mutable);
        s.add_field(Field::new("id", IdlType::Primitive(PrimitiveType::Int32)));
        s.add_field(Field::new(
            "value",
            IdlType::Primitive(PrimitiveType::Float),
        ));
        file.add_definition(Definition::Struct(s));

        let pyg = PythonGenerator::new();
        let code = pyg.generate(&file)?;

        // Check MUTABLE encode: has DHEADER + EMHEADER
        assert!(
            code.contains("MUTABLE extensibility"),
            "Should document MUTABLE"
        );
        assert!(
            code.contains("offset = 4  # Reserve space for DHEADER"),
            "Should reserve DHEADER space"
        );
        assert!(code.contains("emheader = ("), "Should compute EMHEADER");
        assert!(code.contains("<< 28) | ("), "Should use LC in EMHEADER");
        assert!(code.contains("& 0x0FFFFFFF)"), "Should mask member_id");
        assert!(
            code.contains("parts.append(struct.pack('<I', emheader))"),
            "Should encode EMHEADER"
        );
        assert!(
            code.contains("dheader = struct.pack('<I', len(payload))"),
            "Should write DHEADER"
        );

        // Check MUTABLE decode: reads EMHEADER per field
        assert!(
            code.contains("while offset < _payload_end:"),
            "Should loop through members"
        );
        assert!(
            code.contains("_emheader, = struct.unpack_from('<I', data, offset)"),
            "Should read EMHEADER"
        );
        assert!(
            code.contains("_lc = (_emheader >> 28)"),
            "Should extract LC"
        );
        assert!(
            code.contains("_member_id = _emheader & 0x0FFFFFFF"),
            "Should extract member_id"
        );
        assert!(code.contains("if _lc == 0:"), "Should handle LC=0");
        assert!(
            code.contains("elif _lc == 5:  # NEXTINT"),
            "Should handle LC=5 NEXTINT"
        );
        assert!(
            code.contains("_member_size, = struct.unpack_from('<I', data, offset)"),
            "Should read NEXTINT"
        );
        Ok(())
    }

    #[test]
    fn python_generates_mutable_with_id_annotation() -> TestResult<()> {
        let mut file = IdlFile::new();
        // @mutable struct Point { @id(100) int32 x; @id(200) int32 y; };
        let mut s = Struct::new("Point");
        s.extensibility = Some(ExtensibilityKind::Mutable);
        s.add_field(
            Field::new("x", IdlType::Primitive(PrimitiveType::Int32))
                .with_annotation(Annotation::Id(100)),
        );
        s.add_field(
            Field::new("y", IdlType::Primitive(PrimitiveType::Int32))
                .with_annotation(Annotation::Id(200)),
        );
        file.add_definition(Definition::Struct(s));

        let pyg = PythonGenerator::new();
        let code = pyg.generate(&file)?;

        // Check that explicit @id values are used
        assert!(
            code.contains("member_id=0x00000064"),
            "Should use @id(100) = 0x64"
        );
        assert!(
            code.contains("member_id=0x000000C8"),
            "Should use @id(200) = 0xC8"
        );
        // Decode should also match these IDs
        assert!(
            code.contains("if _member_id == 0x00000064:  # x"),
            "Should decode by explicit ID for x"
        );
        assert!(
            code.contains("elif _member_id == 0x000000C8:  # y"),
            "Should decode by explicit ID for y"
        );
        Ok(())
    }

    #[test]
    fn python_generates_optional_presence_flags_final() -> TestResult<()> {
        let mut file = IdlFile::new();
        // @final struct OptionalTest { int32 required; @optional int32 opt_value; };
        let mut s = Struct::new("OptionalTest");
        s.extensibility = Some(ExtensibilityKind::Final);
        s.add_field(Field::new(
            "required",
            IdlType::Primitive(PrimitiveType::Int32),
        ));
        s.add_field(
            Field::new("opt_value", IdlType::Primitive(PrimitiveType::Int32))
                .with_annotation(Annotation::Optional),
        );
        file.add_definition(Definition::Struct(s));

        let pyg = PythonGenerator::new();
        let code = pyg.generate(&file)?;

        // Check field type is Optional[int]
        assert!(
            code.contains("opt_value: Optional[int] = None"),
            "Optional field should have Optional type"
        );

        // Check encode: writes presence flag
        assert!(
            code.contains("# @optional field 'opt_value': presence flag"),
            "Should document optional field in encode"
        );
        assert!(
            code.contains("if self.opt_value is None:"),
            "Should check for None in encode"
        );
        assert!(
            code.contains("parts.append(b'\\x00')  # absent"),
            "Should write absent flag"
        );
        assert!(
            code.contains("parts.append(b'\\x01')  # present"),
            "Should write present flag"
        );

        // Check decode: reads presence flag
        assert!(
            code.contains("# @optional field 'opt_value': read presence flag"),
            "Should document optional field in decode"
        );
        assert!(
            code.contains("_has_opt_value = data[offset] != 0"),
            "Should read presence flag"
        );
        assert!(
            code.contains("if _has_opt_value:"),
            "Should check presence flag"
        );
        assert!(
            code.contains("_opt_value = None"),
            "Should set to None when absent"
        );

        Ok(())
    }

    #[test]
    fn python_generates_optional_presence_flags_appendable() -> TestResult<()> {
        let mut file = IdlFile::new();
        // @appendable struct OptionalAppendable { string name; @optional float temp; };
        let mut s = Struct::new("OptionalAppendable");
        s.extensibility = Some(ExtensibilityKind::Appendable);
        s.add_field(Field::new(
            "name",
            IdlType::Primitive(PrimitiveType::String),
        ));
        s.add_field(
            Field::new("temp", IdlType::Primitive(PrimitiveType::Float))
                .with_annotation(Annotation::Optional),
        );
        file.add_definition(Definition::Struct(s));

        let pyg = PythonGenerator::new();
        let code = pyg.generate(&file)?;

        // Check APPENDABLE with optional: has DHEADER + presence flags
        assert!(
            code.contains("APPENDABLE extensibility"),
            "Should document APPENDABLE"
        );
        assert!(
            code.contains("# @optional field 'temp': presence flag"),
            "Should document optional field"
        );
        assert!(
            code.contains("_has_temp = data[offset] != 0"),
            "Should read presence flag for appendable"
        );

        Ok(())
    }

    /// Test that fields with defaults are sorted AFTER fields without defaults.
    /// Python dataclasses require this ordering otherwise you get:
    /// "TypeError: non-default argument follows default argument"
    #[test]
    fn python_dataclass_field_order_defaults_last() -> TestResult<()> {
        let mut file = IdlFile::new();
        // struct Message {
        //     sequence<int32> items;    // default: field(default_factory=list)
        //     SenderType from_type;     // default: 0 (enum)
        //     int32 id;                 // default: 0
        //     string label;             // default: ""
        // };
        //
        // All fields get defaults so IDL order is preserved (critical for CDR).
        // Python dataclass won't complain because every field has a default.

        // First, define an enum
        let mut e = Enum::new("SenderType");
        e.add_variant(EnumVariant::new("USER", None));
        e.add_variant(EnumVariant::new("SYSTEM", None));
        file.add_definition(Definition::Enum(e));

        // Then define the struct with mixed field types
        let mut s = Struct::new("Message");
        // IDL order: sequence, enum, int32, string
        s.add_field(Field::new(
            "items",
            IdlType::Sequence {
                inner: Box::new(IdlType::Primitive(PrimitiveType::Int32)),
                bound: None,
            },
        ));
        s.add_field(Field::new("from_type", IdlType::Named("SenderType".into())));
        s.add_field(Field::new("id", IdlType::Primitive(PrimitiveType::Int32)));
        s.add_field(Field::new(
            "label",
            IdlType::Primitive(PrimitiveType::String),
        ));
        file.add_definition(Definition::Struct(s));

        let pyg = PythonGenerator::new();
        let code = pyg.generate(&file)?;

        // Find the Message class definition
        let class_start = code
            .find("@dataclass\nclass Message:")
            .expect("Message class");
        let class_end = code[class_start..]
            .find("\n    def ")
            .unwrap_or(code.len() - class_start);
        let class_def = &code[class_start..class_start + class_end];

        // IDL field order must be preserved (CDR serialization depends on it)
        let items_pos = class_def.find("items:").expect("items field");
        let from_type_pos = class_def.find("from_type:").expect("from_type field");
        let id_pos = class_def.find("id:").expect("id field");
        let label_pos = class_def.find("label:").expect("label field");

        assert!(
            items_pos < from_type_pos,
            "IDL order: items before from_type"
        );
        assert!(
            from_type_pos < id_pos,
            "IDL order: from_type before id"
        );
        assert!(
            id_pos < label_pos,
            "IDL order: id before label"
        );

        // All fields must have defaults (so dataclass order doesn't matter)
        assert!(
            class_def.contains("items: List[int] = field(default_factory=list)"),
            "items must have default_factory=list"
        );
        assert!(
            class_def.contains("from_type: SenderType = 0"),
            "from_type (enum) must have default = 0"
        );
        assert!(
            class_def.contains("id: int = 0"),
            "id must have default = 0"
        );
        assert!(
            class_def.contains("label: str = \"\""),
            "label must have default = \"\""
        );

        Ok(())
    }
}
