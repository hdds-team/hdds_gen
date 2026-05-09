// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Bitset code generation for Rust.
//!
//! Generates Rust struct representations of IDL bitset types with accessor methods.

use super::{push_fmt, RustGenerator};
use crate::ast::{BitfieldDecl, Bitset};
use crate::types::Annotation;

impl RustGenerator {
    pub(super) fn generate_bitset(&self, b: &Bitset) -> String {
        let mut output = String::new();
        self.emit_bitset_declaration(&mut output, b);
        self.emit_bitset_impl(&mut output, b);
        self.emit_bitset_default_impl(&mut output, b);
        self.emit_bitset_cdr2_impls(&mut output, b);
        output
    }

    fn emit_bitset_declaration(&self, dst: &mut String, b: &Bitset) {
        let indent = self.indent();
        let serde_derive = self.serde_derive();
        let serde_rename = self.serde_rename_attr();
        push_fmt(
            dst,
            format_args!("{indent}#[derive(Debug, Clone, Copy, PartialEq, Eq{serde_derive})]\n"),
        );
        if !serde_rename.is_empty() {
            push_fmt(dst, format_args!("{indent}{serde_rename}"));
        }
        let name = &b.name;
        push_fmt(
            dst,
            format_args!("{indent}pub struct {name} {{ pub bits: u64 }}\n"),
        );
    }

    fn emit_bitset_impl(&self, dst: &mut String, b: &Bitset) {
        let indent = self.indent();
        let name = &b.name;
        push_fmt(dst, format_args!("{indent}impl {name} {{\n"));

        let mut next_pos = 0;
        for field in &b.fields {
            let start = Self::bitset_field_position(field, &mut next_pos);
            self.emit_bitset_field(dst, field, start);
        }

        self.emit_bitset_common_helpers(dst);
        push_fmt(dst, format_args!("{indent}}}\n\n"));
    }

    fn bitset_field_position(field: &BitfieldDecl, next_pos: &mut u32) -> u32 {
        field
            .annotations
            .iter()
            .find_map(|ann| match ann {
                Annotation::Position(p) => Some(*p),
                _ => None,
            })
            .unwrap_or_else(|| {
                let start = *next_pos;
                *next_pos += field.width;
                start
            })
    }

    fn emit_bitset_field(&self, dst: &mut String, field: &BitfieldDecl, start: u32) {
        self.emit_bitset_field_docs(dst, field, start);
        self.emit_bitset_field_constants(dst, field, start);
        self.emit_bitset_field_getter(dst, field);
        self.emit_bitset_field_setter(dst, field);
        self.emit_bitset_field_builder(dst, field);
    }

    fn emit_bitset_field_docs(&self, dst: &mut String, field: &BitfieldDecl, start: u32) {
        let indent = self.indent();
        let name = &field.name;
        let width = field.width;
        push_fmt(
            dst,
            format_args!("{indent}    /// Field {name}: width {width} at bit {start}\n"),
        );
    }

    fn emit_bitset_field_constants(&self, dst: &mut String, field: &BitfieldDecl, start: u32) {
        let indent = self.indent();
        let upper = field.name.to_uppercase();
        let width = field.width;
        push_fmt(
            dst,
            format_args!("{indent}    pub const {upper}_SHIFT: u32 = {start};\n"),
        );
        push_fmt(
            dst,
            format_args!(
                "{indent}    pub const {upper}_MASK: u64 = ((1u64 << {width}) - 1) << Self::{upper}_SHIFT;\n"
            ),
        );
    }

    fn emit_bitset_field_getter(&self, dst: &mut String, field: &BitfieldDecl) {
        let indent = self.indent();
        let upper = field.name.to_uppercase();
        let name = &field.name;
        push_fmt(
            dst,
            format_args!(
                "{indent}    #[inline]\n{indent}    pub fn {name}(&self) -> u64 {{\n{indent}        (self.bits & Self::{upper}_MASK) >> Self::{upper}_SHIFT\n{indent}    }}\n"
            ),
        );
    }

    fn emit_bitset_field_setter(&self, dst: &mut String, field: &BitfieldDecl) {
        let indent = self.indent();
        let upper = field.name.to_uppercase();
        let name = &field.name;
        let width = field.width;
        push_fmt(
            dst,
            format_args!(
                "{indent}    #[inline]\n{indent}    pub fn set_{name}(&mut self, value: u64) {{\n{indent}        let v = (value & ((1u64 << {width}) - 1)) << Self::{upper}_SHIFT;\n{indent}        self.bits = (self.bits & !Self::{upper}_MASK) | v;\n{indent}    }}\n"
            ),
        );
    }

    fn emit_bitset_field_builder(&self, dst: &mut String, field: &BitfieldDecl) {
        let indent = self.indent();
        let name = &field.name;
        push_fmt(
            dst,
            format_args!(
                "{indent}    #[inline]\n{indent}    pub fn with_{name}(mut self, value: u64) -> Self {{\n{indent}        self.set_{name}(value);\n{indent}        self\n{indent}    }}\n"
            ),
        );
    }

