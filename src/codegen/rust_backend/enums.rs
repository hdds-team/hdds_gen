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

        // Enum definition (the wire holder width follows @bit_bound)
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
            format_args!(
                "{indent}#[allow(non_camel_case_types, clippy::upper_case_acronyms, clippy::enum_variant_names)]\n"
            ),
        );
        let values = e.resolved_values();
        // A negative discriminant needs a signed representation.
        let repr = if values.iter().any(|v| *v < 0) {
            "i32"
        } else {
            "u32"
        };
        push_fmt(&mut output, format_args!("{indent}#[repr({repr})]\n"));
        let name = &e.name;
        push_fmt(&mut output, format_args!("{indent}pub enum {name} {{\n"));

        for (idx, (variant, val)) in e.variants.iter().zip(values).enumerate() {
            // Mark first variant as default
            if idx == 0 {
                push_fmt(&mut output, format_args!("{indent}    #[default]\n"));
            }
            let vname = &variant.name;
            push_fmt(&mut output, format_args!("{indent}    {vname} = {val},\n"));
        }

        push_fmt(&mut output, format_args!("{indent}}}\n\n"));

        // `__hdds_from_i32` and the standalone codec: the value as an
        // integer of the `@bit_bound` width (`XTypes` 1.3 7.3.1.2.1.6).
        output.push_str(&super::codec::emit_enum_codec(e, &indent));

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
