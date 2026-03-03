// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! CDR2 codec generation for C++
//!
//! Generates `encode_cdr2_le()` and `decode_cdr2_le()` methods for structs.
//!
//! Note: `uninlined_format_args` allowed in this module due to extensive `format!()`
//! usage in code generation that would require significant refactoring.

#![allow(clippy::uninlined_format_args)]

use super::super::keywords::cpp_ident;
use super::index::DefinitionIndex;
use crate::ast::{Field, Struct};
use crate::types::{Annotation, AutoIdKind, ExtensibilityKind, IdlType, PrimitiveType};
use std::fmt::Write;

mod decode;
mod encode;
pub mod pubsub_types;

/// Returns true if the struct has MUTABLE extensibility.
fn is_mutable(s: &Struct) -> bool {
    matches!(s.extensibility, Some(ExtensibilityKind::Mutable))
        || s.annotations.iter().any(|a| {
            matches!(
                a,
                Annotation::Extensibility(ExtensibilityKind::Mutable) | Annotation::Mutable
            )
        })
}

/// Returns true if the struct has APPENDABLE extensibility.
fn is_appendable(s: &Struct) -> bool {
    matches!(s.extensibility, Some(ExtensibilityKind::Appendable))
        || s.annotations.iter().any(|a| {
            matches!(
                a,
                Annotation::Extensibility(ExtensibilityKind::Appendable) | Annotation::Appendable
            )
        })
}

/// Compute member ID for a field in a mutable/appendable struct.
///
/// Priority:
/// - `@id` annotation on the field
/// - `@autoid(SEQUENTIAL)` on the struct -> use declaration order
/// - default/`@autoid(HASH)` -> FNV-1a 32-bit & `0x0FFF_FFFF` (`XTypes` ss7.3.1.2.1.2)
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

/// Get the fixed size of a type if it's a fixed-size primitive.
/// Returns None for variable-size types like strings, sequences, etc.
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
            PrimitiveType::String | PrimitiveType::WString | PrimitiveType::Void => None,
        },
        _ => None,
    }
}

/// Compute LC (Length Code) for EMHEADER based on field size.
/// LC values: 0=1byte, 1=2bytes, 2=4bytes, 3=8bytes, 5=NEXTINT follows
const fn compute_lc(ty: &IdlType) -> u32 {
    match cdr2_fixed_size(ty) {
        Some(1) => 0,
        Some(2) => 1,
        Some(4) => 2,
        Some(8) => 3,
        _ => 5, // Variable size: use NEXTINT
    }
}

#[derive(Copy, Clone)]
struct PrimitiveScalar {
    align: usize,
    width: usize,
}

const fn primitive_scalar_layout(prim: &PrimitiveType) -> Option<PrimitiveScalar> {
    match prim {
        PrimitiveType::Octet
        | PrimitiveType::UInt8
        | PrimitiveType::Int8
        | PrimitiveType::Boolean
        | PrimitiveType::Char => Some(PrimitiveScalar { align: 1, width: 1 }),
        PrimitiveType::Short
        | PrimitiveType::Int16
        | PrimitiveType::UnsignedShort
        | PrimitiveType::UInt16 => Some(PrimitiveScalar { align: 2, width: 2 }),
        PrimitiveType::Long
        | PrimitiveType::Int32
        | PrimitiveType::UnsignedLong
        | PrimitiveType::UInt32
        | PrimitiveType::Float
        | PrimitiveType::WChar => Some(PrimitiveScalar { align: 4, width: 4 }),
        PrimitiveType::LongLong
        | PrimitiveType::Int64
        | PrimitiveType::UnsignedLongLong
        | PrimitiveType::UInt64
        | PrimitiveType::Double
        | PrimitiveType::LongDouble => Some(PrimitiveScalar { align: 8, width: 8 }),
        PrimitiveType::Fixed { .. } => Some(PrimitiveScalar {
            align: 4,
            width: 16,
        }),
        PrimitiveType::String | PrimitiveType::WString | PrimitiveType::Void => None,
    }
}

