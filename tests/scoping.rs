// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! IDL 4.2 name resolution (7.5) end to end: the same name declared in
//! several modules, referenced relative, qualified and absolute, must name
//! the same type in the validator and in every backend's output (the key
//! member a `KeyHash` reads is the witness), and constants must be scoped
//! the same way. When Cyclone DDS's `idlc` is installed
//! (`/opt/cyclonedds/bin/idlc`, or `HDDS_IDLC`), its key descriptor is
//! compared too.

#![allow(clippy::expect_used, clippy::panic, clippy::needless_pass_by_value)]

use std::path::PathBuf;
use std::process::Command;

use hddsgen::ast::{Definition, Struct};
use hddsgen::validate::Level;
use hddsgen::{Backend, IdlFile, IdlType, Parser};

fn parse(idl: &str) -> IdlFile {
    Parser::try_new(idl)
        .and_then(|mut p| p.parse())
        .unwrap_or_else(|e| panic!("parse: {e}\n{idl}"))
}

fn errors(idl: &str) -> Vec<String> {
    hddsgen::validate(&parse(idl))
        .into_iter()
        .filter(|d| matches!(d.level, Level::Error))
        .map(|d| d.message)
        .collect()
}

const BACKENDS: [(Backend, &str); 7] = [
    (Backend::Rust, "rust"),
    (Backend::Cpp, "cpp"),
    (Backend::C, "c"),
    (Backend::Python, "python"),
    (Backend::TypeScript, "typescript"),
    (Backend::Micro, "micro"),
    (Backend::CMicro, "c-micro"),
];

fn generate(backend: Backend, idl: &str) -> String {
    backend
        .generator()
        .generate(&parse(idl))
        .unwrap_or_else(|e| panic!("{e}"))
}

/// A relative qualified name: `B::X` from `A::C` is `A::B::X` (its first
/// segment `B` found in `A`), keyed on `b`, not the root `B::X` keyed on
/// `a`.
const RELATIVE_QUALIFIED: &str = "
module B { struct X { @key long a; long b; }; };
module A {
    module B { struct X { long a; @key long b; }; };
    module C { struct U { @key B::X k; long v; }; };
};
";

/// Sibling modules each declaring `X`, referenced unqualified from inside
/// each and from a nested module (found outwards).
const SIBLINGS: &str = "
module P { struct X { @key long p1; long p2; }; struct UseP { @key X kp; }; };
module Q {
    struct X { long q1; @key long q2; };
    struct UseQ { @key X kq; };
    module R { struct UseR { @key X kr; }; };
};
";

#[test]
fn relative_qualified_name_in_every_backend() {
    assert!(errors(RELATIVE_QUALIFIED).is_empty());
    for (backend, lang) in BACKENDS {
        let out = generate(backend, RELATIVE_QUALIFIED);
        assert!(out.contains(".k.b"), "{lang}: key k.b missing");
        assert!(!out.contains(".k.a"), "{lang}: key k.a (the root B::X)");
    }
}

#[test]
fn sibling_and_nested_unqualified_names_in_every_backend() {
    assert!(errors(SIBLINGS).is_empty());
    for (backend, lang) in BACKENDS {
        let out = generate(backend, SIBLINGS);
        for key in [".kp.p1", ".kq.q2", ".kr.q2"] {
            assert!(out.contains(key), "{lang}: {key} missing");
        }
        for wrong in [".kp.p2", ".kp.q", ".kq.q1", ".kq.p", ".kr.q1", ".kr.p"] {
            assert!(!out.contains(wrong), "{lang}: {wrong}");
        }
    }
}

fn idlc() -> Option<PathBuf> {
    let path = std::env::var_os("HDDS_IDLC")
        .map_or_else(|| PathBuf::from("/opt/cyclonedds/bin/idlc"), PathBuf::from);
    path.is_file().then_some(path)
}

