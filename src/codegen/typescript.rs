// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! TypeScript code generator
//!
//! Generates TypeScript interfaces and CDR2 serialization helpers.
//! Output is compatible with both Node.js (Buffer) and browser (Uint8Array).
//!
//! Note: uninlined_format_args allowed here due to extensive format!() usage
//! in code generation that would require significant refactoring.

#![allow(clippy::uninlined_format_args)]

use crate::ast::{
    Bitmask, Bitset, Const, Definition, Enum, Field, IdlFile, Struct, Typedef, Union, UnionLabel,
};
use crate::codegen::CodeGenerator;
use crate::error::Result;
use crate::types::{Annotation, AutoIdKind, ExtensibilityKind, IdlType, PrimitiveType};
use std::fmt::Write;

use super::keywords::ts_ident;

fn push_fmt(dst: &mut String, args: std::fmt::Arguments<'_>) {
    let _ = dst.write_fmt(args);
}

/// Check if a struct is MUTABLE
fn is_mutable(s: &Struct) -> bool {
    matches!(s.extensibility, Some(ExtensibilityKind::Mutable))
        || s.annotations.iter().any(|a| {
            matches!(
                a,
                Annotation::Extensibility(ExtensibilityKind::Mutable) | Annotation::Mutable
            )
        })
}

/// Check if a struct is APPENDABLE
fn is_appendable(s: &Struct) -> bool {
    matches!(s.extensibility, Some(ExtensibilityKind::Appendable))
        || s.annotations.iter().any(|a| {
            matches!(
                a,
                Annotation::Extensibility(ExtensibilityKind::Appendable) | Annotation::Appendable
            )
        })
}