/// Generates CDR2 encode/decode methods for a struct
pub(super) fn generate_struct_codec(s: &Struct, idx: &DefinitionIndex, indent: &str) -> String {
    generate_struct_codec_internal(s, idx, indent, false)
}

/// Generates CDR2 encode/decode methods for a struct with FastDDS-compatible m_ prefixed fields
pub(super) fn generate_struct_codec_fastdds(
    s: &Struct,
    idx: &DefinitionIndex,
    indent: &str,
) -> String {
    generate_struct_codec_internal(s, idx, indent, true)
}

fn generate_struct_codec_internal(
    s: &Struct,
    idx: &DefinitionIndex,
    indent: &str,
    fastdds_compat: bool,
) -> String {
    if is_mutable(s) {
        generate_mutable_codec(s, idx, indent, fastdds_compat)
    } else if is_appendable(s) {
        generate_appendable_codec(s, idx, indent, fastdds_compat)
    } else {
        generate_final_codec(s, idx, indent, fastdds_compat)
    }
}

/// Generate codec for FINAL structs (no DHEADER, current behavior).
fn generate_final_codec(
    s: &Struct,
    idx: &DefinitionIndex,
    indent: &str,
    fastdds_compat: bool,
) -> String {
    let mut out = String::new();

    // encode_cdr2_le method
    let _ = writeln!(
        out,
        "{indent}/// Encode this struct to CDR2 little-endian format"
    );
    let _ = writeln!(
        out,
        "{indent}/// Returns the number of bytes written, or -1 on error"
    );
    let _ = writeln!(
        out,
        "{indent}[[nodiscard]] int encode_cdr2_le(std::uint8_t* dst, std::size_t len) const noexcept {{"
    );
    let _ = writeln!(out, "{indent}    std::size_t offset = 0;");
    for field in &s.fields {
        out.push_str(&encode::emit_encode_field_compat(
            field,
            idx,
            &format!("{indent}    "),
            fastdds_compat,
        ));
    }
    let _ = writeln!(out, "{indent}    return static_cast<int>(offset);");
    let _ = writeln!(out, "{indent}}}\n");

    // decode_cdr2_le method
    let _ = writeln!(
        out,
        "{indent}/// Decode this struct from CDR2 little-endian format"
    );
    let _ = writeln!(
        out,
        "{indent}/// Returns the number of bytes read, or -1 on error"
    );
    let _ = writeln!(
        out,
        "{indent}[[nodiscard]] int decode_cdr2_le(const std::uint8_t* src, std::size_t len) noexcept {{"
    );
    let _ = writeln!(out, "{indent}    std::size_t offset = 0;");
    for field in &s.fields {
        out.push_str(&decode::emit_decode_field_compat(
            field,
            idx,
            &format!("{indent}    "),
            fastdds_compat,
        ));
    }
    let _ = writeln!(out, "{indent}    return static_cast<int>(offset);");
    let _ = writeln!(out, "{indent}}}");

    out
}

