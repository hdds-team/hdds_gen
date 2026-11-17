// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! XCDR1 decoders of the generated C types (DDS-`XTypes` 1.3 7.4.1, the
//! `CDR_LE` / `PL_CDR_LE` encapsulations, 0x0001 / 0x0003).
//!
//! Every structure and union of the C backend gets, next to its XCDR2
//! encoder and decoder, a decoder `T__xdec1(T* v, hdds_x2c_r* c, size_t o)`
//! reading the same value from XCDR1 bytes, through the same read cursor
//! and the XCDR1 reads of the runtime (`hdds_x1c_*`). `o` is the origin
//! alignment is measured from: the start of the buffer handed to the
//! public entry point, reset to the first byte of the value of every
//! parameter (7.4.1.2.2, rules 24-25). Under XCDR1:
//!
//! - an 8-byte value aligns at 8 (XCDR2 caps alignment at 4);
//! - no DHEADER precedes a collection, nor an APPENDABLE structure or
//!   union (serialized as FINAL): a member of an APPENDABLE structure
//!   past the end of the data takes its default (a writer whose type has
//!   fewer members, 7.4.1.2.3);
//! - an optional member of a FINAL or APPENDABLE structure is a parameter,
//!   empty when the member is absent;
//! - a MUTABLE structure is a parameter list (short or `PID_EXTENDED`
//!   headers, `PID_IGNORE` skipped) closed by `PID_LIST_END`: a member is
//!   decoded by id, an unknown one skipped unless it must be understood,
//!   a missing one takes its default; a MUTABLE union is the list of its
//!   discriminator (member 0) and the selected case.
//!
//! The generated code is the XCDR2 decoder's model (Rust backend:
//! `rust_backend::codec`), C89. The C-micro backend has no XCDR1 decoder.

use std::fmt::Write as _;

use super::{
    c_ident, c_name, case_chain, discriminator, member_place, sint, uint, Flavor, Gen, Names,
};
use crate::ast::{Field, Struct, Union};
use crate::codegen::data_model::{self, member_chain, union_ext, ChainMember, Ext};
use crate::codegen::key_holder::TypeIndex;
use crate::types::{IdlType, PrimitiveType};

/// Name of the XCDR1 decoder of the structure or union `name`.
#[must_use]
pub fn dec1_name(name: &str) -> String {
    format!("{}__xdec1", c_name(name))
}

/// Prototype of the XCDR1 decoder of `n`, `static <inline>`.
#[must_use]
pub fn prototype_dec1(inline: &str, n: &Names<'_>) -> String {
    format!(
        "static {inline} int {d}({t}* v, hdds_x2c_r* c, size_t o);\n",
        d = dec1_name(n.ctype),
        t = n.ctype
    )
}

fn gen1(idx: &TypeIndex) -> Gen<'_> {
    let mut g = Gen::new(idx, Flavor::C);
    g.v1 = true;
    g.origin = "o".to_string();
    g
}

fn function1(inline: &str, n: &Names<'_>, g: &Gen<'_>, locals: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "static {inline} int {}({}* v, hdds_x2c_r* c, size_t o) {{",
        dec1_name(n.ctype),
        n.ctype
    );
    out.push_str(locals);
    out.push_str(&g.counter_decls());
    out.push_str("    (void)v;\n    (void)c;\n    (void)o;\n");
    out.push_str(&g.out);
    out.push_str("    return 0;\n}\n\n");
    out
}

/// Statements of `body` with the origin of a parameter's value: a block
/// declaring it at the cursor (the first byte of the value).
fn with_param_origin(g: &mut Gen<'_>, ind: usize, body: &mut dyn FnMut(&mut Gen<'_>, usize)) {
    let o = g.fresh("o");
    g.line(ind, "{");
    g.line(ind + 1, &format!("size_t {o} = c->pos;"));
    g.line(ind + 1, &format!("(void){o};"));
    let saved = std::mem::replace(&mut g.origin, o);
    body(g, ind + 1);
    g.origin = saved;
    g.line(ind, "}");
}

