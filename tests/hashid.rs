// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! `@hashid` (DDS-XTypes 1.3 7.3.1.2.1.2) is a standard member annotation:
//! accepted with and without its string argument, on struct and union
//! members, alongside `@key` and `@optional`; refused with `@id` (two ids
//! for one member) or a non-string argument. The member ids it gives are
//! the name hashes the data path already uses.

#![allow(clippy::expect_used, clippy::panic)]

use hddsgen::{validate, Backend, Parser};

fn diags(idl: &str) -> Vec<String> {
    let ast = Parser::try_new(idl)
        .and_then(|mut p| p.parse())
        .unwrap_or_else(|e| panic!("{e}"));
    validate(&ast).into_iter().map(|d| d.message).collect()
}

const ACCEPTED: &str = r#"
struct S {
    @hashid long a;
    @hashid("other") long b;
    @key @hashid long c;
    @hashid @key long d;
    @optional @hashid("opt") long e;
    @hashid(value = "named") long g;
};
@mutable struct M { @hashid long a; @hashid("x") @key long b; };
union U switch (long) { case 1: @hashid long a; case 2: @hashid("y") short b; };
"#;

#[test]
fn hashid_is_accepted_everywhere_it_applies() {
    let d = diags(ACCEPTED);
    assert!(d.is_empty(), "{d:?}");
    for backend in [Backend::Rust, Backend::Cpp, Backend::C, Backend::Python] {
        let ast = Parser::try_new(ACCEPTED)
            .and_then(|mut p| p.parse())
            .expect("parse");
        backend.generator().generate(&ast).expect("generate");
    }
}

#[test]
fn hashid_misuse_is_an_error() {
    let d = diags("struct S { @id(3) @hashid long a; };");
    assert!(d.iter().any(|m| m.contains("@id and @hashid")), "{d:?}");
    let d = diags("struct S { @hashid(5) long a; };");
    assert!(d.iter().any(|m| m.contains("@hashid takes")), "{d:?}");
    let d = diags(r#"struct S { @hashid("a", "b") long a; };"#);
    assert!(d.iter().any(|m| m.contains("@hashid takes")), "{d:?}");
}

/// The id is the hash of the given name (or of the member name): the Rust
/// `TypeObject` member ids of `@hashid` and `@hashid("x")` differ from the
/// sequential ones and match `@autoid(HASH)` on a member of that name.
#[test]
fn hashid_sets_the_hashed_member_id() {
    let gen = |idl: &str| {
        let ast = Parser::try_new(idl)
            .and_then(|mut p| p.parse())
            .expect("parse");
        Backend::Rust.generator().generate(&ast).expect("generate")
    };
    let ids = |out: &str| -> Vec<String> {
        out.match_indices("member_id: ")
            .map(|(i, _)| {
                out[i + 11..]
                    .split([',', ' '])
                    .next()
                    .unwrap_or_default()
                    .to_string()
            })
            .collect()
    };
    let hashed = ids(&gen(
        "@mutable struct S { @hashid long a; @hashid(\"x\") long b; };",
    ));
    let auto = ids(&gen("@mutable @autoid(HASH) struct S { long a; long x; };"));
    assert_eq!(hashed.len(), 2, "{hashed:?}");
    assert_eq!(hashed, auto);
    assert_ne!(hashed, ["0", "1"]);
}