/// Generate codec for APPENDABLE structs (DHEADER prefix with payload size).
fn generate_appendable_codec(
    s: &Struct,
    idx: &DefinitionIndex,
    indent: &str,
    fastdds_compat: bool,
) -> String {
    let mut out = String::new();

    // encode_cdr2_le method with DHEADER
    let _ = writeln!(
        out,
        "{indent}/// Encode this struct to CDR2 little-endian format (APPENDABLE with DHEADER)"
    );
    let _ = writeln!(
        out,
        "{indent}/// Returns the number of bytes written, or -1 on error"
    );
    let _ = writeln!(
        out,
        "{indent}[[nodiscard]] int encode_cdr2_le(std::uint8_t* dst, std::size_t len) const noexcept {{"
    );
    let _ = writeln!(out, "{indent}    std::size_t offset = 0;");
    // Reserve space for DHEADER (4 bytes)
    let _ = writeln!(
        out,
        "{indent}    if (!cdr2::can_write(len, offset, 4)) return -1;"
    );
    let _ = writeln!(out, "{indent}    std::size_t dheader_pos = offset;");
    let _ = writeln!(out, "{indent}    offset += 4; // Reserve DHEADER");
    let _ = writeln!(out, "{indent}    std::size_t payload_start = offset;");

    // Encode all fields
    for field in &s.fields {
        out.push_str(&encode::emit_encode_field_compat(
            field,
            idx,
            &format!("{indent}    "),
            fastdds_compat,
        ));
    }

    // Write DHEADER with payload size
    let _ = writeln!(
        out,
        "{indent}    // Write DHEADER (payload size excluding DHEADER itself)"
    );
    let _ = writeln!(
        out,
        "{indent}    std::uint32_t payload_size = static_cast<std::uint32_t>(offset - payload_start);"
    );
    let _ = writeln!(
        out,
        "{indent}    std::memcpy(dst + dheader_pos, &payload_size, 4);"
    );
    let _ = writeln!(out, "{indent}    return static_cast<int>(offset);");
    let _ = writeln!(out, "{indent}}}\n");

    // decode_cdr2_le method with DHEADER
    let _ = writeln!(
        out,
        "{indent}/// Decode this struct from CDR2 little-endian format (APPENDABLE with DHEADER)"
    );
    let _ = writeln!(
        out,
        "{indent}/// Returns the number of bytes read, or -1 on error"
    );
    let _ = writeln!(
        out,
        "{indent}[[nodiscard]] int decode_cdr2_le(const std::uint8_t* src, std::size_t len) noexcept {{"
    );
    let _ = writeln!(out, "{indent}    std::size_t offset = 0;");
    // Read DHEADER
    let _ = writeln!(
        out,
        "{indent}    if (!cdr2::can_read(len, offset, 4)) return -1;"
    );
    let _ = writeln!(out, "{indent}    std::uint32_t payload_size;");
    let _ = writeln!(
        out,
        "{indent}    std::memcpy(&payload_size, src + offset, 4);"
    );
    let _ = writeln!(out, "{indent}    offset += 4;");
    let _ = writeln!(
        out,
        "{indent}    std::size_t payload_end = offset + payload_size;"
    );
    let _ = writeln!(
        out,
        "{indent}    if (payload_end > len) return -1; // Invalid DHEADER"
    );

    // Decode all fields
    for field in &s.fields {
        out.push_str(&decode::emit_decode_field_compat(
            field,
            idx,
            &format!("{indent}    "),
            fastdds_compat,
        ));
    }

    // Skip any remaining payload (for forward compatibility)
    let _ = writeln!(
        out,
        "{indent}    offset = payload_end; // Skip any unknown trailing fields"
    );
    let _ = writeln!(out, "{indent}    return static_cast<int>(offset);");
    let _ = writeln!(out, "{indent}}}");

    out
}