    fn emit_bitset_default_impl(&self, dst: &mut String, b: &Bitset) {
        let indent = self.indent();
        let name = &b.name;
        push_fmt(dst, format_args!("\n{indent}impl Default for {name} {{\n"));
        push_fmt(
            dst,
            format_args!("{indent}    fn default() -> Self {{ Self::zero() }}\n"),
        );
        push_fmt(dst, format_args!("{indent}}}\n"));
    }

    fn emit_bitset_cdr2_impls(&self, dst: &mut String, b: &Bitset) {
        let indent = self.indent();
        let name = &b.name;
        // 2.2-c: hddsgen emits every bitset with a u64 storage regardless of
        // the declared bit-bound (8-byte payload, `dst[..8]`), so the wire
        // encoding is version-invariant from the bitset's own perspective:
        // the outer struct is responsible for aligning the 8 bytes before
        // calling the bitset encoder, and that outer alignment is handled
        // by the struct codegen (`xcdr_alignment` treats Named = 4 in both
        // versions, which matches the XCDR2 MALIGN cap; the wire u64 payload
        // is then placed at whatever offset the outer reached).
        //
        // Both inherent methods therefore share the same body, but both are
        // emitted to preserve the "every type offers encode_xcdrN_le" rule.
        // When a future codegen decision splits bitset storage by bit-bound
        // (e.g. @bit_bound(64) staying u64 but @bit_bound(32) shrinking to
        // u32 with 4-byte payload), this helper will need per-version
        // divergence; flag inline as a TODO until the split lands.
        for &version in super::helpers::VERSIONS_TO_EMIT {
            let suffix = super::helpers::xcdr_method_suffix(version);
            push_fmt(dst, format_args!("\n{indent}impl {name} {{\n"));
            push_fmt(
                dst,
                format_args!(
                    "{indent}    pub fn encode_{suffix}_le(&self, dst: &mut [u8]) -> Result<usize, CdrError> {{\n"
                ),
            );
            push_fmt(dst, format_args!("{indent}        if dst.len() < 8 {{\n"));
            push_fmt(
                dst,
                format_args!("{indent}            return Err(CdrError::BufferTooSmall);\n"),
            );
            push_fmt(dst, format_args!("{indent}        }}\n"));
            push_fmt(
                dst,
                format_args!(
                    "{indent}        dst[..8].copy_from_slice(&self.bits.to_le_bytes());\n"
                ),
            );
            push_fmt(dst, format_args!("{indent}        Ok(8)\n"));
            push_fmt(dst, format_args!("{indent}    }}\n"));
            push_fmt(
                dst,
                format_args!("{indent}    pub fn max_{suffix}_size(&self) -> usize {{ 8 }}\n"),
            );
            push_fmt(
                dst,
                format_args!(
                    "{indent}    pub fn decode_{suffix}_le(src: &[u8]) -> Result<(Self, usize), CdrError> {{\n"
                ),
            );
            push_fmt(dst, format_args!("{indent}        if src.len() < 8 {{\n"));
            push_fmt(
                dst,
                format_args!("{indent}            return Err(CdrError::UnexpectedEof);\n"),
            );
            push_fmt(dst, format_args!("{indent}        }}\n"));
            push_fmt(
                dst,
                format_args!("{indent}        let mut bytes = [0u8; 8];\n"),
            );
            push_fmt(
                dst,
                format_args!("{indent}        bytes.copy_from_slice(&src[..8]);\n"),
            );
            push_fmt(
                dst,
                format_args!(
                    "{indent}        Ok((Self {{ bits: u64::from_le_bytes(bytes) }}, 8))\n"
                ),
            );
            push_fmt(dst, format_args!("{indent}    }}\n"));
            push_fmt(dst, format_args!("{indent}}}\n"));
        }

        // Trait delegator (primary = Xcdr2 always: the body is version-
        // invariant today, so the choice only affects which inherent method
        // `Cdr2Encode::encode_cdr2_le` and `encode_cdr2_le_at`
        // (DDS-XTypes v1.3 §7.4.3.4.1 Tab.15) delegate to -- cosmetic).
        dst.push('\n');
        dst.push_str(&Self::emit_cdr_trait_delegator(
            name,
            super::helpers::CdrVersion::Xcdr2,
        ));
    }

    fn emit_bitset_common_helpers(&self, dst: &mut String) {
        let indent = self.indent();
        push_fmt(
            dst,
            format_args!(
                "{indent}    #[inline]\n{indent}    pub const fn zero() -> Self {{ Self {{ bits: 0 }} }}\n"
            ),
        );
        push_fmt(
            dst,
            format_args!(
                "{indent}    #[inline]\n{indent}    pub const fn from_bits(bits: u64) -> Self {{ Self {{ bits }} }}\n"
            ),
        );
        push_fmt(
            dst,
            format_args!(
                "{indent}    #[inline]\n{indent}    pub const fn bits(&self) -> u64 {{ self.bits }}\n"
            ),
        );
    }
}
