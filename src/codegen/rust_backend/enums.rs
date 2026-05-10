// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Enum code generation for Rust.
//!
//! Generates Rust enum definitions from IDL enum types with CDR2 encoding.

use super::{push_fmt, RustGenerator};
use crate::ast::Enum;

impl RustGenerator {
    // codegen function - line count from template output
    #[allow(clippy::too_many_lines)]
    pub(super) fn generate_enum(&self, e: &Enum) -> String {
        let mut output = String::new();
        let indent = self.indent();

        // Enum definition with repr(u32) for CDR compatibility
        let serde_derive = self.serde_derive();
        let serde_rename = self.serde_rename_attr();
        push_fmt(
            &mut output,
            format_args!(
                "{indent}#[derive(Debug, Clone, Copy, PartialEq, Eq, Default{serde_derive})]\n"
            ),
        );
        if !serde_rename.is_empty() {
            push_fmt(&mut output, format_args!("{indent}{serde_rename}"));
        }
        push_fmt(
            &mut output,
            format_args!("{indent}#[allow(non_camel_case_types)]\n"),
        );
        push_fmt(&mut output, format_args!("{indent}#[repr(u32)]\n"));
        let name = &e.name;
        push_fmt(&mut output, format_args!("{indent}pub enum {name} {{\n"));

        for (idx, variant) in e.variants.iter().enumerate() {
            // @audit-ok: safe cast - enum variant index always << i64::MAX
            #[allow(clippy::cast_possible_wrap)]
            let val = variant.value.unwrap_or(idx as i64);
            // Mark first variant as default
            if idx == 0 {
                push_fmt(&mut output, format_args!("{indent}    #[default]\n"));
            }
            let vname = &variant.name;
            push_fmt(&mut output, format_args!("{indent}    {vname} = {val},\n"));
        }

        push_fmt(&mut output, format_args!("{indent}}}\n\n"));

        // 2.2-c: emit inherent `encode_xcdrN_le` / `max_xcdrN_size` /
        // `decode_xcdrN_le` methods plus a Cdr2Encode / Cdr2Decode trait
        // delegator so outer types -- which in 2.2-d started dispatching
        // sub-field encoders via `.encode_xcdrN_le(...)` -- can reach the
        // enum locally. The wire encoding itself is version-invariant
        // here: hddsgen currently emits every enum as a 32-bit integer
        // (`#[repr(u32)]` + 4-byte LE payload), which aligns to 4 in both
        // XCDR v1 (Table 31) and XCDR v2 (same, since <= maxalign=4).
        // The two inherent methods therefore share the same body, but are
        // both emitted to preserve the "every type offers both versions"
        // invariant established in 2.2-a.
        for &version in super::helpers::VERSIONS_TO_EMIT {
            let suffix = super::helpers::xcdr_method_suffix(version);
            push_fmt(&mut output, format_args!("{indent}impl {name} {{\n"));
            push_fmt(
                &mut output,
                format_args!(
                    "{indent}    pub fn encode_{suffix}_le(&self, dst: &mut [u8]) -> Result<usize, CdrError> {{\n"
                ),
            );
            push_fmt(
                &mut output,
                format_args!(
                    "{indent}        if dst.len() < 4 {{ return Err(CdrError::BufferTooSmall); }}\n"
                ),
            );
            push_fmt(
                &mut output,
                format_args!(
                    "{indent}        dst[..4].copy_from_slice(&(*self as u32).to_le_bytes());\n"
                ),
            );
            push_fmt(&mut output, format_args!("{indent}        Ok(4)\n"));
            push_fmt(&mut output, format_args!("{indent}    }}\n"));
            push_fmt(
                &mut output,
                format_args!("{indent}    pub fn max_{suffix}_size(&self) -> usize {{ 4 }}\n"),
            );
            push_fmt(
                &mut output,
                format_args!(
                    "{indent}    pub fn decode_{suffix}_le(src: &[u8]) -> Result<(Self, usize), CdrError> {{\n"
                ),
            );
            push_fmt(
                &mut output,
                format_args!(
                    "{indent}        if src.len() < 4 {{ return Err(CdrError::UnexpectedEof); }}\n"
                ),
            );
            push_fmt(
                &mut output,
                format_args!(
                    "{indent}        let v = u32::from_le_bytes([src[0], src[1], src[2], src[3]]);\n"
                ),
            );
            push_fmt(&mut output, format_args!("{indent}        match v {{\n"));
            for variant in &e.variants {
                #[allow(clippy::cast_possible_wrap)]
                let val = variant.value.unwrap_or_else(|| {
                    e.variants
                        .iter()
                        .position(|v| v.name == variant.name)
                        .unwrap_or(0) as i64
                });
                let vname = &variant.name;
                push_fmt(
                    &mut output,
                    format_args!("{indent}            {val} => Ok((Self::{vname}, 4)),\n"),
                );
            }
            push_fmt(
                &mut output,
                format_args!("{indent}            _ => Err(CdrError::InvalidEncoding),\n"),
            );
            push_fmt(&mut output, format_args!("{indent}        }}\n"));
            push_fmt(&mut output, format_args!("{indent}    }}\n"));
            output.push_str(&Self::emit_encode_at_wrapper(suffix, &indent));
            push_fmt(&mut output, format_args!("{indent}}}\n\n"));
        }

        // Trait delegator (primary = Xcdr2 always for enums: no
        // @data_representation annotation is parsed onto enums today, and
        // the body is version-invariant anyway, so the choice is cosmetic).
        push_fmt(
            &mut output,
            format_args!(
                "{}",
                Self::emit_cdr_trait_delegator(name, super::helpers::CdrVersion::Xcdr2)
            ),
        );

        output
    }
}
