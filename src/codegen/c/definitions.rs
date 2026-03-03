// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Type definition generation for C.
//!
//! Note: `uninlined_format_args` allowed here due to extensive `format_args!` usage
//! in code generation that would require significant refactoring.
//!
//! Generates C struct, enum, union, typedef, bitset, and bitmask definitions.

#![allow(clippy::uninlined_format_args)]

use super::super::keywords::c_ident;
use super::{
    codec::{
        collect_c89_declarations, emit_decode_field, emit_encode_field, emit_max_field, label_to_c,
    },
    helpers::{c_name, push_fmt, to_upper_ascii},
    index::DefinitionIndex,
    CGenerator, CStandard,
};
use crate::ast::{
    Bitmask, Bitset, Definition, Enum, Field, Struct, Typedef, Union, UnionCase, UnionLabel,
};
use crate::types::{Annotation, IdlType};

impl CGenerator {
    pub(super) fn emit_struct(&self, s: &Struct) -> String {
        let mut out = String::new();
        push_fmt(
            &mut out,
            format_args!("{}typedef struct {} {{\n", self.indent(), s.name),
        );

        if let Some(base) = &s.base_struct {
            push_fmt(
                &mut out,
                format_args!("{}    {} base;\n", self.indent(), base),
            );
        }

        for f in &s.fields {
            let escaped = c_ident(&f.name);
            if let IdlType::Array { inner, size } = &f.field_type {
                let base = Self::type_to_c(inner);
                push_fmt(
                    &mut out,
                    format_args!("{}    {} {}[{}];\n", self.indent(), base, escaped, size),
                );
            } else {
                let cty = Self::type_to_c(&f.field_type);
                push_fmt(
                    &mut out,
                    format_args!("{}    {} {};\n", self.indent(), cty, escaped),
                );
            }
        }

        push_fmt(
            &mut out,
            format_args!("{}}} {} ;\n\n", self.indent(), s.name),
        );
        out
    }

    pub(super) fn emit_enum(&self, e: &Enum) -> String {
        let mut out = String::new();
        push_fmt(&mut out, format_args!("{}typedef enum {{\n", self.indent()));
        let ename_up = to_upper_ascii(&e.name);
        for (i, v) in e.variants.iter().enumerate() {
            let vname_up = to_upper_ascii(&v.name);
            let comma = if i + 1 == e.variants.len() { "" } else { "," };
            if let Some(val) = v.value {
                push_fmt(
                    &mut out,
                    format_args!(
                        "{}    {}_{} = {}{}\n",
                        self.indent(),
                        ename_up,
                        vname_up,
                        val,
                        comma
                    ),
                );
            } else {
                push_fmt(
                    &mut out,
                    format_args!("{}    {}_{}{}\n", self.indent(), ename_up, vname_up, comma),
                );
            }
        }
        push_fmt(
            &mut out,
            format_args!("{}}} {} ;\n\n", self.indent(), e.name),
        );
        out
    }

    pub(super) fn emit_typedef(&self, t: &Typedef) -> String {
        format!(
            "{}typedef {} {} ;\n\n",
            self.indent(),
            Self::type_to_c(&t.base_type),
            t.name
        )
    }