/// Compute Member ID for mutable/appendable structs.
///
/// Priority:
/// - `@id` on the field -> use explicit ID
/// - `@autoid(SEQUENTIAL)` on struct -> use declaration order
/// - default/`@autoid(HASH)` -> FNV-1a 32-bit & `0x0FFF_FFFF` (`XTypes` ss7.3.1.2.1.2)
fn compute_member_id(s: &Struct, idx: usize, field: &Field) -> u32 {
    // Check for explicit @id annotation on the field
    for ann in &field.annotations {
        if let Annotation::Id(id) = ann {
            return *id;
        }
    }

    // Check for @autoid(SEQUENTIAL) on the struct
    let autoid_seq = s
        .annotations
        .iter()
        .any(|a| matches!(a, Annotation::AutoId(AutoIdKind::Sequential)));
    if autoid_seq {
        // @audit-ok: safe cast - field index in struct always << u32::MAX
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

/// Get fixed size for a primitive type (None for variable-size types)
const fn cdr2_fixed_size(ty: &IdlType) -> Option<usize> {
    match ty {
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
            | PrimitiveType::Float
            | PrimitiveType::WChar => Some(4),
            PrimitiveType::LongLong
            | PrimitiveType::UnsignedLongLong
            | PrimitiveType::Int64
            | PrimitiveType::UInt64
            | PrimitiveType::Double
            | PrimitiveType::LongDouble => Some(8),
            PrimitiveType::Fixed { .. } => Some(16),
            PrimitiveType::Void | PrimitiveType::String | PrimitiveType::WString => None,
        },
        IdlType::Sequence { .. }
        | IdlType::Array { .. }
        | IdlType::Map { .. }
        | IdlType::Named(_) => None,
    }
}

/// Compute LC (Length Code) for EMHEADER based on fixed size.
/// LC values: 0=1byte, 1=2bytes, 2=4bytes, 3=8bytes, 5=NEXTINT (variable size)
const fn compute_lc(ty: &IdlType) -> u32 {
    match cdr2_fixed_size(ty) {
        Some(1) => 0,
        Some(2) => 1,
        Some(4) => 2,
        Some(8) => 3,
        _ => 5, // NEXTINT for variable-size or unknown
    }
}

/// TypeScript code generator
pub struct TypeScriptGenerator {}

impl TypeScriptGenerator {
    /// Creates a new TypeScript generator.
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }

    /// Check if a type is a bounded string (string<N> -> Sequence<Char, N>)
    fn is_bounded_string(ty: &IdlType) -> bool {
        matches!(
            ty,
            IdlType::Sequence {
                inner,
                bound: Some(_),
            } if matches!(**inner, IdlType::Primitive(PrimitiveType::Char | PrimitiveType::WChar))
        )
    }

    fn type_to_ts(t: &IdlType) -> String {
        match t {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Void => "void".to_string(),
                PrimitiveType::Boolean => "boolean".to_string(),
                PrimitiveType::Char
                | PrimitiveType::WChar
                | PrimitiveType::String
                | PrimitiveType::WString => "string".to_string(),
                PrimitiveType::Octet
                | PrimitiveType::UInt8
                | PrimitiveType::Int8
                | PrimitiveType::Short
                | PrimitiveType::UnsignedShort
                | PrimitiveType::Long
                | PrimitiveType::UnsignedLong
                | PrimitiveType::Int16
                | PrimitiveType::Int32
                | PrimitiveType::UInt16
                | PrimitiveType::UInt32
                | PrimitiveType::Float
                | PrimitiveType::Double
                | PrimitiveType::LongDouble
                | PrimitiveType::Fixed { .. } => "number".to_string(),
                PrimitiveType::LongLong
                | PrimitiveType::UnsignedLongLong
                | PrimitiveType::Int64
                | PrimitiveType::UInt64 => "bigint".to_string(),
            },
            IdlType::Named(n) => n.replace("::", "_"),
            IdlType::Sequence { inner, .. } => {
                // Handle bounded strings
                if Self::is_bounded_string(t) {
                    return "string".to_string();
                }
                format!("{}[]", Self::type_to_ts(inner))
            }
            IdlType::Array { inner, .. } => {
                format!("{}[]", Self::type_to_ts(inner))
            }
            IdlType::Map { key, value, .. } => {
                format!(
                    "Map<{}, {}>",
                    Self::type_to_ts(key),
                    Self::type_to_ts(value)
                )
            }
        }
    }

    fn emit_header() -> String {
        let mut out = String::new();
        out.push_str("/**\n");
        push_fmt(
            &mut out,
            format_args!(" * Generated by hddsgen v{}\n", env!("HDDS_VERSION")),
        );
        out.push_str(" * DO NOT EDIT\n");
        out.push_str(" *\n");
        out.push_str(" * TypeScript types with CDR2 serialization\n");
        out.push_str(" */\n\n");

        // CDR2 Buffer utilities
        out.push_str("// CDR2 Serialization Helpers\n");
        out.push_str("export class Cdr2Buffer {\n");
        out.push_str("  private view: DataView;\n");
        out.push_str("  private offset: number = 0;\n");
        out.push_str("  private readonly littleEndian: boolean = true;\n\n");

        out.push_str("  constructor(buffer: ArrayBuffer | Uint8Array, offset: number = 0) {\n");
        out.push_str("    if (buffer instanceof Uint8Array) {\n");
        out.push_str("      this.view = new DataView(buffer.buffer, buffer.byteOffset, buffer.byteLength);\n");
        out.push_str("    } else {\n");
        out.push_str("      this.view = new DataView(buffer);\n");
        out.push_str("    }\n");
        out.push_str("    this.offset = offset;\n");
        out.push_str("  }\n\n");

        out.push_str("  get position(): number { return this.offset; }\n");
        out.push_str("  set position(v: number) { this.offset = v; }\n\n");

        // Expose buffer for reading encoded bytes
        out.push_str("  /** Get the encoded bytes (from start to current position) */\n");
        out.push_str("  toBytes(): Uint8Array {\n");
        out.push_str(
            "    return new Uint8Array(this.view.buffer, this.view.byteOffset, this.offset);\n",
        );
        out.push_str("  }\n\n");

        // Read methods
        out.push_str("  readUint8(): number { const v = this.view.getUint8(this.offset); this.offset += 1; return v; }\n");
        out.push_str("  readInt8(): number { const v = this.view.getInt8(this.offset); this.offset += 1; return v; }\n");
        out.push_str("  readUint16(): number { const v = this.view.getUint16(this.offset, this.littleEndian); this.offset += 2; return v; }\n");
        out.push_str("  readInt16(): number { const v = this.view.getInt16(this.offset, this.littleEndian); this.offset += 2; return v; }\n");
        out.push_str("  readUint32(): number { const v = this.view.getUint32(this.offset, this.littleEndian); this.offset += 4; return v; }\n");
        out.push_str("  readInt32(): number { const v = this.view.getInt32(this.offset, this.littleEndian); this.offset += 4; return v; }\n");
        out.push_str("  readFloat32(): number { const v = this.view.getFloat32(this.offset, this.littleEndian); this.offset += 4; return v; }\n");
        out.push_str("  readFloat64(): number { const v = this.view.getFloat64(this.offset, this.littleEndian); this.offset += 8; return v; }\n");
        out.push_str("  readBigUint64(): bigint { const v = this.view.getBigUint64(this.offset, this.littleEndian); this.offset += 8; return v; }\n");
        out.push_str("  readBigInt64(): bigint { const v = this.view.getBigInt64(this.offset, this.littleEndian); this.offset += 8; return v; }\n");
        out.push_str("  readBoolean(): boolean { return this.readUint8() !== 0; }\n\n");

        out.push_str("  readString(): string {\n");
        out.push_str(
            "    const len = this.readUint32(); // CDR2: length includes null terminator\n",
        );
        out.push_str("    const bytes = new Uint8Array(this.view.buffer, this.view.byteOffset + this.offset, len > 0 ? len - 1 : 0);\n");
        out.push_str("    this.offset += len; // advance past string + null\n");
        out.push_str("    return new TextDecoder().decode(bytes);\n");
        out.push_str("  }\n\n");

        // Write methods
        out.push_str("  writeUint8(v: number): void { this.view.setUint8(this.offset, v); this.offset += 1; }\n");
        out.push_str("  writeInt8(v: number): void { this.view.setInt8(this.offset, v); this.offset += 1; }\n");
        out.push_str("  writeUint16(v: number): void { this.view.setUint16(this.offset, v, this.littleEndian); this.offset += 2; }\n");
        out.push_str("  writeInt16(v: number): void { this.view.setInt16(this.offset, v, this.littleEndian); this.offset += 2; }\n");
        out.push_str("  writeUint32(v: number): void { this.view.setUint32(this.offset, v, this.littleEndian); this.offset += 4; }\n");
        out.push_str("  writeInt32(v: number): void { this.view.setInt32(this.offset, v, this.littleEndian); this.offset += 4; }\n");
        out.push_str("  writeFloat32(v: number): void { this.view.setFloat32(this.offset, v, this.littleEndian); this.offset += 4; }\n");
        out.push_str("  writeFloat64(v: number): void { this.view.setFloat64(this.offset, v, this.littleEndian); this.offset += 8; }\n");
        out.push_str("  writeBigUint64(v: bigint): void { this.view.setBigUint64(this.offset, v, this.littleEndian); this.offset += 8; }\n");
        out.push_str("  writeBigInt64(v: bigint): void { this.view.setBigInt64(this.offset, v, this.littleEndian); this.offset += 8; }\n");
        out.push_str("  writeBoolean(v: boolean): void { this.writeUint8(v ? 1 : 0); }\n\n");

        out.push_str("  writeString(s: string): void {\n");
        out.push_str("    const bytes = new TextEncoder().encode(s);\n");
        out.push_str(
            "    this.writeUint32(bytes.length + 1); // CDR2: length includes null terminator\n",
        );
        out.push_str("    new Uint8Array(this.view.buffer, this.view.byteOffset + this.offset).set(bytes);\n");
        out.push_str("    this.offset += bytes.length;\n");
        out.push_str("    this.writeUint8(0); // null terminator\n");
        out.push_str("  }\n\n");

        // Alignment
        out.push_str("  align(n: number): void {\n");
        out.push_str("    const padding = (n - (this.offset % n)) % n;\n");
        out.push_str("    this.offset += padding;\n");
        out.push_str("  }\n");

        out.push_str("}\n\n");

        out
    }

    fn generate_const(c: &Const) -> String {
        let mut out = String::new();
        // Const value is stored as a string, determine the appropriate format
        let ts_type = Self::type_to_ts(&c.const_type);
        let value = if ts_type == "string" {
            format!("\"{}\"", c.value)
        } else {
            c.value.clone()
        };
        push_fmt(
            &mut out,
            format_args!("export const {}: {} = {};\n\n", c.name, ts_type, value),
        );
        out
    }

    fn generate_enum(e: &Enum) -> String {
        let mut out = String::new();

        push_fmt(&mut out, format_args!("export enum {} {{\n", e.name));
        for (i, variant) in e.variants.iter().enumerate() {
            // @audit-ok: safe cast - enum variant index always << i64::MAX
            #[allow(clippy::cast_possible_wrap)]
            let value = variant.value.unwrap_or(i as i64);
            push_fmt(&mut out, format_args!("  {} = {},\n", variant.name, value));
        }
        out.push_str("}\n\n");

        // Encode function - enums are serialized as u32
        push_fmt(
            &mut out,
            format_args!(
                "export function encode{}(v: {}, buf: Cdr2Buffer): void {{\n",
                e.name, e.name
            ),
        );
        out.push_str("  buf.writeUint32(v as number);\n");
        out.push_str("}\n\n");

        // Decode function
        push_fmt(
            &mut out,
            format_args!(
                "export function decode{}(buf: Cdr2Buffer): {} {{\n",
                e.name, e.name
            ),
        );
        push_fmt(
            &mut out,
            format_args!("  return buf.readUint32() as {};\n", e.name),
        );
        out.push_str("}\n\n");

        out
    }

    #[allow(clippy::unused_self)] // Consistent with other generator methods
    fn generate_struct(&self, s: &Struct) -> String {
        if is_mutable(s) {
            return self.generate_mutable_struct(s);
        }
        if is_appendable(s) {
            return self.generate_appendable_struct(s);
        }
        self.generate_final_struct(s)
    }

    /// Generate FINAL struct (no DHEADER, current behavior)
    #[allow(clippy::unused_self)]
    fn generate_final_struct(&self, s: &Struct) -> String {
        let mut out = String::new();

        // Interface definition
        push_fmt(&mut out, format_args!("export interface {} {{\n", s.name));
        for field in &s.fields {
            let escaped = ts_ident(&field.name);
            let ts_type = Self::type_to_ts(&field.field_type);
            let optional = field.is_optional();
            if optional {
                push_fmt(&mut out, format_args!("  {}?: {};\n", escaped, ts_type));
            } else {
                push_fmt(&mut out, format_args!("  {}: {};\n", escaped, ts_type));
            }
        }
        out.push_str("}\n\n");

        // Encode function
        push_fmt(
            &mut out,
            format_args!(
                "export function encode{}(obj: {}, buf: Cdr2Buffer): void {{\n",
                s.name, s.name
            ),
        );
        for field in &s.fields {
            let escaped = ts_ident(&field.name);
            Self::emit_encode_field(&mut out, &escaped, &field.field_type, field.is_optional());
        }
        out.push_str("}\n\n");

        // Decode function
        push_fmt(
            &mut out,
            format_args!(
                "export function decode{}(buf: Cdr2Buffer): {} {{\n",
                s.name, s.name
            ),
        );
        out.push_str("  const obj: any = {};\n");
        for field in &s.fields {
            let escaped = ts_ident(&field.name);
            Self::emit_decode_field(&mut out, &escaped, &field.field_type, field.is_optional());
        }
        push_fmt(&mut out, format_args!("  return obj as {};\n", s.name));
        out.push_str("}\n\n");

        // computeKey function
        out.push_str(&Self::emit_compute_key_function(s));

        out
    }

    /// Generate APPENDABLE struct (DHEADER only, no EMHEADER)
    #[allow(clippy::unused_self)]
    fn generate_appendable_struct(&self, s: &Struct) -> String {
        let mut out = String::new();

        // Interface definition
        push_fmt(&mut out, format_args!("export interface {} {{\n", s.name));
        for field in &s.fields {
            let escaped = ts_ident(&field.name);
            let ts_type = Self::type_to_ts(&field.field_type);
            let optional = field.is_optional();
            if optional {
                push_fmt(&mut out, format_args!("  {}?: {};\n", escaped, ts_type));
            } else {
                push_fmt(&mut out, format_args!("  {}: {};\n", escaped, ts_type));
            }
        }
        out.push_str("}\n\n");

        // Encode function with DHEADER
        push_fmt(
            &mut out,
            format_args!(
                "export function encode{}(obj: {}, buf: Cdr2Buffer): void {{\n",
                s.name, s.name
            ),
        );
        out.push_str("  // DHEADER: reserve 4 bytes for payload length\n");
        out.push_str("  const dheaderPos = buf.position;\n");
        out.push_str("  buf.writeUint32(0); // placeholder\n");
        out.push_str("  const payloadStart = buf.position;\n\n");

        for field in &s.fields {
            let escaped = ts_ident(&field.name);
            Self::emit_encode_field(&mut out, &escaped, &field.field_type, field.is_optional());
        }

        out.push_str("\n  // Fill DHEADER with actual payload length\n");
        out.push_str("  const payloadLen = buf.position - payloadStart;\n");
        out.push_str("  const savedPos = buf.position;\n");
        out.push_str("  buf.position = dheaderPos;\n");
        out.push_str("  buf.writeUint32(payloadLen);\n");
        out.push_str("  buf.position = savedPos;\n");
        out.push_str("}\n\n");

        // Decode function with DHEADER
        push_fmt(
            &mut out,
            format_args!(
                "export function decode{}(buf: Cdr2Buffer): {} {{\n",
                s.name, s.name
            ),
        );
        out.push_str("  // Read DHEADER (payload length)\n");
        out.push_str("  const _dheaderLen = buf.readUint32();\n");
        out.push_str("  const obj: any = {};\n");
        for field in &s.fields {
            let escaped = ts_ident(&field.name);
            Self::emit_decode_field(&mut out, &escaped, &field.field_type, field.is_optional());
        }
        push_fmt(&mut out, format_args!("  return obj as {};\n", s.name));
        out.push_str("}\n\n");

        // computeKey function
        out.push_str(&Self::emit_compute_key_function(s));

        out
    }

    /// Generate MUTABLE struct (DHEADER + EMHEADER per field)
    #[allow(clippy::unused_self)]
    fn generate_mutable_struct(&self, s: &Struct) -> String {
        let mut out = String::new();

        // Interface definition
        push_fmt(&mut out, format_args!("export interface {} {{\n", s.name));
        for field in &s.fields {
            let escaped = ts_ident(&field.name);
            let ts_type = Self::type_to_ts(&field.field_type);
            let optional = field.is_optional();
            if optional {
                push_fmt(&mut out, format_args!("  {}?: {};\n", escaped, ts_type));
            } else {
                push_fmt(&mut out, format_args!("  {}: {};\n", escaped, ts_type));
            }
        }
        out.push_str("}\n\n");

        // Encode function with DHEADER + EMHEADER per field
        push_fmt(
            &mut out,
            format_args!(
                "export function encode{}(obj: {}, buf: Cdr2Buffer): void {{\n",
                s.name, s.name
            ),
        );
        out.push_str("  // DHEADER: reserve 4 bytes for payload length\n");
        out.push_str("  const dheaderPos = buf.position;\n");
        out.push_str("  buf.writeUint32(0); // placeholder\n");
        out.push_str("  const payloadStart = buf.position;\n\n");

        for (idx, field) in s.fields.iter().enumerate() {
            let escaped = ts_ident(&field.name);
            let member_id = compute_member_id(s, idx, field);
            let lc = compute_lc(&field.field_type);
            let use_nextint = lc == 5;

            let mu = field.is_key() || field.is_must_understand();

            if field.is_optional() {
                push_fmt(
                    &mut out,
                    format_args!("  if (obj.{} !== undefined) {{\n", escaped),
                );
                Self::emit_emheader_encode(&mut out, member_id, lc, use_nextint, mu, "    ");
                Self::emit_mutable_field_encode(
                    &mut out,
                    &escaped,
                    &field.field_type,
                    use_nextint,
                    member_id,
                    "    ",
                );
                out.push_str("  }\n\n");
            } else {
                Self::emit_emheader_encode(&mut out, member_id, lc, use_nextint, mu, "  ");
                Self::emit_mutable_field_encode(
                    &mut out,
                    &escaped,
                    &field.field_type,
                    use_nextint,
                    member_id,
                    "  ",
                );
            }
        }

        out.push_str("  // Fill DHEADER with actual payload length\n");
        out.push_str("  const payloadLen = buf.position - payloadStart;\n");
        out.push_str("  const savedPos = buf.position;\n");
        out.push_str("  buf.position = dheaderPos;\n");
        out.push_str("  buf.writeUint32(payloadLen);\n");
        out.push_str("  buf.position = savedPos;\n");
        out.push_str("}\n\n");

        // Decode function with DHEADER + EMHEADER per field
        Self::emit_mutable_decode(&mut out, s);

        // computeKey function
        out.push_str(&Self::emit_compute_key_function(s));

        out
    }

    /// Emit EMHEADER encoding for a mutable field
    fn emit_emheader_encode(
        out: &mut String,
        member_id: u32,
        lc: u32,
        use_nextint: bool,
        must_understand: bool,
        indent: &str,
    ) {
        let mu_bit = if must_understand { "0x80000000 | " } else { "" };
        push_fmt(
            out,
            format_args!("{indent}// EMHEADER: LC={lc}, MemberId=0x{member_id:08X}\n"),
        );
        push_fmt(
            out,
            format_args!(
                "{indent}const emheader_{member_id:x} = {mu_bit}({lc} << 28) | (0x{member_id:08X} & 0x0FFFFFFF);\n"
            ),
        );
        push_fmt(
            out,
            format_args!("{indent}buf.writeUint32(emheader_{member_id:x});\n"),
        );
        if use_nextint {
            push_fmt(
                out,
                format_args!("{indent}const memberLenPos_{member_id:x} = buf.position;\n"),
            );
            push_fmt(
                out,
                format_args!("{indent}buf.writeUint32(0); // NEXTINT placeholder\n"),
            );
            push_fmt(
                out,
                format_args!("{indent}const memberStart_{member_id:x} = buf.position;\n"),
            );
        }
    }

    /// Emit field value encoding for mutable structs
    fn emit_mutable_field_encode(
        out: &mut String,
        name: &str,
        ty: &IdlType,
        use_nextint: bool,
        member_id: u32,
        indent: &str,
    ) {
        Self::emit_encode_value(out, &format!("obj.{name}"), ty, indent);

        if use_nextint {
            push_fmt(
                out,
                format_args!("{indent}// Fill NEXTINT with member length\n"),
            );
            push_fmt(
                out,
                format_args!(
                    "{indent}const memberLen_{member_id:x} = buf.position - memberStart_{member_id:x};\n"
                ),
            );
            push_fmt(
                out,
                format_args!("{indent}const savedMemberPos_{member_id:x} = buf.position;\n"),
            );
            push_fmt(
                out,
                format_args!("{indent}buf.position = memberLenPos_{member_id:x};\n"),
            );
            push_fmt(
                out,
                format_args!("{indent}buf.writeUint32(memberLen_{member_id:x});\n"),
            );
            push_fmt(
                out,
                format_args!("{indent}buf.position = savedMemberPos_{member_id:x};\n"),
            );
        }
        out.push('\n');
    }

    /// Emit mutable struct decode function
    fn emit_mutable_decode(out: &mut String, s: &Struct) {
        push_fmt(
            out,
            format_args!(
                "export function decode{}(buf: Cdr2Buffer): {} {{\n",
                s.name, s.name
            ),
        );
        out.push_str("  // Read DHEADER (payload length)\n");
        out.push_str("  const dheaderLen = buf.readUint32();\n");
        out.push_str("  const payloadEnd = buf.position + dheaderLen;\n");
        out.push_str("  const obj: any = {};\n\n");

        out.push_str("  // Read EMHEADER + member data until end of payload\n");
        out.push_str("  while (buf.position < payloadEnd) {\n");
        out.push_str("    const emheader = buf.readUint32();\n");
        out.push_str("    const lc = (emheader >>> 28) & 0xF;\n");
        out.push_str("    const memberId = emheader & 0x0FFFFFFF;\n");
        out.push_str("    let memberLen: number;\n\n");

        out.push_str("    // Determine member length based on LC\n");
        out.push_str("    switch (lc) {\n");
        out.push_str("      case 0: memberLen = 1; break;\n");
        out.push_str("      case 1: memberLen = 2; break;\n");
        out.push_str("      case 2: memberLen = 4; break;\n");
        out.push_str("      case 3: memberLen = 8; break;\n");
        out.push_str("      case 5: memberLen = buf.readUint32(); break; // NEXTINT\n");
        out.push_str("      default: memberLen = buf.readUint32(); break; // treat as NEXTINT\n");
        out.push_str("    }\n\n");

        out.push_str("    const memberEnd = buf.position + memberLen;\n\n");

        out.push_str("    // Dispatch based on memberId\n");
        out.push_str("    switch (memberId) {\n");

        for (idx, field) in s.fields.iter().enumerate() {
            let escaped = ts_ident(&field.name);
            let member_id = compute_member_id(s, idx, field);
            push_fmt(
                out,
                format_args!("      case 0x{member_id:08X}: // {}\n", field.name),
            );
            Self::emit_mutable_field_decode(out, &escaped, &field.field_type, "        ");
            out.push_str("        break;\n");
        }

        out.push_str("      default:\n");
        out.push_str("        // Unknown member, skip\n");
        out.push_str("        buf.position = memberEnd;\n");
        out.push_str("        break;\n");
        out.push_str("    }\n");
        out.push_str("  }\n\n");

        push_fmt(out, format_args!("  return obj as {};\n", s.name));
        out.push_str("}\n\n");
    }

    /// Generate `computeKey()` function for @key fields
    ///
    /// Returns a 16-byte key hash computed from @key fields using FNV-1a.
    fn emit_compute_key_function(s: &Struct) -> String {
        let mut out = String::new();

        // Find @key fields with their types
        let key_fields: Vec<(&str, &IdlType)> = s
            .fields
            .iter()
            .filter(|f| f.annotations.iter().any(|a| matches!(a, Annotation::Key)))
            .map(|f| (f.name.as_str(), &f.field_type))
            .collect();

        let has_key = !key_fields.is_empty();

        push_fmt(
            &mut out,
            format_args!("/** Compute instance key hash from @key fields (FNV-1a, 16 bytes) */\n"),
        );
        push_fmt(
            &mut out,
            format_args!(
                "export function computeKey{}(obj: {}): Uint8Array {{\n",
                s.name, s.name
            ),
        );

        if has_key {
            out.push_str("  // FNV-1a hash of @key fields\n");
            out.push_str("  let hash = 14695981039346656037n;\n");
            out.push_str("  const PRIME = 1099511628211n;\n\n");

            for (field, field_type) in &key_fields {
                let escaped = ts_ident(field);
                push_fmt(&mut out, format_args!("  // Hash @key field: {}\n", field));

                // Check if field is a string type
                let is_string = matches!(
                    field_type,
                    IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString)
                ) || Self::is_bounded_string(field_type);

                if is_string {
                    push_fmt(
                        &mut out,
                        format_args!(
                            "  const {}_bytes = new TextEncoder().encode(obj.{});\n",
                            escaped, escaped
                        ),
                    );
                } else {
                    // For numbers, use DataView to get bytes
                    let (byte_size, set_method, is_bigint) = Self::get_dataview_info(field_type);
                    push_fmt(
                        &mut out,
                        format_args!(
                            "  const {}_buf = new ArrayBuffer({});\n",
                            escaped, byte_size
                        ),
                    );
                    push_fmt(
                        &mut out,
                        format_args!(
                            "  const {}_view = new DataView({}_buf);\n",
                            escaped, escaped
                        ),
                    );
                    // Note: is_bigint is used for type checking but the generated code is the same
                    let _ = is_bigint;
                    push_fmt(
                        &mut out,
                        format_args!(
                            "  {}_view.{}(0, obj.{}, true);\n",
                            escaped, set_method, escaped
                        ),
                    );
                    push_fmt(
                        &mut out,
                        format_args!(
                            "  const {}_bytes = new Uint8Array({}_buf);\n",
                            escaped, escaped
                        ),
                    );
                }

                push_fmt(
                    &mut out,
                    format_args!("  for (const b of {}_bytes) {{\n", escaped),
                );
                out.push_str("    hash ^= BigInt(b);\n");
                out.push_str("    hash = (hash * PRIME) & 0xFFFFFFFFFFFFFFFFn;\n");
                out.push_str("  }\n\n");
            }

            out.push_str("  // Expand to 16 bytes\n");
            out.push_str("  const key = new Uint8Array(16);\n");
            out.push_str("  const keyView = new DataView(key.buffer);\n");
            out.push_str("  keyView.setBigUint64(0, hash, true);\n");
            out.push_str("  hash = (hash * PRIME) & 0xFFFFFFFFFFFFFFFFn;\n");
            out.push_str("  keyView.setBigUint64(8, hash, true);\n");
            out.push_str("  return key;\n");
        } else {
            out.push_str("  // No @key fields - return zeroed hash\n");
            out.push_str("  return new Uint8Array(16);\n");
        }

        out.push_str("}\n\n");
        out
    }

    /// Get `DataView` info for a type: (`byte_size`, `set_method`, `is_bigint`)
    #[allow(clippy::match_same_arms)] // Named types explicitly documented for clarity
    const fn get_dataview_info(ty: &IdlType) -> (usize, &'static str, bool) {
        match ty {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Boolean
                | PrimitiveType::Char
                | PrimitiveType::Octet
                | PrimitiveType::UInt8
                | PrimitiveType::Int8
                | PrimitiveType::Void => (1, "setUint8", false),
                PrimitiveType::Short | PrimitiveType::Int16 => (2, "setInt16", false),
                PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => (2, "setUint16", false),
                PrimitiveType::Long | PrimitiveType::Int32 => (4, "setInt32", false),
                PrimitiveType::UnsignedLong | PrimitiveType::UInt32 | PrimitiveType::WChar => {
                    (4, "setUint32", false)
                }
                PrimitiveType::LongLong | PrimitiveType::Int64 => (8, "setBigInt64", true),
                PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => {
                    (8, "setBigUint64", true)
                }
                PrimitiveType::Float => (4, "setFloat32", false),
                PrimitiveType::Double | PrimitiveType::LongDouble | PrimitiveType::Fixed { .. } => {
                    (8, "setFloat64", false)
                }
                PrimitiveType::String | PrimitiveType::WString => (0, "", false), // Handled separately
            },
            // For named types (enums) and other complex types, treat as u32
            IdlType::Named(_)
            | IdlType::Sequence { .. }
            | IdlType::Array { .. }
            | IdlType::Map { .. } => (4, "setUint32", false),
        }
    }

    /// Emit single field decode for mutable struct
    fn emit_mutable_field_decode(out: &mut String, name: &str, ty: &IdlType, indent: &str) {
        match ty {
            IdlType::Primitive(p) => {
                let method = Self::primitive_read_method(p);
                push_fmt(out, format_args!("{indent}obj.{name} = buf.{method}();\n"));
            }
            IdlType::Sequence { .. } if Self::is_bounded_string(ty) => {
                push_fmt(
                    out,
                    format_args!("{indent}obj.{name} = buf.readString();\n"),
                );
            }
            IdlType::Sequence { inner, .. } | IdlType::Array { inner, .. } => {
                let elem_type = Self::type_to_ts(inner);
                push_fmt(
                    out,
                    format_args!("{indent}const {name}_len = buf.readUint32();\n"),
                );
                push_fmt(
                    out,
                    format_args!("{indent}obj.{name} = [] as {elem_type}[];\n"),
                );
                push_fmt(
                    out,
                    format_args!("{indent}for (let i = 0; i < {name}_len; i++) {{\n"),
                );
                Self::emit_decode_array_element(out, name, inner, &format!("{indent}  "));
                push_fmt(out, format_args!("{indent}}}\n"));
            }
            IdlType::Map { key, value, .. } => {
                let key_type = Self::type_to_ts(key);
                let val_type = Self::type_to_ts(value);
                push_fmt(
                    out,
                    format_args!("{indent}const {name}_len = buf.readUint32();\n"),
                );
                push_fmt(
                    out,
                    format_args!("{indent}obj.{name} = new Map<{key_type}, {val_type}>();\n"),
                );
                push_fmt(
                    out,
                    format_args!("{indent}for (let i = 0; i < {name}_len; i++) {{\n"),
                );
                Self::emit_decode_map_element(out, name, key, value, &format!("{indent}  "));
                push_fmt(out, format_args!("{indent}}}\n"));
            }
            IdlType::Named(type_name) => {
                let ts_name = type_name.replace("::", "_");
                push_fmt(
                    out,
                    format_args!("{indent}obj.{name} = decode{ts_name}(buf);\n"),
                );
            }
        }
    }

    fn emit_encode_field(out: &mut String, name: &str, ty: &IdlType, optional: bool) {
        let indent = "  ";

        // Alignment
        let align = Self::get_alignment(ty);
        if align > 1 {
            push_fmt(out, format_args!("{indent}buf.align({align});\n"));
        }

        if optional {
            push_fmt(
                out,
                format_args!("{indent}if (obj.{name} !== undefined) {{\n"),
            );
            push_fmt(out, format_args!("{indent}  buf.writeUint8(1);\n"));
            Self::emit_encode_value(out, &format!("obj.{name}"), ty, &format!("{indent}  "));
            push_fmt(out, format_args!("{indent}}} else {{\n"));
            push_fmt(out, format_args!("{indent}  buf.writeUint8(0);\n"));
            push_fmt(out, format_args!("{indent}}}\n"));
        } else {
            Self::emit_encode_value(out, &format!("obj.{name}"), ty, indent);
        }
    }

    fn emit_encode_value(out: &mut String, expr: &str, ty: &IdlType, indent: &str) {
        match ty {
            IdlType::Primitive(p) => {
                let method = Self::primitive_write_method(p);
                push_fmt(out, format_args!("{indent}buf.{method}({expr});\n"));
            }
            IdlType::Sequence { inner, .. } if Self::is_bounded_string(ty) => {
                push_fmt(out, format_args!("{indent}buf.writeString({expr});\n"));
            }
            IdlType::Sequence { inner, .. } | IdlType::Array { inner, .. } => {
                push_fmt(
                    out,
                    format_args!("{indent}buf.writeUint32({expr}.length);\n"),
                );
                push_fmt(out, format_args!("{indent}for (const elem of {expr}) {{\n"));
                Self::emit_encode_value(out, "elem", inner, &format!("{indent}  "));
                push_fmt(out, format_args!("{indent}}}\n"));
            }
            IdlType::Map { key, value, .. } => {
                push_fmt(out, format_args!("{indent}buf.writeUint32({expr}.size);\n"));
                push_fmt(
                    out,
                    format_args!("{indent}for (const [k, v] of {expr}.entries()) {{\n"),
                );
                Self::emit_encode_value(out, "k", key, &format!("{indent}  "));
                Self::emit_encode_value(out, "v", value, &format!("{indent}  "));
                push_fmt(out, format_args!("{indent}}}\n"));
            }
            IdlType::Named(name) => {
                let ts_name = name.replace("::", "_");
                push_fmt(out, format_args!("{indent}encode{ts_name}({expr}, buf);\n"));
            }
        }
    }

    fn emit_decode_field(out: &mut String, name: &str, ty: &IdlType, optional: bool) {
        let indent = "  ";

        // Alignment
        let align = Self::get_alignment(ty);
        if align > 1 {
            push_fmt(out, format_args!("{indent}buf.align({align});\n"));
        }

        if optional {
            push_fmt(out, format_args!("{indent}if (buf.readUint8() !== 0) {{\n"));
            Self::emit_decode_value(out, name, ty, &format!("{indent}  "));
            push_fmt(out, format_args!("{indent}}}\n"));
        } else {
            Self::emit_decode_value(out, name, ty, indent);
        }
    }

    fn emit_decode_value(out: &mut String, target: &str, ty: &IdlType, indent: &str) {
        match ty {
            IdlType::Primitive(p) => {
                let method = Self::primitive_read_method(p);
                push_fmt(
                    out,
                    format_args!("{indent}obj.{target} = buf.{method}();\n"),
                );
            }
            IdlType::Sequence { inner, .. } if Self::is_bounded_string(ty) => {
                push_fmt(
                    out,
                    format_args!("{indent}obj.{target} = buf.readString();\n"),
                );
            }
            IdlType::Sequence { inner, .. } | IdlType::Array { inner, .. } => {
                let elem_type = Self::type_to_ts(inner);
                push_fmt(
                    out,
                    format_args!("{indent}const {target}_len = buf.readUint32();\n"),
                );
                push_fmt(
                    out,
                    format_args!("{indent}obj.{target} = [] as {elem_type}[];\n"),
                );
                push_fmt(
                    out,
                    format_args!("{indent}for (let i = 0; i < {target}_len; i++) {{\n"),
                );
                Self::emit_decode_array_element(out, target, inner, &format!("{indent}  "));
                push_fmt(out, format_args!("{indent}}}\n"));
            }
            IdlType::Map { key, value, .. } => {
                let key_type = Self::type_to_ts(key);
                let val_type = Self::type_to_ts(value);
                push_fmt(
                    out,
                    format_args!("{indent}const {target}_len = buf.readUint32();\n"),
                );
                push_fmt(
                    out,
                    format_args!("{indent}obj.{target} = new Map<{key_type}, {val_type}>();\n"),
                );
                push_fmt(
                    out,
                    format_args!("{indent}for (let i = 0; i < {target}_len; i++) {{\n"),
                );
                Self::emit_decode_map_element(out, target, key, value, &format!("{indent}  "));
                push_fmt(out, format_args!("{indent}}}\n"));
            }
            IdlType::Named(name) => {
                let ts_name = name.replace("::", "_");
                push_fmt(
                    out,
                    format_args!("{indent}obj.{target} = decode{ts_name}(buf);\n"),
                );
            }
        }
    }

    fn emit_decode_array_element(out: &mut String, target: &str, inner: &IdlType, indent: &str) {
        match inner {
            IdlType::Primitive(p) => {
                let method = Self::primitive_read_method(p);
                push_fmt(
                    out,
                    format_args!("{indent}obj.{target}.push(buf.{method}());\n"),
                );
            }
            IdlType::Sequence { .. } if Self::is_bounded_string(inner) => {
                push_fmt(
                    out,
                    format_args!("{indent}obj.{target}.push(buf.readString());\n"),
                );
            }
            IdlType::Named(name) => {
                let ts_name = name.replace("::", "_");
                push_fmt(
                    out,
                    format_args!("{indent}obj.{target}.push(decode{ts_name}(buf));\n"),
                );
            }
            _ => {
                // Nested sequences/arrays - simplified handling
                push_fmt(
                    out,
                    format_args!("{indent}// nested container: not yet supported\n"),
                );
            }
        }
    }

    fn emit_decode_map_element(
        out: &mut String,
        target: &str,
        key: &IdlType,
        value: &IdlType,
        indent: &str,
    ) {
        // Decode key
        let key_method = match key {
            IdlType::Primitive(p) => Self::primitive_read_method(p),
            _ => "readUint32", // fallback for complex keys
        };
        push_fmt(out, format_args!("{indent}const k = buf.{key_method}();\n"));

        // Decode value
        match value {
            IdlType::Primitive(p) => {
                let method = Self::primitive_read_method(p);
                push_fmt(out, format_args!("{indent}const v = buf.{method}();\n"));
            }
            IdlType::Named(name) => {
                let ts_name = name.replace("::", "_");
                push_fmt(
                    out,
                    format_args!("{indent}const v = decode{ts_name}(buf);\n"),
                );
            }
            _ => {
                push_fmt(
                    out,
                    format_args!("{indent}const v = null; // unsupported type\n"),
                );
            }
        }
        push_fmt(out, format_args!("{indent}obj.{target}.set(k, v);\n"));
    }

    const fn primitive_write_method(p: &PrimitiveType) -> &'static str {
        match p {
            PrimitiveType::Boolean => "writeBoolean",
            PrimitiveType::Char
            | PrimitiveType::Octet
            | PrimitiveType::UInt8
            | PrimitiveType::Void => "writeUint8",
            PrimitiveType::Int8 => "writeInt8",
            PrimitiveType::Short | PrimitiveType::Int16 => "writeInt16",
            PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => "writeUint16",
            PrimitiveType::Long | PrimitiveType::Int32 => "writeInt32",
            PrimitiveType::UnsignedLong | PrimitiveType::UInt32 | PrimitiveType::WChar => {
                "writeUint32"
            }
            PrimitiveType::LongLong | PrimitiveType::Int64 => "writeBigInt64",
            PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => "writeBigUint64",
            PrimitiveType::Float => "writeFloat32",
            PrimitiveType::Double | PrimitiveType::LongDouble | PrimitiveType::Fixed { .. } => {
                "writeFloat64" // Fixed is simplified as Float64
            }
            PrimitiveType::String | PrimitiveType::WString => "writeString",
        }
    }

    const fn primitive_read_method(p: &PrimitiveType) -> &'static str {
        match p {
            PrimitiveType::Boolean => "readBoolean",
            PrimitiveType::Char
            | PrimitiveType::Octet
            | PrimitiveType::UInt8
            | PrimitiveType::Void => "readUint8",
            PrimitiveType::Int8 => "readInt8",
            PrimitiveType::Short | PrimitiveType::Int16 => "readInt16",
            PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => "readUint16",
            PrimitiveType::Long | PrimitiveType::Int32 => "readInt32",
            PrimitiveType::UnsignedLong | PrimitiveType::UInt32 | PrimitiveType::WChar => {
                "readUint32"
            }
            PrimitiveType::LongLong | PrimitiveType::Int64 => "readBigInt64",
            PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => "readBigUint64",
            PrimitiveType::Float => "readFloat32",
            PrimitiveType::Double | PrimitiveType::LongDouble | PrimitiveType::Fixed { .. } => {
                "readFloat64"
            }
            PrimitiveType::String | PrimitiveType::WString => "readString",
        }
    }

    const fn get_alignment(ty: &IdlType) -> usize {
        match ty {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Void
                | PrimitiveType::Boolean
                | PrimitiveType::Char
                | PrimitiveType::Octet
                | PrimitiveType::Int8
                | PrimitiveType::UInt8 => 1,
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
            IdlType::Sequence { .. }
            | IdlType::Array { .. }
            | IdlType::Map { .. }
            | IdlType::Named(_) => 4,
        }
    }

    fn generate_typedef(t: &Typedef) -> String {
        let mut out = String::new();
        let ts_type = Self::type_to_ts(&t.base_type);
        push_fmt(
            &mut out,
            format_args!("export type {} = {};\n\n", t.name, ts_type),
        );
        out
    }

    #[allow(clippy::too_many_lines)]
    fn generate_union(u: &Union) -> String {
        let mut out = String::new();

        // Generate discriminator type
        let disc_type = Self::type_to_ts(&u.discriminator);

        // Generate interface for each case
        push_fmt(&mut out, format_args!("// Union: {}\n", u.name));
        for case in &u.cases {
            let escaped = ts_ident(&case.field.name);
            let case_name = format!("{}_{}", u.name, case.field.name);
            push_fmt(&mut out, format_args!("export interface {case_name} {{\n"));
            push_fmt(&mut out, format_args!("  discriminator: {disc_type};\n"));
            let field_type = Self::type_to_ts(&case.field.field_type);
            push_fmt(&mut out, format_args!("  {}: {};\n", escaped, field_type));
            out.push_str("}\n");
        }

        // Union type alias
        push_fmt(&mut out, format_args!("export type {} = ", u.name));
        let case_names: Vec<String> = u
            .cases
            .iter()
            .map(|c| format!("{}_{}", u.name, c.field.name))
            .collect();
        out.push_str(&case_names.join(" | "));
        out.push_str(";\n\n");

        // Encode function
        push_fmt(
            &mut out,
            format_args!(
                "export function encode{}(obj: {}, buf: Cdr2Buffer): void {{\n",
                u.name, u.name
            ),
        );

        // Discriminator write method
        let disc_write_method = Self::discriminator_write_method(&u.discriminator);

        out.push_str("  // Write discriminator\n");
        push_fmt(
            &mut out,
            format_args!("  buf.{}(obj.discriminator);\n", disc_write_method),
        );
        out.push_str("  // Encode value based on discriminator\n");

        // Generate switch on discriminator
        let mut first = true;
        for case in &u.cases {
            let disc_value = Self::get_discriminator_value_ts(case, &u.discriminator);
            let is_default = case.labels.iter().any(|l| matches!(l, UnionLabel::Default));

            if is_default {
                out.push_str("  } else {\n");
            } else if first {
                push_fmt(
                    &mut out,
                    format_args!("  if (obj.discriminator === {}) {{\n", disc_value),
                );
                first = false;
            } else {
                push_fmt(
                    &mut out,
                    format_args!("  }} else if (obj.discriminator === {}) {{\n", disc_value),
                );
            }

            // Type guard and encode value
            let field_name = &case.field.name;
            let escaped_field = ts_ident(field_name);
            push_fmt(
                &mut out,
                format_args!(
                    "    const v = (obj as {}_{}).{};\n",
                    u.name, field_name, escaped_field
                ),
            );
            Self::emit_encode_union_value(&mut out, "v", &case.field.field_type, "    ");
        }
        out.push_str("  }\n");
        out.push_str("}\n\n");

        // Decode function
        push_fmt(
            &mut out,
            format_args!(
                "export function decode{}(buf: Cdr2Buffer): {} {{\n",
                u.name, u.name
            ),
        );

        // Discriminator read method
        let disc_read_method = Self::discriminator_read_method(&u.discriminator);

        out.push_str("  // Read discriminator\n");
        push_fmt(
            &mut out,
            format_args!("  const disc = buf.{}();\n", disc_read_method),
        );
        out.push_str("  // Decode value based on discriminator\n");

        // Generate switch on discriminator
        let mut first = true;
        let mut has_default = false;
        for case in &u.cases {
            let disc_value = Self::get_discriminator_value_ts(case, &u.discriminator);
            let is_default = case.labels.iter().any(|l| matches!(l, UnionLabel::Default));

            if is_default {
                has_default = true;
                out.push_str("  } else {\n");
            } else if first {
                push_fmt(
                    &mut out,
                    format_args!("  if (disc === {}) {{\n", disc_value),
                );
                first = false;
            } else {
                push_fmt(
                    &mut out,
                    format_args!("  }} else if (disc === {}) {{\n", disc_value),
                );
            }

            // Decode the value and return the appropriate variant
            let field_name = &case.field.name;
            let escaped_field = ts_ident(field_name);
            let decode_expr = Self::decode_value_expr(&case.field.field_type);
            push_fmt(
                &mut out,
                format_args!(
                    "    return {{ discriminator: disc, {}: {} }} as {};\n",
                    escaped_field, decode_expr, u.name
                ),
            );
        }

        // Add else clause if no default case
        if !has_default {
            out.push_str("  } else {\n");
            push_fmt(
                &mut out,
                format_args!(
                    "    throw new Error(`Unknown discriminator value: ${{disc}} for union {}`);\n",
                    u.name
                ),
            );
        }
        out.push_str("  }\n");
        out.push_str("}\n\n");

        out
    }

    /// Get the TypeScript discriminator value for a union case
    fn get_discriminator_value_ts(case: &crate::ast::UnionCase, disc_type: &IdlType) -> String {
        for label in &case.labels {
            if let UnionLabel::Value(v) = label {
                // If discriminator is a named type (enum), use the enum value
                if let IdlType::Named(enum_name) = disc_type {
                    let ts_enum = enum_name.replace("::", "_");
                    return format!("{}.{}", ts_enum, v);
                }
                return v.clone();
            }
        }
        // Default case: zero value for the discriminator type
        match disc_type {
            IdlType::Primitive(PrimitiveType::Boolean) => "false".to_string(),
            _ => "0".to_string(),
        }
    }

    /// Get the write method for the discriminator type
    #[allow(clippy::missing_const_for_fn)]
    fn discriminator_write_method(disc_type: &IdlType) -> &'static str {
        #[allow(clippy::match_same_arms)]
        match disc_type {
            IdlType::Primitive(p) => Self::primitive_write_method(p),
            IdlType::Named(_)
            | IdlType::Sequence { .. }
            | IdlType::Array { .. }
            | IdlType::Map { .. } => "writeUint32", // Enums and others are serialized as u32
        }
    }

    /// Get the read method for the discriminator type
    #[allow(clippy::missing_const_for_fn)]
    fn discriminator_read_method(disc_type: &IdlType) -> &'static str {
        #[allow(clippy::match_same_arms)]
        match disc_type {
            IdlType::Primitive(p) => Self::primitive_read_method(p),
            IdlType::Named(_)
            | IdlType::Sequence { .. }
            | IdlType::Array { .. }
            | IdlType::Map { .. } => "readUint32", // Enums and others are serialized as u32
        }
    }

    /// Emit encode for a union value (similar to `emit_encode_value` but for union context)
    fn emit_encode_union_value(out: &mut String, expr: &str, ty: &IdlType, indent: &str) {
        match ty {
            IdlType::Primitive(p) => {
                let method = Self::primitive_write_method(p);
                push_fmt(out, format_args!("{indent}buf.{method}({expr});\n"));
            }
            IdlType::Sequence { .. } if Self::is_bounded_string(ty) => {
                push_fmt(out, format_args!("{indent}buf.writeString({expr});\n"));
            }
            IdlType::Sequence { inner, .. } | IdlType::Array { inner, .. } => {
                push_fmt(
                    out,
                    format_args!("{indent}buf.writeUint32({expr}.length);\n"),
                );
                push_fmt(out, format_args!("{indent}for (const elem of {expr}) {{\n"));
                Self::emit_encode_union_value(out, "elem", inner, &format!("{indent}  "));
                push_fmt(out, format_args!("{indent}}}\n"));
            }
            IdlType::Map { key, value, .. } => {
                push_fmt(out, format_args!("{indent}buf.writeUint32({expr}.size);\n"));
                push_fmt(
                    out,
                    format_args!("{indent}for (const [k, vv] of {expr}.entries()) {{\n"),
                );
                Self::emit_encode_union_value(out, "k", key, &format!("{indent}  "));
                Self::emit_encode_union_value(out, "vv", value, &format!("{indent}  "));
                push_fmt(out, format_args!("{indent}}}\n"));
            }
            IdlType::Named(name) => {
                let ts_name = name.replace("::", "_");
                push_fmt(out, format_args!("{indent}encode{ts_name}({expr}, buf);\n"));
            }
        }
    }

    /// Generate decode expression for a type (returns the expression, not assignment)
    fn decode_value_expr(ty: &IdlType) -> String {
        match ty {
            IdlType::Primitive(p) => {
                let method = Self::primitive_read_method(p);
                format!("buf.{}()", method)
            }
            IdlType::Sequence { .. } if Self::is_bounded_string(ty) => {
                "buf.readString()".to_string()
            }
            IdlType::Sequence { inner, .. } | IdlType::Array { inner, .. } => {
                let inner_decode = Self::decode_value_expr(inner);
                format!(
                    "(() => {{ const len = buf.readUint32(); const arr = []; for (let i = 0; i < len; i++) arr.push({}); return arr; }})()",
                    inner_decode
                )
            }
            IdlType::Map { key, value, .. } => {
                let key_decode = Self::decode_value_expr(key);
                let value_decode = Self::decode_value_expr(value);
                format!(
                    "(() => {{ const len = buf.readUint32(); const m = new Map(); for (let i = 0; i < len; i++) m.set({}, {}); return m; }})()",
                    key_decode, value_decode
                )
            }
            IdlType::Named(name) => {
                let ts_name = name.replace("::", "_");
                format!("decode{}(buf)", ts_name)
            }
        }
    }

    fn generate_bitset(b: &Bitset) -> String {
        let mut out = String::new();
        push_fmt(&mut out, format_args!("// Bitset: {}\n", b.name));
        push_fmt(&mut out, format_args!("export interface {} {{\n", b.name));
        for field in &b.fields {
            let escaped = ts_ident(&field.name);
            // Use boolean for single-bit fields, number for multi-bit fields
            let field_type = if field.width == 1 {
                "boolean"
            } else {
                "number"
            };
            push_fmt(&mut out, format_args!("  {}: {};\n", escaped, field_type));
        }
        out.push_str("}\n\n");

        // Generate encode function - pack all fields into uint64
        push_fmt(
            &mut out,
            format_args!(
                "export function encode{}(obj: {}, buf: Cdr2Buffer): void {{\n",
                b.name, b.name
            ),
        );
        out.push_str("  let packed = 0n;\n");

        let mut offset: u32 = 0;
        for field in &b.fields {
            let escaped = ts_ident(&field.name);
            if field.width == 1 {
                // Single bit: boolean
                push_fmt(
                    &mut out,
                    format_args!(
                        "  packed |= BigInt(obj.{} ? 1 : 0) << {}n;\n",
                        escaped, offset
                    ),
                );
            } else {
                // Multi-bit: mask to field width
                let mask = (1u64 << field.width) - 1;
                push_fmt(
                    &mut out,
                    format_args!(
                        "  packed |= (BigInt(obj.{}) & 0x{:X}n) << {}n;\n",
                        escaped, mask, offset
                    ),
                );
            }
            offset += field.width;
        }

        out.push_str("  buf.writeBigUint64(packed);\n");
        out.push_str("}\n\n");

        // Generate decode function - unpack uint64 into fields
        push_fmt(
            &mut out,
            format_args!(
                "export function decode{}(buf: Cdr2Buffer): {} {{\n",
                b.name, b.name
            ),
        );
        out.push_str("  const packed = buf.readBigUint64();\n");
        push_fmt(&mut out, format_args!("  const obj: {} = {{\n", b.name));

        let mut offset: u32 = 0;
        for (i, field) in b.fields.iter().enumerate() {
            let escaped = ts_ident(&field.name);
            let comma = if i < b.fields.len() - 1 { "," } else { "" };
            if field.width == 1 {
                // Single bit: convert to boolean
                push_fmt(
                    &mut out,
                    format_args!(
                        "    {}: ((packed >> {}n) & 1n) !== 0n{}\n",
                        escaped, offset, comma
                    ),
                );
            } else {
                // Multi-bit: mask and convert to number
                let mask = (1u64 << field.width) - 1;
                push_fmt(
                    &mut out,
                    format_args!(
                        "    {}: Number((packed >> {}n) & 0x{:X}n){}\n",
                        escaped, offset, mask, comma
                    ),
                );
            }
            offset += field.width;
        }

        out.push_str("  };\n");
        out.push_str("  return obj;\n");
        out.push_str("}\n\n");

        out
    }

    fn generate_bitmask(m: &Bitmask) -> String {
        let mut out = String::new();
        push_fmt(&mut out, format_args!("export enum {} {{\n", m.name));
        for (i, flag) in m.flags.iter().enumerate() {
            // Extract position from @position annotation if present, otherwise use index
            // @audit-ok: safe cast - bitmask flag index always << u32::MAX
            #[allow(clippy::cast_possible_truncation)]
            let position = flag
                .annotations
                .iter()
                .find_map(|a| {
                    if let Annotation::Position(pos) = a {
                        Some(*pos)
                    } else {
                        None
                    }
                })
                .unwrap_or(i as u32);
            push_fmt(
                &mut out,
                format_args!("  {} = 1 << {},\n", flag.name, position),
            );
        }
        out.push_str("}\n\n");

        // Generate encode function - bitmasks are stored as uint64
        push_fmt(
            &mut out,
            format_args!(
                "export function encode{}(val: number | bigint, buf: Cdr2Buffer): void {{\n",
                m.name
            ),
        );
        out.push_str("  buf.writeBigUint64(BigInt(val));\n");
        out.push_str("}\n\n");

        // Generate decode function - return as number (safe for typical bitmask values)
        push_fmt(
            &mut out,
            format_args!(
                "export function decode{}(buf: Cdr2Buffer): number {{\n",
                m.name
            ),
        );
        out.push_str("  return Number(buf.readBigUint64());\n");
        out.push_str("}\n\n");

        out
    }

    fn generate_definitions(&self, defs: &[Definition], out: &mut String) {
        for def in defs {
            match def {
                Definition::Module(m) => {
                    push_fmt(out, format_args!("// Module: {}\n", m.name));
                    push_fmt(out, format_args!("export namespace {} {{\n", m.name));
                    // Generate inner definitions with increased indent
                    for inner in &m.definitions {
                        let inner_code = match inner {
                            Definition::Struct(s) => self.generate_struct(s),
                            Definition::Enum(e) => Self::generate_enum(e),
                            Definition::Typedef(t) => Self::generate_typedef(t),
                            Definition::Union(u) => Self::generate_union(u),
                            Definition::Const(c) => Self::generate_const(c),
                            Definition::Bitset(b) => Self::generate_bitset(b),
                            Definition::Bitmask(m) => Self::generate_bitmask(m),
                            _ => String::new(),
                        };
                        // Indent inner code
                        for line in inner_code.lines() {
                            if line.is_empty() {
                                out.push('\n');
                            } else {
                                push_fmt(out, format_args!("  {}\n", line));
                            }
                        }
                    }
                    out.push_str("}\n\n");
                }
                Definition::Struct(s) => out.push_str(&self.generate_struct(s)),
                Definition::Enum(e) => out.push_str(&Self::generate_enum(e)),
                Definition::Typedef(t) => out.push_str(&Self::generate_typedef(t)),
                Definition::Union(u) => out.push_str(&Self::generate_union(u)),
                Definition::Const(c) => out.push_str(&Self::generate_const(c)),
                Definition::Bitset(b) => out.push_str(&Self::generate_bitset(b)),
                Definition::Bitmask(m) => out.push_str(&Self::generate_bitmask(m)),
                _ => {}
            }
        }
    }
}