/// Generate codec for MUTABLE structs (DHEADER + EMHEADER per field).
#[allow(clippy::too_many_lines)]
fn generate_mutable_codec(
    s: &Struct,
    idx: &DefinitionIndex,
    indent: &str,
    fastdds_compat: bool,
) -> String {
    let mut out = String::new();

    // encode_cdr2_le method with DHEADER + EMHEADER
    let _ = writeln!(
        out,
        "{indent}/// Encode this struct to CDR2 little-endian format (MUTABLE with DHEADER + EMHEADER)"
    );
    let _ = writeln!(
        out,
        "{indent}/// Returns the number of bytes written, or -1 on error"
    );
    let _ = writeln!(
        out,
        "{indent}[[nodiscard]] int encode_cdr2_le(std::uint8_t* dst, std::size_t len) const noexcept {{"
    );
    let _ = writeln!(out, "{indent}    std::size_t offset = 0;");
    // Reserve space for DHEADER (4 bytes)
    let _ = writeln!(
        out,
        "{indent}    if (!cdr2::can_write(len, offset, 4)) return -1;"
    );
    let _ = writeln!(out, "{indent}    std::size_t dheader_pos = offset;");
    let _ = writeln!(out, "{indent}    offset += 4; // Reserve DHEADER");
    let _ = writeln!(out, "{indent}    std::size_t payload_start = offset;");

    // Encode each field with EMHEADER
    for (field_idx, field) in s.fields.iter().enumerate() {
        let member_id = compute_member_id(s, field_idx, field);
        let lc = compute_lc(&field.field_type);
        let use_nextint = lc == 5;
        let is_optional = field.is_optional();

        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "{indent}    // Field '{}': member_id={:#010X}, LC={}{}",
            field.name,
            member_id,
            lc,
            if is_optional { " (optional)" } else { "" }
        );

        // For optional fields in MUTABLE structs, skip the entire field if absent
        let escaped = cpp_ident(&field.name);
        let field_value_expr = if fastdds_compat {
            format!("this->m_{}", escaped)
        } else {
            format!("this->{}", escaped)
        };

        if is_optional {
            let _ = writeln!(out, "{indent}    if ({field_value_expr}.has_value()) {{");
        }

        let inner_indent = if is_optional {
            format!("{indent}        ")
        } else {
            format!("{indent}    ")
        };

        // Check buffer space for EMHEADER (+ NEXTINT if needed)
        if use_nextint {
            let _ = writeln!(
                out,
                "{inner_indent}if (!cdr2::can_write(len, offset, 8)) return -1;"
            );
        } else {
            let _ = writeln!(
                out,
                "{inner_indent}if (!cdr2::can_write(len, offset, 4)) return -1;"
            );
        }

        // Write EMHEADER: M (bit 31) | LC (bits 28-30) | Member ID (bits 0-27)
        let mu_bit = if field.is_key() || field.is_must_understand() {
            "0x80000000u | "
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "{inner_indent}std::uint32_t emheader_{idx} = {mu_bit}({lc}u << 28) | ({member_id:#010X}u & 0x0FFFFFFFu);",
            idx = field_idx,
            lc = lc,
            member_id = member_id
        );
        let _ = writeln!(
            out,
            "{inner_indent}std::memcpy(dst + offset, &emheader_{idx}, 4);",
            idx = field_idx
        );
        let _ = writeln!(out, "{inner_indent}offset += 4;");

        if use_nextint {
            // Reserve space for NEXTINT (member length)
            let _ = writeln!(
                out,
                "{inner_indent}std::size_t nextint_pos_{idx} = offset;",
                idx = field_idx
            );
            let _ = writeln!(
                out,
                "{inner_indent}offset += 4; // Reserve NEXTINT for member length"
            );
            let _ = writeln!(
                out,
                "{inner_indent}std::size_t member_start_{idx} = offset;",
                idx = field_idx
            );
        }

        // Encode the field value (for optional, use dereferenced value)
        if is_optional {
            let deref_expr = format!("(*{})", field_value_expr);
            out.push_str(&encode::emit_encode_type_for_mutable(
                &inner_indent,
                &field.field_type,
                idx,
                &deref_expr,
                &escaped,
            ));
        } else {
            out.push_str(&encode::emit_encode_type_for_mutable(
                &inner_indent,
                &field.field_type,
                idx,
                &field_value_expr,
                &escaped,
            ));
        }

        if use_nextint {
            // Write NEXTINT with member length
            let _ = writeln!(
                out,
                "{inner_indent}std::uint32_t member_len_{idx} = static_cast<std::uint32_t>(offset - member_start_{idx});",
                idx = field_idx
            );
            let _ = writeln!(
                out,
                "{inner_indent}std::memcpy(dst + nextint_pos_{idx}, &member_len_{idx}, 4);",
                idx = field_idx
            );
        }

        if is_optional {
            let _ = writeln!(out, "{indent}    }}");
        }
    }

    // Write DHEADER with payload size
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{indent}    // Write DHEADER (payload size excluding DHEADER itself)"
    );
    let _ = writeln!(
        out,
        "{indent}    std::uint32_t payload_size = static_cast<std::uint32_t>(offset - payload_start);"
    );
    let _ = writeln!(
        out,
        "{indent}    std::memcpy(dst + dheader_pos, &payload_size, 4);"
    );
    let _ = writeln!(out, "{indent}    return static_cast<int>(offset);");
    let _ = writeln!(out, "{indent}}}\n");

    // decode_cdr2_le method with DHEADER + EMHEADER
    out.push_str(&generate_mutable_decode(s, idx, indent, fastdds_compat));

    out
}

