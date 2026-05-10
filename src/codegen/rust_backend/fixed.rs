// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Fixed-point decimal support for Rust.
//!
//! Emits the `Fixed<D, S>` struct template for IDL fixed types.

use super::RustGenerator;

impl RustGenerator {
    pub(super) fn emit_fixed_support(output: &mut String) {
        output.push_str("// Fixed-point decimal with const generics (digits, scale)\n");
        output.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n");
        output.push_str("pub struct Fixed<const D: u32, const S: u32>(pub i128);\n");
        output.push_str("impl<const D: u32, const S: u32> Fixed<D, S> {\n");
        output.push_str("    #[inline] pub const fn from_raw(v: i128) -> Self { Self(v) }\n");
        output.push_str("    #[inline] pub const fn raw(&self) -> i128 { self.0 }\n");
        output.push_str(
            "    #[inline] pub const fn pow10() -> i128 { let mut p: i128 = 1; let mut i = 0; while i < S { p *= 10; i += 1; } p }\n",
        );
        output.push_str("    /// Build from integer and fractional parts (frac in [0, 10^S))\n");
        output.push_str(
            "    pub const fn from_parts(int: i128, frac: i128) -> Self { let base = Self::pow10(); let sign = if int < 0 { -1 } else { 1 }; Self(int * base + (sign as i128) * frac) }\n",
        );
        output.push_str(
            "    #[inline] pub fn to_f64(self) -> f64 { (self.0 as f64) / (Self::pow10() as f64) }\n",
        );
        output.push_str("}\n\n");
        output.push_str("impl<const D: u32, const S: u32> core::fmt::Display for Fixed<D, S> {\n");
        output.push_str(
            "    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {\n",
        );
        output.push_str(
            "        let base = Self::pow10();\n        let neg = self.0 < 0;\n        let mut v = if neg { -self.0 } else { self.0 };\n        let intp = v / base;\n        let fracp = (v % base) as u128;\n        if S == 0 { if neg { write!(f, \"-{}\", intp) } else { write!(f, \"{}\", intp) } } else { let width = S as usize; if neg { write!(f, \"-{}.{:0width$}\", intp, fracp) } else { write!(f, \"{}.{:0width$}\", intp, fracp) } }\n    }\n}",
        );
        output.push('\n');
        // 2.2-c: emit inherent `encode_xcdrN_le` / `decode_xcdrN_le` on
        // `Fixed<D, S>` so outer struct and union codegen can dispatch
        // sub-field calls via the versioned name regardless of wire format.
        // Fixed is version-invariant on the wire: hddsgen always encodes it
        // as 16 raw bytes via `self.raw().to_le_bytes()`, aligned to 4 by
        // the outer (`xcdr_alignment` reports 4 for `PrimitiveType::Fixed`
        // in both XCDR v1 and XCDR v2), so the two inherent bodies share
        // the same implementation. The `Cdr2Encode` / `Cdr2Decode` trait
        // delegators are emitted manually below (they can't go through
        // `RustGenerator::emit_cdr_trait_delegator` because `Fixed<D, S>`
        // carries const generics that helper does not format).
        for (suffix, _version) in [("xcdr1", "Xcdr1"), ("xcdr2", "Xcdr2")] {
            output.push_str("impl<const D: u32, const S: u32> Fixed<D, S> {\n");
            output.push_str(&format!(
                "    pub fn encode_{suffix}_le(&self, dst: &mut [u8]) -> Result<usize, CdrError> {{\n"
            ));
            output
                .push_str("        if dst.len() < 16 { return Err(CdrError::BufferTooSmall); }\n");
            output.push_str("        dst[..16].copy_from_slice(&self.raw().to_le_bytes());\n");
            output.push_str("        Ok(16)\n");
            output.push_str("    }\n");
            output.push_str(&format!(
                "    pub fn max_{suffix}_size(&self) -> usize {{ 16 }}\n"
            ));
            output.push_str(&format!(
                "    pub fn decode_{suffix}_le(src: &[u8]) -> Result<(Self, usize), CdrError> {{\n"
            ));
            output.push_str("        if src.len() < 16 { return Err(CdrError::UnexpectedEof); }\n");
            output.push_str(
                "        let raw = {\n            let mut __hdds_tmp = [0u8; 16];\n            __hdds_tmp.copy_from_slice(&src[..16]);\n            i128::from_le_bytes(__hdds_tmp)\n        };\n",
            );
            output.push_str("        Ok((Fixed::<D, S>::from_raw(raw), 16))\n");
            output.push_str("    }\n");
            // Inherent encode_xcdrN_le_at trivial wrapper. Mirrors the helper
            // emitted by Self::emit_encode_at_wrapper but inlined here because
            // Fixed<D, S> carries const generics that the shared helper
            // signature does not format. Keep in sync with the shared helper.
            output.push_str(&format!(
                "    pub fn encode_{suffix}_le_at(\n        &self,\n        dst: &mut [u8],\n        offset: &mut usize,\n    ) -> Result<(), CdrError> {{\n"
            ));
            output.push_str(&format!(
                "        let len = self.encode_{suffix}_le(&mut dst[*offset..])?;\n"
            ));
            output.push_str("        *offset += len;\n");
            output.push_str("        Ok(())\n");
            output.push_str("    }\n");
            output.push_str("}\n\n");
        }

        // Manual Cdr2Encode / Cdr2Decode delegators for the generic Fixed<D, S>.
        // emit_cdr_trait_delegator can't be used here because Fixed<D, S>
        // carries const generics that helper does not format.
        output.push_str("impl<const D: u32, const S: u32> Cdr2Encode for Fixed<D, S> {\n");
        output.push_str(
            "    fn encode_cdr2_le(&self, dst: &mut [u8]) -> Result<usize, CdrError> {\n",
        );
        output.push_str("        self.encode_xcdr2_le(dst)\n");
        output.push_str("    }\n");
        output.push_str("    fn max_cdr2_size(&self) -> usize { self.max_xcdr2_size() }\n");
        output.push_str(
            "    fn encode_cdr2_le_at(&self, dst: &mut [u8], offset: &mut usize) -> Result<(), CdrError> {\n",
        );
        output.push_str("        let len = self.encode_xcdr2_le(&mut dst[*offset..])?;\n");
        output.push_str("        *offset += len;\n");
        output.push_str("        Ok(())\n");
        output.push_str("    }\n");
        output.push_str("}\n\n");
        output.push_str("impl<const D: u32, const S: u32> Cdr2Decode for Fixed<D, S> {\n");
        output.push_str("    fn decode_cdr2_le(src: &[u8]) -> Result<(Self, usize), CdrError> {\n");
        output.push_str("        Self::decode_xcdr2_le(src)\n");
        output.push_str("    }\n");
        output.push_str("}\n");
        output.push('\n');
        output.push_str("impl<const D: u32, const S: u32> Default for Fixed<D, S> {\n");
        output.push_str("    fn default() -> Self { Self::from_raw(0) }\n");
        output.push_str("}\n");
    }
}
