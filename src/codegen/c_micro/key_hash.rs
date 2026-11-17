// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Instance `KeyHash` of generated C-micro types (RTPS 2.5 9.6.4.8), through
//! the C runtime the C backend emits (`hdds_key_holder`, `hdds_kh_*`; no
//! allocation, no libc beyond `<string.h>`).

use std::cell::RefCell;
use std::fmt::Write as _;

use super::super::c::key_hash::{runtime, CFlavor, Emit};
use super::super::key_holder::TypeIndex;
use super::super::keywords::c_ident;
use crate::ast::{IdlFile, Struct};
use crate::types::IdlType;

thread_local! {
    /// Type index of the file being generated.
    // @audit-ok: thread_local! RefCell is inherently thread-safe (one instance per thread)
    static INDEX: RefCell<TypeIndex> = RefCell::new(TypeIndex::default());
}

/// Index `ast` for the structs about to be generated; return the runtime
/// to emit when some struct has a `KeyHolder` the model covers.
pub(super) fn begin_file(ast: &IdlFile) -> Option<String> {
    fn walk(defs: &[crate::ast::Definition], idx: &TypeIndex) -> bool {
        defs.iter().any(|d| match d {
            crate::ast::Definition::Struct(s) => matches!(idx.key_holder(s), Ok(Some(_))),
            crate::ast::Definition::Module(m) => walk(&m.definitions, idx),
            _ => false,
        })
    }
    let index = TypeIndex::new(ast);
    let needed = walk(&ast.definitions, &index);
    INDEX.with(|cell| *cell.borrow_mut() = index);
    needed.then(|| format!("{}\n", runtime()))
}

/// This backend's types: base members in the `base` member, strings in
/// `char[N+1]` (unbounded: the configured default size) read up to their
/// first NUL, wide strings in `uint16_t[]` (UTF-16), sequences in
/// `{ data[N]; count; }`, a union's discriminator in `_d` and its member in
/// `_u`, enum labels spelled `ENUM_LABEL`.
struct CMicroTypes;

impl CFlavor for CMicroTypes {
    fn member(&self, parent: &str, base_depth: usize, name: &str) -> Option<String> {
        let mut place = parent.to_string();
        for _ in 0..base_depth {
            place.push_str(".base");
        }
        Some(format!("{place}.{}", c_ident(name)))
    }

    fn string(&self, place: &str, bounded: bool, wide: bool) -> Option<String> {
        match (bounded, wide) {
            (_, false) => Some(format!(
                "hdds_kh_string_n(w, {place}, hdds_kh_strnlen({place}, sizeof({place})));"
            )),
            (_, true) => Some(format!(
                "hdds_kh_u16string_n(w, {place}, hdds_kh_u16nlen({place}, sizeof({place}) / sizeof(({place})[0])));"
            )),
        }
    }

    fn sequence(&self, place: &str, _bounded: bool, i: &str) -> (String, String) {
        (format!("{place}.count"), format!("{place}.data[{i}]"))
    }

    fn union_disc(&self, place: &str) -> String {
        format!("{place}._d")
    }

    fn union_member(&self, place: &str, _union_name: &str, name: &str) -> String {
        format!("{place}._u.{}", c_ident(name))
    }

    fn label(&self, disc: &IdlType, label: &str) -> String {
        if let IdlType::Named(enum_name) = disc {
            format!("{}_{}", enum_name.to_uppercase(), label.to_uppercase())
        } else {
            label.to_string()
        }
    }
}

/// `<prefix>_compute_key()` of the C type `cname`, whose `KeyHolder` is
/// that of `key_source`, the struct as the IDL declares it.
pub(super) fn compute_key_function(key_source: &Struct, func_prefix: &str, cname: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "/* Instance KeyHash of {name} (RTPS 2.5 9.6.4.8): the KeyHolder serialized\n \
         * as PLAIN_CDR2 big endian, zero-padded to 16 bytes when its maximum size\n \
         * is at most 16, its MD5 otherwise. Returns 1 (keyed), 0 (no key: all\n \
         * zeros), -1 (NULL argument), -2 (a key hddsgen's KeyHash model does not\n \
         * cover: all zeros). */",
        name = cname
    );
    let _ = writeln!(
        out,
        "static inline int {func_prefix}_compute_key(const {cname}* value, uint8_t key_hash[16]) {{"
    );
    let holder = INDEX.with(|cell| cell.borrow().key_holder(key_source));
    let emitted: Result<(String, String, bool), Option<String>> = match holder {
        Ok(Some(kh)) => {
            let flavor = CMicroTypes;
            let mut e = Emit::new(&flavor);
            e.structure(1, &kh, "(*value)")
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
            out.push_str("    hdds_key_holder kh;\n    hdds_key_holder* w = &kh;\n");
            if !decls.is_empty() {
                let _ = writeln!(out, "    {decls}");
            }
            out.push_str(
                "    if (value == (void*)0 || key_hash == (void*)0) { return -1; }\n    hdds_kh_init(w);\n",
            );
            out.push_str(&body);
            let _ = writeln!(
                out,
                "    hdds_kh_finish(w, {}, key_hash);",
                i32::from(padded)
            );
            out.push_str("    return 1;\n");
        }
        Err(reason) => {
            out.push_str("    uint32_t i;\n");
            out.push_str("    if (value == (void*)0 || key_hash == (void*)0) { return -1; }\n");
            out.push_str("    for (i = 0; i < 16; ++i) { key_hash[i] = 0; }\n");
            match reason {
                Some(reason) => {
                    let reason = reason.replace(['\n', '\r'], " ").replace("*/", "* /");
                    let _ = writeln!(
                        out,
                        "    /* No KeyHolder generated: the key of this type is not covered by\n     \
                         * hddsgen's KeyHash model ({reason}). */\n    return -2;"
                    );
                }
                None => out.push_str("    return 0;\n"),
            }
        }
    }
    out.push_str("}\n\n");
    out
}