/// Generate decoder for MUTABLE structs.
#[allow(clippy::too_many_lines)]
fn generate_mutable_decode(
    s: &Struct,
    idx: &DefinitionIndex,
    indent: &str,
    fastdds_compat: bool,
) -> String {
    let mut out = String::new();

    let _ = writeln!(
        out,
        "{indent}/// Decode this struct from CDR2 little-endian format (MUTABLE with DHEADER + EMHEADER)"
    );
    let _ = writeln!(
        out,
        "{indent}/// Returns the number of bytes read, or -1 on error"
    );
    let _ = writeln!(
        out,
        "{indent}[[nodiscard]] int decode_cdr2_le(const std::uint8_t* src, std::size_t len) noexcept {{"
    );
    let _ = writeln!(out, "{indent}    std::size_t offset = 0;");

    // Read DHEADER
    let _ = writeln!(
        out,
        "{indent}    if (!cdr2::can_read(len, offset, 4)) return -1;"
    );
    let _ = writeln!(out, "{indent}    std::uint32_t payload_size;");
    let _ = writeln!(
        out,
        "{indent}    std::memcpy(&payload_size, src + offset, 4);"
    );
    let _ = writeln!(out, "{indent}    offset += 4;");
    let _ = writeln!(
        out,
        "{indent}    std::size_t payload_end = offset + payload_size;"
    );
    let _ = writeln!(
        out,
        "{indent}    if (payload_end > len) return -1; // Invalid DHEADER"
    );

    // Track which fields have been decoded
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{indent}    // Parse members by EMHEADER until payload exhausted"
    );
    let _ = writeln!(out, "{indent}    while (offset < payload_end) {{");
    let _ = writeln!(
        out,
        "{indent}        if (!cdr2::can_read(len, offset, 4)) return -1;"
    );
    let _ = writeln!(out, "{indent}        std::uint32_t emheader;");
    let _ = writeln!(
        out,
        "{indent}        std::memcpy(&emheader, src + offset, 4);"
    );
    let _ = writeln!(out, "{indent}        offset += 4;");
    let _ = writeln!(
        out,
        "{indent}        std::uint32_t lc = (emheader >> 28) & 0x7u;"
    );
    let _ = writeln!(
        out,
        "{indent}        std::uint32_t member_id = emheader & 0x0FFFFFFFu;"
    );

    // Compute member length based on LC
    let _ = writeln!(out);
    let _ = writeln!(out, "{indent}        std::size_t member_len = 0;");
    let _ = writeln!(out, "{indent}        switch (lc) {{");
    let _ = writeln!(
        out,
        "{indent}            case 0: member_len = 1; break; // 1 byte"
    );
    let _ = writeln!(
        out,
        "{indent}            case 1: member_len = 2; break; // 2 bytes"
    );
    let _ = writeln!(
        out,
        "{indent}            case 2: member_len = 4; break; // 4 bytes"
    );
    let _ = writeln!(
        out,
        "{indent}            case 3: member_len = 8; break; // 8 bytes"
    );
    let _ = writeln!(out, "{indent}            case 5: {{ // NEXTINT follows");
    let _ = writeln!(
        out,
        "{indent}                if (!cdr2::can_read(len, offset, 4)) return -1;"
    );
    let _ = writeln!(out, "{indent}                std::uint32_t nextint;");
    let _ = writeln!(
        out,
        "{indent}                std::memcpy(&nextint, src + offset, 4);"
    );
    let _ = writeln!(out, "{indent}                offset += 4;");
    let _ = writeln!(out, "{indent}                member_len = nextint;");
    let _ = writeln!(out, "{indent}                break;");
    let _ = writeln!(out, "{indent}            }}");
    let _ = writeln!(out, "{indent}            default:");
    let _ = writeln!(out, "{indent}                return -1; // Unsupported LC");
    let _ = writeln!(out, "{indent}        }}");

    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{indent}        std::size_t member_end = offset + member_len;"
    );
    let _ = writeln!(
        out,
        "{indent}        if (member_end > payload_end) return -1; // Member exceeds payload"
    );

    // Switch on member_id to decode the right field
    let _ = writeln!(out);
    let _ = writeln!(out, "{indent}        switch (member_id) {{");

    for (field_idx, field) in s.fields.iter().enumerate() {
        let escaped = cpp_ident(&field.name);
        let member_id = compute_member_id(s, field_idx, field);
        let is_optional = field.is_optional();
        let _ = writeln!(
            out,
            "{indent}            case {member_id:#010X}u: {{ // {}{}",
            field.name,
            if is_optional { " (optional)" } else { "" }
        );

        // For MUTABLE structs with optional fields, decode directly into the optional
        // (EMHEADER presence means value exists)
        if is_optional {
            // Decode into a temp, then assign to the optional
            let inner_type = super::helpers::type_to_cpp(&field.field_type);
            let field_expr = if fastdds_compat {
                format!("this->m_{}", escaped)
            } else {
                format!("this->{}", escaped)
            };
            let _ = writeln!(
                out,
                "{indent}                {inner_type} tmp_{}{{}};\n",
                escaped
            );
            let field_decode = decode::emit_decode_type_for_mutable(
                &format!("{indent}                "),
                &field.field_type,
                idx,
                &format!("tmp_{}", escaped),
                &escaped,
            );
            out.push_str(&field_decode);
            let _ = writeln!(
                out,
                "{indent}                {field_expr} = std::move(tmp_{name});",
                name = escaped
            );
        } else {
            let field_decode = decode::emit_decode_field_compat(
                field,
                idx,
                &format!("{indent}                "),
                fastdds_compat,
            );
            out.push_str(&field_decode);
        }

        let _ = writeln!(out, "{indent}                break;");
        let _ = writeln!(out, "{indent}            }}");
    }

    // Skip unknown members (forward compatibility)
    let _ = writeln!(out, "{indent}            default:");
    let _ = writeln!(out, "{indent}                // Unknown member: skip it");
    let _ = writeln!(out, "{indent}                break;");
    let _ = writeln!(out, "{indent}        }}");

    // Advance to member_end (handles any padding or unused bytes)
    let _ = writeln!(out, "{indent}        offset = member_end;");
    let _ = writeln!(out, "{indent}    }}");

    let _ = writeln!(out);
    let _ = writeln!(out, "{indent}    return static_cast<int>(offset);");
    let _ = writeln!(out, "{indent}}}");

    out
}