    pub(super) fn emit_bitset(&self, b: &Bitset) -> String {
        let mut out = String::new();
        push_fmt(
            &mut out,
            format_args!(
                "{}typedef struct {} {{ uint64_t bits; }} {} ;\n",
                self.indent(),
                b.name,
                b.name
            ),
        );
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
            let escaped = c_ident(&f.name);
            push_fmt(
                &mut out,
                format_args!(
                    "{}/* {}: width {} at bit {} */\n",
                    self.indent(),
                    f.name,
                    f.width,
                    start
                ),
            );
            push_fmt(
                &mut out,
                format_args!(
                    "{}static inline uint64_t {n}_get_{f}(const {n}* s) {{ return (s->bits >> {start}) & ((1ull << {w}) - 1ull); }}\n",
                    self.indent(),
                    n = b.name,
                    f = escaped,
                    start = start,
                    w = f.width
                ),
            );
            push_fmt(
                &mut out,
                format_args!(
                    "{}static inline void {n}_set_{f}({n}* s, uint64_t v) {{ uint64_t mask = ((1ull << {w}) - 1ull) << {start}; s->bits = (s->bits & ~mask) | (((v) & ((1ull << {w}) - 1ull)) << {start}); }}\n",
                    self.indent(),
                    n = b.name,
                    f = escaped,
                    w = f.width,
                    start = start
                ),
            );
        }
        out.push('\n');
        out
    }

    pub(super) fn emit_bitmask(&self, m: &Bitmask) -> String {
        let mut out = String::new();
        push_fmt(
            &mut out,
            format_args!("{}typedef uint64_t {} ;\n", self.indent(), m.name),
        );
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
                format_args!(
                    "{}#define {tn}_{fn} (1ull << {bit})\n",
                    self.indent(),
                    tn = m.name.to_uppercase(),
                    fn = flag.name.to_uppercase(),
                    bit = bit
                ),
            );
        }
        out.push('\n');
        out
    }

    pub(super) fn emit_union(&self, u: &Union) -> String {
        let mut out = String::new();
        push_fmt(
            &mut out,
            format_args!(
                "{}/* Union: {} (discriminator: {}) */\n",
                self.indent(),
                u.name,
                match &u.discriminator {
                    IdlType::Primitive(p) => format!("{:?}", p),
                    IdlType::Named(n) => n.clone(),
                    other => other.to_idl_string(),
                }
            ),
        );
        push_fmt(
            &mut out,
            format_args!("{}typedef struct {} {{\n", self.indent(), u.name),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}    {} _d; /* discriminator */\n",
                self.indent(),
                Self::type_to_c(&u.discriminator)
            ),
        );
        push_fmt(&mut out, format_args!("{}    union {{\n", self.indent()));
        for case in &u.cases {
            let escaped = c_ident(&case.field.name);
            if let IdlType::Array { inner, size } = &case.field.field_type {
                let base = Self::type_to_c(inner);
                push_fmt(
                    &mut out,
                    format_args!("{}        {} {}[{}];\n", self.indent(), base, escaped, size),
                );
            } else {
                let cty = Self::type_to_c(&case.field.field_type);
                push_fmt(
                    &mut out,
                    format_args!("{}        {} {};\n", self.indent(), cty, escaped),
                );
            }
        }
        push_fmt(&mut out, format_args!("{}    }} _u;\n", self.indent()));
        push_fmt(
            &mut out,
            format_args!("{}}} {} ;\n\n", self.indent(), u.name),
        );
        out
    }

    pub(super) fn emit_type_definitions(&self, defs: &[&Definition]) -> String {
        let mut out = String::new();
        for d in defs {
            match d {
                Definition::Struct(s) => out.push_str(&self.emit_struct(s)),
                Definition::Enum(e) => out.push_str(&self.emit_enum(e)),
                Definition::Typedef(t) => out.push_str(&self.emit_typedef(t)),
                Definition::Bitset(b) => out.push_str(&self.emit_bitset(b)),
                Definition::Bitmask(m) => out.push_str(&self.emit_bitmask(m)),
                Definition::Union(u) => out.push_str(&self.emit_union(u)),
                _ => {}
            }
        }
        out
    }

    pub(super) fn emit_struct_helpers_section(
        defs: &[&Definition],
        idx: &DefinitionIndex,
        c_std: CStandard,
    ) -> String {
        let mut out = String::new();
        let is_c89 = matches!(c_std, CStandard::C89);

        for def in defs {
            if let Definition::Struct(sdef) = def {
                let fname = c_name(&sdef.name);

                // Collect C89 declarations if needed
                let c89_decls = if is_c89 {
                    let decls = collect_c89_declarations(&sdef.fields, idx);
                    let mut decl_str = String::new();
                    for d in &decls {
                        decl_str.push_str("    ");
                        decl_str.push_str(d);
                        decl_str.push('\n');
                    }
                    decl_str
                } else {
                    String::new()
                };

                // Encode function
                push_fmt(
                    &mut out,
                    format_args!(
                        "static inline int {fname}_encode_cdr2_le(const {ty}* value, uint8_t* dst, size_t len) {{\n    size_t offset = 0;\n    int err = CDR_OK;\n{c89_decls}",
                        fname = fname,
                        ty = sdef.name,
                        c89_decls = c89_decls
                    ),
                );
                if sdef.fields.is_empty() {
                    push_fmt(
                        &mut out,
                        format_args!(
                            "    (void)value;\n    (void)dst;\n    (void)len;\n    (void)err;\n"
                        ),
                    );
                }
                for field in &sdef.fields {
                    out.push_str(&emit_encode_field(field, idx, "value", c_std));
                }
                push_fmt(&mut out, format_args!("    return (int)offset;\n}}\n\n"));

                // Decode function
                push_fmt(
                    &mut out,
                    format_args!(
                        "static inline int {fname}_decode_cdr2_le({ty}* out, const uint8_t* src, size_t len) {{\n    size_t offset = 0;\n    int err = CDR_OK;\n{c89_decls}",
                        fname = fname,
                        ty = sdef.name,
                        c89_decls = c89_decls
                    ),
                );
                if sdef.fields.is_empty() {
                    push_fmt(
                        &mut out,
                        format_args!(
                            "    (void)out;\n    (void)src;\n    (void)len;\n    (void)err;\n"
                        ),
                    );
                }
                for field in &sdef.fields {
                    out.push_str(&emit_decode_field(field, idx, "out", c_std));
                }
                push_fmt(&mut out, format_args!("    return (int)offset;\n}}\n\n"));

                // Max size function
                push_fmt(
                    &mut out,
                    format_args!(
                        "static inline size_t {fname}_max_cdr2_size(const {ty}* value) {{\n    size_t offset = 0;\n",
                        fname = fname,
                        ty = sdef.name
                    ),
                );
                for field in &sdef.fields {
                    out.push_str(&emit_max_field(field, idx, "value"));
                }
                push_fmt(&mut out, format_args!("    return offset;\n}}\n\n"));

                // Generate TypeDescriptor for this struct
                out.push_str(&super::type_descriptor::generate_type_descriptor(sdef));
            }
        }
        out
    }

    pub(super) fn emit_union_helpers_section(
        defs: &[&Definition],
        idx: &DefinitionIndex,
        c_std: CStandard,
    ) -> String {
        let mut out = String::new();
        for def in defs {
            if let Definition::Union(udef) = def {
                let fname = c_name(&udef.name);
                push_fmt(
                    &mut out,
                    format_args!(
                        "static inline int {fname}_encode_cdr2_le(const {ty}* value, uint8_t* dst, size_t len) {{\n    size_t offset = 0;\n    int err = CDR_OK;\n",
                        fname = fname,
                        ty = udef.name
                    ),
                );
                out.push_str(&emit_encode_field(
                    &Field {
                        name: "_d".into(),
                        field_type: udef.discriminator.clone(),
                        annotations: vec![],
                    },
                    idx,
                    "value",
                    c_std,
                ));
                append_union_case_chain(
                    &mut out,
                    &udef.cases,
                    &udef.discriminator,
                    "value->_d",
                    |case: &UnionCase| emit_encode_field(&case.field, idx, "(&value->_u)", c_std),
                );
                push_fmt(&mut out, format_args!("    return (int)offset;\n}}\n\n"));

                push_fmt(
                    &mut out,
                    format_args!(
                        "static inline int {fname}_decode_cdr2_le({ty}* out, const uint8_t* src, size_t len) {{\n    size_t offset = 0;\n    int err = CDR_OK;\n",
                        fname = fname,
                        ty = udef.name
                    ),
                );
                out.push_str(&emit_decode_field(
                    &Field {
                        name: "_d".into(),
                        field_type: udef.discriminator.clone(),
                        annotations: vec![],
                    },
                    idx,
                    "out",
                    c_std,
                ));
                append_union_case_chain(
                    &mut out,
                    &udef.cases,
                    &udef.discriminator,
                    "out->_d",
                    |case: &UnionCase| emit_decode_field(&case.field, idx, "(&out->_u)", c_std),
                );
                push_fmt(&mut out, format_args!("    return (int)offset;\n}}\n\n"));

                push_fmt(
                    &mut out,
                    format_args!(
                        "static inline size_t {fname}_max_cdr2_size(const {ty}* self) {{\n    size_t offset = 0;\n",
                        fname = fname,
                        ty = udef.name
                    ),
                );
                out.push_str(&emit_max_field(
                    &Field {
                        name: "_d".into(),
                        field_type: udef.discriminator.clone(),
                        annotations: vec![],
                    },
                    idx,
                    "self",
                ));
                append_union_case_chain(
                    &mut out,
                    &udef.cases,
                    &udef.discriminator,
                    "self->_d",
                    |case: &UnionCase| emit_max_field(&case.field, idx, "(&self->_u)"),
                );
                push_fmt(&mut out, format_args!("    return offset;\n}}\n\n"));
            }
        }
        out
    }
}