/// Cyclone's key descriptors for `idl`, as `{ "k.b", ...` lines.
fn cyclone_keys(idl: &str, name: &str) -> Option<String> {
    let idlc = idlc()?;
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("scoping_idlc")
        .join(name);
    std::fs::create_dir_all(&dir).expect("dir");
    let file = dir.join(format!("{name}.idl"));
    std::fs::write(&file, idl).expect("idl");
    let lib = idlc.parent()?.parent()?.join("lib");
    let out = Command::new(&idlc)
        .arg(&file)
        .current_dir(&dir)
        .env("LD_LIBRARY_PATH", lib)
        .output()
        .expect("idlc");
    assert!(
        out.status.success(),
        "idlc refused {name}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let c = std::fs::read_to_string(dir.join(format!("{name}.c"))).expect("idlc output");
    Some(
        c.lines()
            .filter(|l| l.trim_start().starts_with("{ \""))
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

#[test]
fn cyclone_reads_the_same_keys() {
    let Some(keys) = cyclone_keys(RELATIVE_QUALIFIED, "relative") else {
        eprintln!("idlc not found: skipped");
        return;
    };
    assert!(keys.contains("{ \"k.b\""), "{keys}");
    let keys = cyclone_keys(SIBLINGS, "siblings").expect("idlc");
    for key in ["\"kp.p1\"", "\"kq.q2\"", "\"kr.q2\""] {
        assert!(keys.contains(key), "{key}: {keys}");
    }
}

/// 7.5.1: once its first segment is found, a qualified name is looked up
/// in that scope only.
#[test]
fn qualified_name_does_not_fall_back_outwards() {
    let errs = errors(
        "module Inner { struct X { long r; }; };
         module A { module Inner { struct Y { long a; }; }; struct U { Inner::X f; }; };",
    );
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(errs[0].starts_with("Unresolved type reference: Inner::X"));
}

fn struct_at<'a>(file: &'a IdlFile, path: &[&str], name: &str) -> &'a Struct {
    let mut defs = file.definitions.as_slice();
    for m in path {
        defs = defs
            .iter()
            .find_map(|d| match d {
                Definition::Module(x) if x.name == *m => Some(x.definitions.as_slice()),
                _ => None,
            })
            .expect("module");
    }
    defs.iter()
        .find_map(|d| match d {
            Definition::Struct(s) if s.name == name => Some(s),
            _ => None,
        })
        .expect("struct")
}

fn array_size(t: &IdlType) -> u32 {
    match t {
        IdlType::Array { size, .. } => *size,
        other => panic!("not an array: {other:?}"),
    }
}

/// Constants are scoped like types: `::MAX` is the root one, `MAX` the
/// innermost, and a root-level reference is not changed by a module
/// declaring its own `MAX`.
#[test]
fn constants_are_scoped() {
    let file = parse(
        "const long MAX = 4;
         module m {
             const long MAX = 8;
             const long TWICE = ::MAX * 2;
             struct S { long a[::MAX]; long b[MAX]; long c[TWICE]; long d[m::MAX]; };
         };
         struct R { long r[MAX]; };",
    );
    let sizes = |s: &Struct| {
        s.fields
            .iter()
            .map(|f| array_size(&f.field_type))
            .collect::<Vec<_>>()
    };
    assert_eq!(sizes(struct_at(&file, &["m"], "S")), [4, 8, 8, 8]);
    assert_eq!(sizes(struct_at(&file, &[], "R")), [4]);
    let err = Parser::try_new("module m { const long K = 1; }; struct S { long a[K]; };")
        .and_then(|mut p| p.parse())
        .expect_err("K is not visible at the root");
    assert!(err.to_string().contains("Unknown constant 'K'"), "{err}");
}

/// A name is resolved where it is used: `X` in `M::U`, before `M::X` is
/// declared, is the root `X` (as Cyclone reads it), pinned as `::X`;
/// after it, `M::X`.
#[test]
fn declaration_order() {
    let idl = "struct X { long outer; };
         module M {
             struct U { X x; };
             struct X { short inner; };
             struct V { X x; };
         };";
    let file = parse(idl);
    assert_eq!(
        struct_at(&file, &["M"], "U").fields[0].field_type,
        IdlType::Named("::X".into())
    );
    assert_eq!(
        struct_at(&file, &["M"], "V").fields[0].field_type,
        IdlType::Named("X".into())
    );
    assert!(errors(idl).is_empty());
    let c = generate(Backend::C, idl);
    let u = c.find("typedef struct M_U").expect("M_U");
    assert!(c[u..]
        .lines()
        .nth(1)
        .is_some_and(|l| l.contains("X x;") && !l.contains("M_X")));
}

#[test]
fn duplicate_member_names_are_errors() {
    for idl in [
        "struct S { long _a; long a; };",
        "struct S { long a, a; };",
        "struct S { long a; short A; };",
        "union U switch (long) { case 1: long x; case 2: short x; };",
    ] {
        let errs = errors(idl);
        assert!(
            errs.iter().any(|e| e.contains("collides with member")),
            "{idl}: {errs:?}"
        );
    }
    assert!(errors("struct S { long a, b; };").is_empty());
}

#[test]
fn literal_forms() {
    // `::K` inside a constant expression.
    let file = parse("const long D2 = 2; module m { const long K = ::D2 + 1; };");
    let Some(Definition::Module(m)) = file.definitions.get(1) else {
        panic!("module m");
    };
    assert!(
        matches!(&m.definitions[0], Definition::Const(c) if c.value == "3"),
        "{m:?}"
    );
    parse("const double E = 2E-2; const float F = 1e3; const double G = 1.5e+3;");
    let printed = hddsgen::idl_pretty(&parse(
        "const double D = 1.5e3; const char NL = '\\n'; const wchar W = L'x';
         const wstring WS = L\"w\"; const string S = \"a \\\"b\\\"\"; const boolean B = TRUE;",
    ));
    for text in [
        "const double D = 1500.0;",
        "const char NL = '\\n';",
        "const wchar W = L'x';",
        "const wstring WS = L\"w\";",
        "const string S = \"a \\\"b\\\"\";",
        "const boolean B = TRUE;",
    ] {
        assert!(printed.contains(text), "missing {text}\n{printed}");
    }
    let py = generate(
        Backend::Python,
        "const boolean B = TRUE; const string S = \"x y\";",
    );
    assert!(py.contains("B = True"), "{py}");
    assert!(py.contains("S = \"x y\""), "{py}");
}

/// A recursive member names its own struct, never an outer one of the
/// same name declared earlier.
#[test]
fn recursive_member_names_itself() {
    let file = parse(
        "struct Node { long v; };
         module M { struct Node { sequence<Node> kids; }; };",
    );
    assert_eq!(
        struct_at(&file, &["M"], "Node").fields[0].field_type,
        IdlType::Sequence {
            inner: Box::new(IdlType::Named("Node".into())),
            bound: None
        }
    );
}

/// Two structures of one name in modules with different `@autoid`: each
/// keeps its own module's.
#[test]
fn module_autoid_per_global_name() {
    use hddsgen::codegen::key_holder::TypeIndex;
    let file = parse(
        "@autoid(HASH) module H { struct X { long a; }; };
         module S { struct X { long a; }; };",
    );
    let idx = TypeIndex::new(&file);
    let hashed = |name: &str| idx.hashes_member_ids(idx.struct_def(name).expect(name));
    assert!(hashed("::H::X"));
    assert!(!hashed("::S::X"));
}

/// A type reaching two types of one name lists both nested `TypeObject`s.
#[test]
fn nested_type_objects_by_global_name() {
    let out = generate(
        Backend::Rust,
        &format!("{SIBLINGS} struct Both {{ P::X a; Q::X b; }};"),
    );
    let both = out.find("DDS for Both {").expect("Both");
    let nested = &out[both..];
    let nested = &nested[nested.find("fn nested_type_objects").expect("nested")..];
    let body = &nested[..nested.find('}').expect("end")];
    for call in ["P::__hdds_type_object_X()", "Q::__hdds_type_object_X()"] {
        assert!(body.contains(call), "{call} missing: {body}");
    }
}

/// The Rust `TypeObject` of two structures of one name in modules with
/// different `@autoid`: hashed member ids (flag `IS_AUTOID_HASH`, 0x0010)
/// for the one in the `@autoid(HASH)` module only.
#[test]
fn rust_type_objects_keep_their_module_autoid() {
    let out = generate(
        Backend::Rust,
        "@autoid(HASH) module H { struct X { long a; }; };
         module S { struct X { long a; }; };",
    );
    let flags = |module: &str| {
        let m = out.find(&format!("pub mod {module} {{")).expect(module);
        let tail = &out[m..];
        let at = tail
            .find("struct_flags: ::hdds::xtypes::StructTypeFlag(")
            .expect("flags");
        let v = &tail[at + "struct_flags: ::hdds::xtypes::StructTypeFlag(".len()..];
        u16::from_str_radix(v[2..v.find(')').expect(")")].trim(), 16).expect("hex")
    };
    assert_ne!(flags("H") & 0x0010, 0);
    assert_eq!(flags("S") & 0x0010, 0);
}

/// A typedef's name is not in scope in its own type: `typedef T T;` inside
/// `A` names the outer `T` (Cyclone reads it so), and a typedef naming
/// itself is an error, never a stack overflow.
#[test]
fn typedef_does_not_see_its_own_name() {
    let idl = "typedef long T;
         module A { typedef T T; module B { struct S { T x; }; }; };";
    let file = parse(idl);
    let Some(Definition::Module(a)) = file.definitions.get(1) else {
        panic!("module A");
    };
    let Some(Definition::Typedef(t)) = a.definitions.first() else {
        panic!("typedef A::T");
    };
    assert_eq!(t.base_type, IdlType::Named("::T".into()));
    assert!(errors(idl).is_empty());
    let c = generate(Backend::C, idl);
    assert!(c.contains("typedef T A_T"), "A::T is the outer T");
    assert!(!c.contains("typedef A_T A_T"), "self typedef");

    for cyclic in [
        "typedef X X; struct S { X x; };",
        "typedef Y X; typedef X Y; struct S { X x; };",
        "module M { typedef M::Z Z; };",
    ] {
        let errs = errors(cyclic);
        assert!(!errs.is_empty(), "{cyclic}: accepted");
    }
    let errs = errors("typedef Y X; typedef X Y; struct S { X x; };");
    assert!(
        errs.iter().any(|e| e.contains("alias chain names itself")),
        "{errs:?}"
    );
}

/// Names Cyclone refuses: a member spelled like its type, case-insensitive
/// collisions in one scope (enumerators included, declared in the scope
/// enclosing their enum), bitfields; a reopened module and a forward
/// declaration are not collisions. A member differing from its type only by
/// case is a warning instead (`member_case_collision.rs`).
#[test]
fn scope_collisions() {
    for idl in [
        "struct B { long B; };",
        "union U switch (long) { case 1: long U; };",
        "module M { enum E { RED }; const long red = 1; };",
        "module M { enum E1 { A }; enum E2 { A }; };",
        "typedef long T; typedef short t;",
        "module m { struct X { long a; }; }; module M { struct Y { long b; }; };",
        "bitset BS { bitfield<2> f, F; };",
    ] {
        let errs = errors(idl);
        assert!(
            errs.iter()
                .any(|e| e.contains("collides") || e.contains("named like")),
            "{idl}: {errs:?}"
        );
    }
    assert!(errors(
        "module M { struct X; }; module M { struct X { long a; }; struct Y { X x; }; };"
    )
    .is_empty());
}
