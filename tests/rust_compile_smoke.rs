// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Compile-sanity for the Rust backend.
//!
//! The cross-language roundtrip script (`scripts/test-codegen-roundtrip.sh`)
//! covers Python/C/C++ only, because generated Rust `use`s the external `hdds`
//! crate. `@nested` types need just three items from it, so this test stubs
//! them and runs `rustc` over the emitted code — catching whole classes of
//! non-compiling output (notably E0063: an incomplete struct literal, which is
//! what un-flattened IDL inheritance produced) that substring assertions miss.

#![allow(clippy::expect_used)]

use hddsgen::{Backend, Parser};
use std::process::Command;

/// Minimal stand-in for the parts of the `hdds` crate that `@nested`
/// generated code references. Rust 2018+ uniform paths let the generated
/// `use hdds::{...}` bind to this crate-root module.
const HDDS_STUB: &str = r"
#![allow(dead_code, unused_imports, unused_variables, unused_assignments, unreachable_code, clippy::all)]
pub mod hdds {
    #[derive(Debug)]
    pub enum CdrError { BufferTooSmall, UnexpectedEof, InvalidEncoding }
    pub trait Cdr2Encode {
        fn encode_cdr2_le(&self, dst: &mut [u8]) -> Result<usize, CdrError>;
        fn max_cdr2_size(&self) -> usize;
        fn encode_cdr2_le_at(&self, dst: &mut [u8], offset: &mut usize) -> Result<(), CdrError>;
    }
    pub trait Cdr2Decode: Sized {
        fn decode_cdr2_le(src: &[u8]) -> Result<(Self, usize), CdrError>;
        fn decode_cdr2_le_at(src: &[u8], offset: &mut usize) -> Result<Self, CdrError>;
    }
}
";

/// Generate Rust for `idl` and compile it as a library. Returns `rustc`'s
/// stderr on failure, `None` on success (or if `rustc` is unavailable).
fn compile_generated(idl: &str) -> Option<String> {
    compile_generated_with(idl, "")
}

/// As [`compile_generated`], with `probe` appended after the generated code.
/// A probe that reads a field through its expected type is an oracle for
/// *which* type a name bound to — something no substring assertion can see.
fn compile_generated_with(idl: &str, probe: &str) -> Option<String> {
    let mut parser = Parser::try_new(idl).expect("lexer error");
    let ast = parser.parse().expect("parse error");
    let generated = Backend::Rust
        .generator()
        .generate(&ast)
        .expect("codegen error");

    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("probe.rs");
    std::fs::write(&src, format!("{HDDS_STUB}{generated}{probe}")).expect("write probe");

    let out = Command::new("rustc")
        .args(["--edition", "2021", "--crate-type", "lib", "--out-dir"])
        .arg(dir.path())
        .arg(&src)
        .output();
    match out {
        // No rustc in this environment: nothing to assert. Any other spawn
        // failure is a real problem and must not read as a pass.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("SKIP: no rustc on PATH");
            None
        }
        Err(e) => panic!("could not run rustc: {e}"),
        Ok(out) if out.status.success() => None,
        Ok(out) => Some(String::from_utf8_lossy(&out.stderr).into_owned()),
    }
}

#[test]
fn generated_rust_for_inherited_structs_compiles() {
    // XTypes 1.3 7.2.2.4.4: single-level, multi-level, module-qualified and
    // annotated inheritance must all emit compilable Rust.
    let idl = r"
        @nested struct Base { @key long id; string label; };
        @nested struct Derived : Base { long b; sequence<long> xs; };
        @nested struct Deeper : Derived { @optional long opt; };
        module M {
            @nested struct MBase { double d; };
            @nested struct MDerived : MBase { long n; };
        };
        @nested struct FromQualified : M::MBase { long q; };
        @nested struct Embeds { Derived inner; };
    ";
    if let Some(stderr) = compile_generated(idl) {
        panic!("generated Rust for inherited structs must compile:\n{stderr}");
    }
}

#[test]
fn generated_rust_for_cross_module_inherited_named_members_compiles() {
    // Generated modules only carry `use super::*`, so a member whose type is a
    // NAMED type declared beside the base must be requalified when it is
    // copied into a derived struct in another module — otherwise the emitted
    // `pub i: Inner` / `<Inner>::decode_...` do not resolve (E0412/E0433).
    // `module Shadow` declares its own `Inner`, where a bare copy would bind to
    // the wrong type and still compile; the `_probe_*` functions below are the
    // oracle for that (a compile leg alone cannot see a mis-binding), and
    // `cross_module_inherited_named_members_are_requalified` in
    // src/codegen/rust_backend/tests.rs pins the emitted spelling.
    let idl = r"
        module A {
            @nested struct Inner { long v; };
            enum Kind { ONE, TWO };
            module Sub { @nested struct Deep { long d; }; };
            @nested struct Base {
                Inner i;
                sequence<Inner> xs;
                Inner arr[2];
                Kind k;
                Sub::Deep deep;
            };
        };
        module B { @nested struct Derived : A::Base { long b; }; };
        module Shadow {
            @nested struct Inner { double other; };
            @nested struct AlsoDerived : A::Base { long c; };
        };
        @nested struct RootDerived : A::Base { long r; };
    ";
    // `A::Inner::v` is `long`; `Shadow::Inner` has no `v` and its only member
    // is a `double`. So these bodies type-check only if the inherited members
    // bound to A::Inner — including through the shadowing module.
    let probe = r"
        fn _probe_b(d: &B::Derived) -> i64 { i64::from(d.i.v) + i64::from(d.xs[0].v) + i64::from(d.arr[1].v) }
        fn _probe_shadow(d: &Shadow::AlsoDerived) -> i64 { i64::from(d.i.v) + i64::from(d.deep.d) }
        fn _probe_root(d: &RootDerived) -> i64 { i64::from(d.i.v) }
    ";
    if let Some(stderr) = compile_generated_with(idl, probe) {
        panic!("cross-module inherited named members must compile AND bind to the base's types:\n{stderr}");
    }
}

#[test]
fn generated_rust_for_plain_structs_compiles() {
    // Control: the same check over inheritance-free types, so a failure in
    // the test above points at inheritance rather than at the stub.
    let idl = r"
        @nested struct Plain { @key long id; string label; sequence<long> xs; };
        @nested struct Holder { Plain p; @optional double d; };
    ";
    if let Some(stderr) = compile_generated(idl) {
        panic!("generated Rust for plain structs must compile:\n{stderr}");
    }
}