fn union_case_conditions(
    case: &UnionCase,
    discriminator: &IdlType,
    discr_expr: &str,
) -> (Option<String>, bool, bool) {
    let mut conds = Vec::new();
    let mut has_default = false;
    for label in &case.labels {
        match label {
            UnionLabel::Value(v) => {
                conds.push(format!("{discr_expr} == {}", label_to_c(discriminator, v)));
            }
            UnionLabel::Default => has_default = true,
        }
    }
    let cond = if conds.is_empty() {
        None
    } else {
        Some(conds.join(" || "))
    };
    (cond, has_default, !case.labels.is_empty())
}

fn append_union_case_chain<F>(
    out: &mut String,
    cases: &[UnionCase],
    discriminator: &IdlType,
    discr_expr: &str,
    mut emit_body: F,
) where
    F: FnMut(&UnionCase) -> String,
{
    let mut first = true;
    for case in cases {
        let (cond, has_default, had_labels) =
            union_case_conditions(case, discriminator, discr_expr);
        if let Some(cond_expr) = cond {
            push_fmt(
                out,
                format_args!(
                    "    {} ({cond_expr}) {{\n",
                    if first { "if" } else { "else if" }
                ),
            );
        } else if has_default {
            out.push_str(if first {
                "    if (1 /* default */) {\n"
            } else {
                "    else {\n"
            });
        } else if had_labels {
            out.push_str(if first {
                "    if (1 /* implicit case */) {\n"
            } else {
                "    else {\n"
            });
        } else {
            out.push_str(if first {
                "    if (1) {\n"
            } else {
                "    else {\n"
            });
        }
        out.push_str(&emit_body(case));
        out.push_str("    }\n");
        first = false;
    }
}