impl Default for TypeScriptGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGenerator for TypeScriptGenerator {
    fn generate(&self, ast: &IdlFile) -> Result<String> {
        let mut output = String::new();

        output.push_str(&Self::emit_header());
        self.generate_definitions(&ast.definitions, &mut output);

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Field;

    fn make_final_struct() -> Struct {
        let mut s = Struct::new("FinalPoint");
        s.add_field(Field::new("x", IdlType::Primitive(PrimitiveType::Float)));
        s.add_field(Field::new("y", IdlType::Primitive(PrimitiveType::Float)));
        s
    }

    fn make_appendable_struct() -> Struct {
        let mut s = Struct::new("AppendablePoint");
        s.extensibility = Some(ExtensibilityKind::Appendable);
        s.add_field(Field::new("x", IdlType::Primitive(PrimitiveType::Float)));
        s.add_field(Field::new("y", IdlType::Primitive(PrimitiveType::Float)));
        s
    }

    fn make_mutable_struct() -> Struct {
        let mut s = Struct::new("MutablePoint");
        s.extensibility = Some(ExtensibilityKind::Mutable);
        s.add_field(Field::new("x", IdlType::Primitive(PrimitiveType::Float)));
        s.add_field(Field::new("y", IdlType::Primitive(PrimitiveType::Float)));
        s
    }

    #[test]
    fn final_struct_no_dheader() {
        let generator = TypeScriptGenerator::new();
        let code = generator.generate_struct(&make_final_struct());
        // FINAL structs should NOT have DHEADER
        assert!(
            !code.contains("DHEADER"),
            "FINAL struct should not have DHEADER"
        );
        assert!(code.contains("export interface FinalPoint"));
        assert!(code.contains("export function encodeFinalPoint"));
        assert!(code.contains("export function decodeFinalPoint"));
    }

    #[test]
    fn appendable_struct_has_dheader() {
        let generator = TypeScriptGenerator::new();
        let code = generator.generate_struct(&make_appendable_struct());
        // APPENDABLE structs should have DHEADER but no EMHEADER
        assert!(
            code.contains("DHEADER"),
            "APPENDABLE struct should have DHEADER"
        );
        assert!(
            !code.contains("EMHEADER"),
            "APPENDABLE struct should not have EMHEADER"
        );
        assert!(code.contains("dheaderPos"));
        assert!(code.contains("payloadStart"));
        assert!(code.contains("payloadLen"));
    }

    #[test]
    fn mutable_struct_has_dheader_and_emheader() {
        let generator = TypeScriptGenerator::new();
        let code = generator.generate_struct(&make_mutable_struct());
        // MUTABLE structs should have both DHEADER and EMHEADER
        assert!(
            code.contains("DHEADER"),
            "MUTABLE struct should have DHEADER"
        );
        assert!(
            code.contains("EMHEADER"),
            "MUTABLE struct should have EMHEADER per field"
        );
        assert!(code.contains("emheader"), "should emit EMHEADER write");
        assert!(code.contains("lc"), "should decode LC field");
        assert!(code.contains("memberId"), "should decode memberId");
    }

    #[test]
    fn mutable_struct_member_id_computed() {
        let s = make_mutable_struct();
        // Member IDs are computed as FNV-1a hash of field name
        let id_x = compute_member_id(&s, 0, &s.fields[0]);
        let id_y = compute_member_id(&s, 1, &s.fields[1]);
        // IDs should be different for different field names
        assert_ne!(
            id_x, id_y,
            "Different fields should have different member IDs"
        );
        // IDs should be within the 28-bit range
        assert!(id_x <= 0x0FFF_FFFF, "Member ID should fit in 28 bits");
        assert!(id_y <= 0x0FFF_FFFF, "Member ID should fit in 28 bits");
    }

    #[test]
    fn mutable_struct_with_explicit_id() {
        let mut s = Struct::new("ExplicitIdStruct");
        s.extensibility = Some(ExtensibilityKind::Mutable);
        let mut field = Field::new("value", IdlType::Primitive(PrimitiveType::Int32));
        field.annotations.push(Annotation::Id(42));
        s.add_field(field);

        let id = compute_member_id(&s, 0, &s.fields[0]);
        assert_eq!(id, 42, "Explicit @id annotation should be used");
    }

    #[test]
    fn mutable_struct_with_autoid_sequential() {
        let mut s = Struct::new("SequentialIdStruct");
        s.extensibility = Some(ExtensibilityKind::Mutable);
        s.annotations
            .push(Annotation::AutoId(AutoIdKind::Sequential));
        s.add_field(Field::new(
            "first",
            IdlType::Primitive(PrimitiveType::Int32),
        ));
        s.add_field(Field::new(
            "second",
            IdlType::Primitive(PrimitiveType::Int32),
        ));

        let id0 = compute_member_id(&s, 0, &s.fields[0]);
        let id1 = compute_member_id(&s, 1, &s.fields[1]);
        assert_eq!(id0, 0, "Sequential ID for field 0 should be 0");
        assert_eq!(id1, 1, "Sequential ID for field 1 should be 1");
    }

    #[test]
    fn lc_values_correct() {
        // LC=0 for 1-byte types
        assert_eq!(compute_lc(&IdlType::Primitive(PrimitiveType::Boolean)), 0);
        assert_eq!(compute_lc(&IdlType::Primitive(PrimitiveType::Octet)), 0);
        assert_eq!(compute_lc(&IdlType::Primitive(PrimitiveType::Int8)), 0);

        // LC=1 for 2-byte types
        assert_eq!(compute_lc(&IdlType::Primitive(PrimitiveType::Short)), 1);
        assert_eq!(compute_lc(&IdlType::Primitive(PrimitiveType::UInt16)), 1);

        // LC=2 for 4-byte types
        assert_eq!(compute_lc(&IdlType::Primitive(PrimitiveType::Int32)), 2);
        assert_eq!(compute_lc(&IdlType::Primitive(PrimitiveType::Float)), 2);

        // LC=3 for 8-byte types
        assert_eq!(compute_lc(&IdlType::Primitive(PrimitiveType::Int64)), 3);
        assert_eq!(compute_lc(&IdlType::Primitive(PrimitiveType::Double)), 3);

        // LC=5 (NEXTINT) for variable-size types
        assert_eq!(compute_lc(&IdlType::Primitive(PrimitiveType::String)), 5);
        assert_eq!(
            compute_lc(&IdlType::Sequence {
                inner: Box::new(IdlType::Primitive(PrimitiveType::Int32)),
                bound: None
            }),
            5
        );
    }
}