/// An optional member of a FINAL or APPENDABLE structure (rule 19): its
/// parameter, the member present when the parameter has a value.
fn optional_param(g: &mut Gen<'_>, ind: usize, m: &ChainMember) {
    let place = member_place(m, "");
    let has = member_place(m, "has_");
    let einv = g.fl.einv();
    g.line(ind, "{");
    g.line(ind + 1, "int hx_last;");
    g.line(ind + 1, "uint32_t hx_id;");
    g.line(ind + 1, "int hx_mu;");
    g.line(ind + 1, "size_t hx_end;");
    g.line(
        ind + 1,
        "HDDS_X2C_TRY(hdds_x1c_rpl(c, &hx_last, &hx_id, &hx_mu, &hx_end));",
    );
    g.line(ind + 1, "(void)hx_id;");
    g.line(ind + 1, "(void)hx_mu;");
    g.line(ind + 1, &format!("if (hx_last) {{ return {einv}; }}"));
    g.line(ind + 1, &format!("{has} = hx_end > c->pos;"));
    g.line(ind + 1, &format!("if ({has}) {{"));
    with_param_origin(g, ind + 2, &mut |g, i| {
        g.dec(i, &place, &m.field.field_type);
    });
    g.line(ind + 1, "}");
    g.try_(ind + 1, "rskip(c, hx_end)");
    g.line(ind, "}");
}

/// The XCDR1 decoder of a structure.
///
/// # Errors
///
/// Returns why the structure cannot be decoded by this backend (a member
/// type it cannot hold, an unknown base).
pub fn struct_dec1(
    idx: &TypeIndex,
    inline: &str,
    s: &Struct,
    n: &Names<'_>,
) -> Result<String, String> {
    let chain = member_chain(idx, s)?;
    let ext = data_model::struct_ext(s);
    let mut g = gen1(idx);
    let mut locals = String::new();
    match ext {
        Ext::Final | Ext::Appendable => {
            // APPENDABLE: no DHEADER; a member past the end of the data was
            // not written by the (older) writer's type and takes its
            // default (7.4.1.2.3; Table 9).
            let app = ext == Ext::Appendable;
            let ind = if app { 2 } else { 1 };
            for m in &chain {
                if app {
                    g.line(1, "if (c->pos < c->size) {");
                }
                if m.field.is_optional() {
                    optional_param(&mut g, ind, m);
                } else {
                    g.dec(ind, &member_place(m, ""), &m.field.field_type);
                }
                if app {
                    g.line(1, "} else {");
                    if m.field.is_optional() {
                        g.line(2, &format!("{} = 0;", member_place(m, "has_")));
                    } else {
                        g.default_value(2, &member_place(m, ""), &m.field.field_type);
                    }
                    g.line(1, "}");
                }
            }
        }
        Ext::Mutable => {
            let _ = write!(
                locals,
                "    size_t mend;\n    uint32_t id;\n    int mu;\n    int last;\n    unsigned char seen[{}];\n",
                chain.len().max(1)
            );
            dec1_mutable(&mut g, &chain);
        }
    }
    if let Some(e) = g.error.take() {
        return Err(e);
    }
    Ok(function1(inline, n, &g, &locals))
}

/// The parameter loop of a MUTABLE structure's XCDR1 decoder (rules
/// 24-25): a member by id, an unknown one skipped unless it must be
/// understood, a non-optional member absent from the data at its default
/// (7.4.1.2.3, Table 9), an absent optional member without value.
fn dec1_mutable(g: &mut Gen<'_>, chain: &[ChainMember]) {
    let einv = g.fl.einv();
    g.line(1, "memset(seen, 0, sizeof(seen));");
    for m in chain.iter().filter(|m| m.field.is_optional()) {
        g.line(1, &format!("{} = 0;", member_place(m, "has_")));
    }
    g.line(1, "for (;;) {");
    g.line(2, "HDDS_X2C_TRY(hdds_x1c_rpl(c, &last, &id, &mu, &mend));");
    g.line(2, "if (last) { break; }");
    with_param_origin(g, 2, &mut |g, ind| {
        g.line(ind, "switch (id) {");
        for (k, m) in chain.iter().enumerate() {
            g.line(ind, &format!("case 0x{:08x}u:", m.id));
            g.dec(ind + 1, &member_place(m, ""), &m.field.field_type);
            g.line(ind + 1, &format!("seen[{k}] = 1;"));
            if m.field.is_optional() {
                g.line(ind + 1, &format!("{} = 1;", member_place(m, "has_")));
            }
            g.line(ind + 1, "break;");
        }
        g.line(ind, "default:");
        g.line(ind + 1, &format!("if (mu) {{ return {einv}; }}"));
        g.line(ind + 1, "break;");
        g.line(ind, "}");
    });
    g.try_(2, "rskip(c, mend)");
    g.line(1, "}");
    for (k, m) in chain.iter().enumerate() {
        if !m.field.is_optional() {
            g.line(1, &format!("if (!seen[{k}]) {{"));
            g.default_value(2, &member_place(m, ""), &m.field.field_type);
            g.line(1, "}");
        }
    }
}

