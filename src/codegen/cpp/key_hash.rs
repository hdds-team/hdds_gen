// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Instance `KeyHash` of generated C++ types (RTPS 2.5 9.6.4.8), through the
//! C runtime the C backend emits (`hdds_key_holder`, `hdds_kh_*`).

use std::fmt::Write as _;

use super::super::c::key_hash::{runtime, CFlavor, Emit};
use super::super::keywords::cpp_ident;
use super::index::DefinitionIndex;
use crate::ast::Struct;
use crate::types::IdlType;

/// The C runtime, emitted once after the includes of a header with a keyed
/// type.
pub(super) fn runtime_section() -> String {
    format!("{}\n", runtime())
}

/// How this backend's C++ types hold `KeyHolder` values: base members are
/// inherited, strings are `std::string` / `std::wstring`, bounded strings
/// `std::array<char, N>` read up to their first NUL, bounded sequences
/// `std::array`, unbounded ones `std::vector`; a union keeps `_d` and its
/// members either in `_u` or, when one is not trivially copyable, directly.
/// In FastDDS-compatible mode every struct member is read through its
/// getter.
struct CppTypes<'a, 'b> {
    idx: &'a DefinitionIndex<'b>,
    fastdds_compat: bool,
}

impl CFlavor for CppTypes<'_, '_> {
    fn member(&self, parent: &str, _base_depth: usize, name: &str) -> Option<String> {
        let ident = cpp_ident(name);
        Some(if self.fastdds_compat {
            format!("{parent}.{ident}()")
        } else {
            format!("{parent}.{ident}")
        })
    }

    fn string(&self, place: &str, bounded: bool, wide: bool) -> Option<String> {
        Some(match (bounded, wide) {
            (false, false) => format!("hdds_kh_string_n(w, {place}.data(), {place}.size());"),
            (false, true) => format!("hdds_kh_wstring_n(w, {place}.data(), {place}.size());"),
            (true, false) => format!(
                "hdds_kh_string_n(w, {place}.data(), hdds_kh_strnlen({place}.data(), {place}.size()));"
            ),
            (true, true) => format!(
                "hdds_kh_wstring_n(w, {place}.data(), hdds_kh_wcsnlen({place}.data(), {place}.size()));"
            ),
        })
    }

    fn sequence(&self, place: &str, _bounded: bool, i: &str) -> (String, String) {
        (format!("{place}.size()"), format!("{place}[{i}]"))
    }

    fn union_disc(&self, place: &str) -> String {
        format!("{place}._d")
    }

    fn union_member(&self, place: &str, union_name: &str, name: &str) -> String {
        let direct = self
            .idx
            .unions
            .get(union_name)
            .is_some_and(|u| super::unions::has_nontrivial_case(u, self.idx));
        if direct {
            format!("{place}.{name}")
        } else {
            format!("{place}._u.{name}")
        }
    }

    fn label(&self, disc: &IdlType, label: &str) -> String {
        super::unions::label_to_cpp(self.idx, disc, label)
    }
}

/// Static key operations preserve the historical two-argument callback type.
pub(super) fn pubsub_key_methods(
    s: &Struct,
    idx: &DefinitionIndex,
    fastdds_compat: bool,
) -> String {
    key_methods(s, idx, fastdds_compat, false)
}

/// Instance operations consumed by the typed SDK without a `PubSubType` lookup.
pub(super) fn instance_key_methods(
    s: &Struct,
    idx: &DefinitionIndex,
    fastdds_compat: bool,
) -> String {
    key_methods(s, idx, fastdds_compat, true)
}

fn key_methods(s: &Struct, idx: &DefinitionIndex, fastdds_compat: bool, instance: bool) -> String {
    let type_name = &s.name;
    let keyed = idx.keys.is_keyed(s);
    let mut out = String::new();
    out.push_str("    /// Instance KeyHash: normal PLAIN_CDR2, or MD5 of CDR BE key fields\n    /// for payload protection (RTPS 2.5 9.6.4.8; DDS-Security 1.1 7.3.4).\n");
    if instance {
        out.push_str("    bool hdds_compute_key(std::uint8_t key_buffer[16], bool protected_key = false) const noexcept {\n");
    } else {
        out.push_str("    static bool compute_key_with_protection(const void* data, std::uint8_t key_buffer[16], bool protected_key) noexcept {\n");
    }
    let emitted: Result<(String, String, bool), Option<String>> = match idx.keys.key_holder(s) {
        Ok(Some(kh)) => {
            let flavor = CppTypes {
                idx,
                fastdds_compat,
            };
            let mut e = Emit::new(&flavor);
            e.structure(2, &kh, "(*typed)")
                .map(|()| {
                    (
                        e.declarations(),
                        e.statements().to_string(),
                        kh.is_padded_form(),
                    )
                })
                .map_err(Some)
        }
        Ok(None) => Err(None),
        Err(reason) => Err(Some(reason)),
    };
    match emitted {
        Ok((decls, body, padded)) => {
            if instance {
                out.push_str(
                    "        if (!key_buffer) return false;\n        const auto* typed = this;\n",
                );
            } else {
                out.push_str("        if (!data || !key_buffer) return false;\n");
                let _ = writeln!(
                    out,
                    "        const auto* typed = static_cast<const {type_name}*>(data);"
                );
            }
            out.push_str("        hdds_key_holder kh;\n        hdds_key_holder* w = &kh;\n");
            if !decls.is_empty() {
                let _ = writeln!(out, "        {decls}");
            }
            out.push_str("        hdds_kh_init(w); w->max_align = protected_key ? 8u : 4u;\n");
            out.push_str(&body);
            let _ = writeln!(
                out,
                "        hdds_kh_finish(w, protected_key ? 0 : {}, key_buffer);",
                i32::from(padded)
            );
            out.push_str("        return true;\n");
        }
        Err(reason) => {
            if let Some(reason) = reason {
                let reason = reason.replace(['\n', '\r'], " ");
                let _ = writeln!(
                    out,
                    "        // KeyHolder serialization is not supported: {reason}"
                );
            }
            if !instance {
                out.push_str("        (void)data;\n");
            }
            out.push_str("        (void)protected_key;\n        if (key_buffer) std::memset(key_buffer, 0, 16);\n        return false;\n");
        }
    }
    out.push_str("    }\n\n");
    if !instance {
        out.push_str("    static bool compute_key(const void* data, std::uint8_t key_buffer[16]) noexcept {\n        return compute_key_with_protection(data, key_buffer, false);\n    }\n\n    static bool compute_protected_key(const void* data, std::uint8_t key_buffer[16]) noexcept {\n        return compute_key_with_protection(data, key_buffer, true);\n    }\n\n");
    }
    let name = if instance { "hdds_has_key" } else { "has_key" };
    let _ = writeln!(
        out,
        "    static constexpr bool {name}() noexcept {{ return {keyed}; }}\n"
    );
    out
}