/// Generates the CDR2 helper functions header (inline in generated header)
pub(super) fn generate_cdr2_helpers() -> String {
    r"// CDR2 serialization helpers (with include guard to allow multiple IDL files)
#ifndef HDDS_CDR2_HELPERS_DEFINED
#define HDDS_CDR2_HELPERS_DEFINED
namespace cdr2 {

inline std::size_t align_offset(std::size_t offset, std::size_t alignment) noexcept {
    return (offset + alignment - 1) & ~(alignment - 1);
}

inline bool can_write(std::size_t len, std::size_t offset, std::size_t bytes) noexcept {
    return offset + bytes <= len;
}

inline bool can_read(std::size_t len, std::size_t offset, std::size_t bytes) noexcept {
    return offset + bytes <= len;
}

template<typename T>
inline void write_le(std::uint8_t* dst, std::size_t& offset, T value) noexcept {
    std::memcpy(dst + offset, &value, sizeof(T));
    offset += sizeof(T);
}

template<typename T>
inline T read_le(const std::uint8_t* src, std::size_t& offset) noexcept {
    T value;
    std::memcpy(&value, src + offset, sizeof(T));
    offset += sizeof(T);
    return value;
}

}  // namespace cdr2
#endif // HDDS_CDR2_HELPERS_DEFINED

"
    .to_string()
}