/// Statements reading a discriminator of `width` bytes into `v->_d`.
fn read_disc(g: &mut Gen<'_>, ind: usize, width: usize, signed: bool, dctype: &str) {
    g.line(ind, "{");
    g.line(ind + 1, &format!("{} t;", uint(width)));
    g.read_uint(ind + 1, width, "t");
    if signed {
        g.line(ind + 1, &format!("v->_d = ({dctype})({})t;", sint(width)));
    } else {
        g.line(ind + 1, &format!("v->_d = ({dctype})t;"));
    }
    g.line(ind, "}");
}

/// The XCDR1 decoder of a union: FINAL and APPENDABLE alike (the
/// discriminator, then the selected member; rule 26), MUTABLE as a
/// parameter list of the discriminator (member 0) and the selected case
/// (rules 27, 24-25), checked as the XCDR2 decoder checks it.
///
/// # Errors
///
/// Returns why the union cannot be decoded by this backend.
pub fn union_dec1(
    idx: &TypeIndex,
    inline: &str,
    u: &Union,
    n: &Names<'_>,
) -> Result<String, String> {
    let (width, signed, dctype) = discriminator(idx, Flavor::C, u)?;
    let member = |m: &Field| format!("v->_u.{}", c_ident(&m.name));
    let is_void = |m: &Field| matches!(m.field_type, IdlType::Primitive(PrimitiveType::Void));
    let mut g = gen1(idx);
    let einv = g.fl.einv();
    let mut locals = String::new();
    if union_ext(u) == Ext::Mutable {
        let ids = data_model::union_case_ids(idx, u);
        locals.push_str(
            "    size_t mend;\n    uint32_t id;\n    uint32_t cid;\n    int mu;\n    int last;\n    int dseen;\n    int cseen;\n",
        );
        g.line(1, "dseen = 0;");
        g.line(1, "cseen = 0;");
        g.line(1, "cid = 0;");
        g.line(1, "for (;;) {");
        g.line(2, "HDDS_X2C_TRY(hdds_x1c_rpl(c, &last, &id, &mu, &mend));");
        g.line(2, "if (last) { break; }");
        with_param_origin(&mut g, 2, &mut |g, ind| {
            g.line(ind, "switch (id) {");
            g.line(ind, "case 0x00000000u:");
            read_disc(g, ind + 1, width, signed, &dctype);
            g.line(ind + 1, "dseen = 1;");
            g.line(ind + 1, "break;");
            for (i, case) in u.cases.iter().enumerate() {
                if is_void(&case.field) {
                    continue;
                }
                g.line(ind, &format!("case 0x{:08x}u:", ids[i]));
                g.line(ind + 1, &format!("if (cseen) {{ return {einv}; }}"));
                g.dec(ind + 1, &member(&case.field), &case.field.field_type);
                g.line(ind + 1, "cseen = 1;");
                g.line(ind + 1, "cid = id;");
                g.line(ind + 1, "break;");
            }
            g.line(ind, "default:");
            g.line(ind + 1, &format!("if (mu) {{ return {einv}; }}"));
            g.line(ind + 1, "break;");
            g.line(ind, "}");
        });
        g.try_(2, "rskip(c, mend)");
        g.line(1, "}");
        g.line(1, &format!("if (!dseen) {{ return {einv}; }}"));
        case_chain(&mut g, u, &dctype, true, &mut |g, i| match i {
            Some(i) if !is_void(&u.cases[i].field) => g.line(
                2,
                &format!(
                    "if (!cseen || cid != 0x{:08x}u) {{ return {einv}; }}",
                    ids[i]
                ),
            ),
            _ => g.line(2, &format!("if (cseen) {{ return {einv}; }}")),
        });
    } else {
        read_disc(&mut g, 1, width, signed, &dctype);
        case_chain(&mut g, u, &dctype, false, &mut |g, i| {
            if let Some(m) = i.map(|i| &u.cases[i].field) {
                if !is_void(m) {
                    g.dec(2, &member(m), &m.field_type);
                }
            }
        });
    }
    if let Some(e) = g.error.take() {
        return Err(e);
    }
    Ok(function1(inline, n, &g, &locals))
}
