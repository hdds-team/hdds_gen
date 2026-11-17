// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Parser / printer holes: IDL 4.2 the parser refused, or the printer wrote
//! back as different or invalid IDL (multiple declarators, scoped and
//! absolute names, escaped identifiers, annotation literals, `@topic`
//! parameters, `@verbatim`), and the code generated for those constructs.
//!
//! The C and C++ compile checks run when a compiler is found; the Rust one
//! needs an HDDS checkout:
//! `HDDSGEN_HDDS_DIR=<hdds> cargo test --test parser_printer_roundtrip -- --ignored`.

#![allow(clippy::expect_used, clippy::panic, clippy::needless_pass_by_value)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use hddsgen::ast::{Definition, Struct, Union, UnionLabel};
use hddsgen::codegen::data_model::union_label_value;
use hddsgen::codegen::key_holder::TypeIndex;
use hddsgen::types::{AnnotationParam, AnnotationParamKind};
use hddsgen::validate::Level;
use hddsgen::{idl_pretty, Annotation, Backend, IdlFile, IdlType, Parser, PrimitiveType};

const HOLES: &str = include_str!("fixtures/roundtrip/holes.idl");
const ESCAPED_TYPES: &str = include_str!("fixtures/roundtrip/escaped_type_names.idl");

fn parse(idl: &str) -> IdlFile {
    Parser::try_new(idl)
        .and_then(|mut p| p.parse())
        .unwrap_or_else(|e| panic!("parse: {e}\n{idl}"))
}

/// The definitions of the module at `path` (the root for an empty path).
fn defs<'a>(file: &'a IdlFile, path: &[&str]) -> &'a [Definition] {
    let mut defs = file.definitions.as_slice();
    for m in path {
        defs = defs
            .iter()
            .find_map(|d| match d {
                Definition::Module(x) if x.name == *m => Some(x.definitions.as_slice()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("module {m}"));
    }
    defs
}

fn struct_at<'a>(file: &'a IdlFile, path: &[&str], name: &str) -> &'a Struct {
    defs(file, path)
        .iter()
        .find_map(|d| match d {
            Definition::Struct(s) if s.name == name => Some(s),
            _ => None,
        })
        .unwrap_or_else(|| panic!("struct {name}"))
}

fn union_at<'a>(file: &'a IdlFile, path: &[&str], name: &str) -> &'a Union {
    defs(file, path)
        .iter()
        .find_map(|d| match d {
            Definition::Union(u) if u.name == name => Some(u),
            _ => None,
        })
        .unwrap_or_else(|| panic!("union {name}"))
}

fn field_names(s: &Struct) -> Vec<&str> {
    s.fields.iter().map(|f| f.name.as_str()).collect()
}

const fn long() -> IdlType {
    IdlType::Primitive(PrimitiveType::Long)
}

fn array(inner: IdlType, size: u32) -> IdlType {
    IdlType::Array {
        inner: Box::new(inner),
        size,
    }
}

const SENSOR: &[&str] = &["Platform", "Sensor"];

/// parse -> print -> parse gives the same AST, and printing it again the
/// same text.
#[test]
fn corpus_round_trips() {
    for src in [HOLES, ESCAPED_TYPES] {
        let first = parse(src);
        let printed = idl_pretty(&first);
        let second = parse(&printed);
        assert_eq!(first, second, "AST changed through the printer:\n{printed}");
        assert_eq!(printed, idl_pretty(&second), "printer not idempotent");
    }
}

/// `float a, b, c;` (IDL 4.2 7.4.1.4.4.2): a field per declarator, each
/// with the member's type and annotations; an array declarator wraps the
/// type for its own name only. Typedefs and bitfields take lists too.
#[test]
fn multiple_declarators() {
    let file = parse(HOLES);

    let root = struct_at(&file, &[], "Position");
    assert_eq!(field_names(root), ["latitude", "longitude", "altitude"]);

    let track = struct_at(&file, SENSOR, "Track");
    assert_eq!(
        field_names(track)[..9],
        [
            "track_id",
            "source_id",
            "bearing",
            "range",
            "elevation",
            "history",
            "count",
            "matrix",
            "trace"
        ]
    );
    for key in &track.fields[..2] {
        assert!(key.is_key(), "{} lost @key", key.name);
    }
    assert!(!track.fields[2].is_key());
    assert_eq!(track.fields[5].field_type, array(long(), 4));
    assert_eq!(track.fields[6].field_type, long());
    assert_eq!(track.fields[7].field_type, array(array(long(), 3), 2));
    assert_eq!(track.fields[8].field_type, long());

    let typedefs: Vec<(&str, &IdlType)> = defs(&file, &["Common"])
        .iter()
        .filter_map(|d| match d {
            Definition::Typedef(t) => Some((t.name.as_str(), &t.base_type)),
            _ => None,
        })
        .collect();
    let label = IdlType::BoundedString {
        wide: false,
        bound: 32,
    };
    assert_eq!(
        typedefs,
        [
            ("Label", &label),
            ("LabelPair", &array(label.clone(), 2)),
            ("Grid", &array(array(long(), 3), 2)),
            ("Cell", &long()),
        ]
    );

    let bits: Vec<(String, u32)> = defs(&file, SENSOR)
        .iter()
        .find_map(|d| match d {
            Definition::Bitset(b) => Some(b.fields.iter().map(|f| (f.name.clone(), f.width))),
            _ => None,
        })
        .expect("bitset")
        .collect();
    assert_eq!(
        bits,
        [("low".into(), 3), ("mid".into(), 3), ("high".into(), 2)]
    );

    // A union branch has one declarator (7.4.1.4.4.4 `element_spec`).
    assert!(
        Parser::try_new("union U switch (long) { case 1: long a, b; };")
            .and_then(|mut p| p.parse())
            .is_err()
    );
}

/// `@topic` and `@topic(name=..., platform=...)` both come back, the
/// latter with its parameters; `@verbatim` is printed at all.
#[test]
fn topic_and_verbatim_survive() {
    let file = parse(HOLES);
    assert!(struct_at(&file, SENSOR, "Track")
        .annotations
        .contains(&Annotation::Topic {
            name: None,
            platform: None
        }));
    assert!(struct_at(&file, SENSOR, "Status")
        .annotations
        .contains(&Annotation::Topic {
            name: Some("SensorStatus".into()),
            platform: Some("vehicle \"A\"\\1".into()),
        }));
    let printed = idl_pretty(&file);
    for line in [
        "        @topic\n",
        r#"@topic(name="SensorStatus", platform="vehicle \"A\"\\1")"#,
        r#"@verbatim(language="c", placement=BEGIN_FILE, text="/* generated */")"#,
    ] {
        assert!(printed.contains(line), "missing {line}\n{printed}");
    }
    let positional = parse(r#"@topic("Named") struct S { long a; };"#);
    assert!(struct_at(&positional, &[], "S")
        .annotations
        .contains(&Annotation::Topic {
            name: Some("Named".into()),
            platform: None
        }));
}

/// `case X::K1:` with an enum of another module, and an absolute
/// `case ::X::K2:`, kept as written and resolved to their values.
#[test]
fn scoped_case_labels() {
    let file = parse(HOLES);
    let u = union_at(&file, SENSOR, "Reading");
    let labels: Vec<&str> = u
        .cases
        .iter()
        .flat_map(|c| &c.labels)
        .map(|l| match l {
            UnionLabel::Value(v) => v.as_str(),
            UnionLabel::Default => "default",
        })
        .collect();
    assert_eq!(
        labels,
        [
            "Common::MODE_OFF",
            "::Common::MODE_STANDBY",
            "Common::MODE_ACTIVE"
        ]
    );
    let idx = TypeIndex::new(&file);
    let values: Vec<Option<i64>> = labels
        .iter()
        .map(|l| union_label_value(&idx, &u.discriminator, l))
        .collect();
    assert_eq!(values, [Some(0), Some(1), Some(2)]);
}

/// Custom annotation parameters keep their literal kind, so a string is
/// quoted and escaped again and a name, a number, a boolean are not.
#[test]
fn custom_annotation_literal_kinds() {
    let file = parse(HOLES);
    let s = struct_at(&file, SENSOR, "Annotated");
    let note = s
        .annotations
        .iter()
        .find_map(|a| match a {
            Annotation::Custom { name, params } if name == "note" => Some(params.clone()),
            _ => None,
        })
        .expect("@note");
    let p = |n: &str, v: &str, k| AnnotationParam::new(n, v, k);
    assert_eq!(
        note,
        [
            p("text", "x y, z", AnnotationParamKind::String),
            p("level", "3", AnnotationParamKind::Number),
            p("scale", "-1.5", AnnotationParamKind::Number),
            p("enabled", "true", AnnotationParamKind::Bool),
            p("kind", "Common::MODE_OFF", AnnotationParamKind::Identifier),
        ]
    );
    let printed = idl_pretty(&file);
    for text in [
        r#"@note(text="x y, z", level=3, scale=-1.5, enabled=TRUE, kind=Common::MODE_OFF)"#,
        r#"@label("a \"quoted\" \\ word")"#,
        r#"@unit("m/s")"#,
        "@range(min=0, max=100)",
        "@default(-2)",
        "@min(-1.5) @max(2.5)",
        r#"@default("hello, world")"#,
        r#"case Common::MODE_OFF: @unit("s") long off_code;"#,
        "@default('q')",
    ] {
        assert!(printed.contains(text), "missing {text}\n{printed}");
    }
    // A string that looks like a name or a number stays a string.
    let file = parse(r#"struct S { @tag(a="RED", b="12", c="") long x; };"#);
    let printed = idl_pretty(&file);
    assert!(
        printed.contains(r#"@tag(a="RED", b="12", c="")"#),
        "{printed}"
    );
}

/// `_map` is the identifier `map` (IDL 4.2 7.2.3.2); the printer escapes a
/// name again when it is a keyword or begins with an underscore.
#[test]
fn escaped_identifiers() {
    let file = parse(HOLES);
    let m = struct_at(&file, &["module"], "map");
    assert_eq!(
        field_names(m),
        ["struct", "switch", "union", "case", "default"]
    );
    assert_eq!(m.fields[2].field_type, IdlType::Named("sequence".into()));
    let printed = idl_pretty(&file);
    for text in [
        "@default(_octet)",
        "module _module {",
        "struct _map {",
        "long      _struct;",
        "_sequence _union;",
        "_default[2];",
        "_octet,",
    ] {
        assert!(printed.contains(text), "missing {text}\n{printed}");
    }

    let file = parse("struct X { long _y; long Module; };");
    let s = struct_at(&file, &[], "X");
    assert_eq!(field_names(s), ["y", "Module"]);
    let printed = idl_pretty(&file);
    // Keyword collisions are case-insensitive (7.2.3.1).
    assert!(printed.contains("long _Module;"), "{printed}");

    // After the escaping underscore comes an identifier, which begins with
    // a letter (7.2.3.2).
    for bad in [
        "struct _1x { long a; };",
        "struct __x { long a; };",
        "struct _ { long a; };",
    ] {
        assert!(Parser::try_new(bad).is_err(), "{bad} accepted");
    }
}

/// `@verbatim(placement=_struct)`: an escaped name stays a name.
#[test]
fn escaped_annotation_names_stay_names() {
    let src = r#"@verbatim(language="c", placement=_struct, text="x") struct V { long w; };"#;
    let printed = idl_pretty(&parse(src));
    assert!(printed.contains("placement=_struct"), "{printed}");
    assert_eq!(parse(&printed), parse(src));
}

/// A reopened module stays where it is reopened; the corpus's last
/// `module Common` uses `Scope`, declared after the first `Common`.
#[test]
fn reopened_module_keeps_its_place() {
    let file = parse(HOLES);
    let modules: Vec<&str> = file
        .definitions
        .iter()
        .filter_map(|d| match d {
            Definition::Module(m) => Some(m.name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(modules.first(), Some(&"Common"));
    assert_eq!(modules.last(), Some(&"Common"));
    let printed = idl_pretty(&file);
    assert!(
        printed.rfind("module Common {") > printed.find("module Scope {"),
        "{printed}"
    );
}

/// `::Base` names the root type even inside a module declaring its own
/// `Base`; the AST keeps the `::`, the printer writes it back, the
/// validator resolves it from the root only.
#[test]
fn absolute_names() {
    let file = parse(HOLES);
    let uses = struct_at(&file, &["Scope"], "Uses");
    let types: Vec<&IdlType> = uses.fields.iter().map(|f| &f.field_type).collect();
    assert_eq!(
        types,
        [
            &IdlType::Named("::Base".into()),
            &IdlType::Named("Base".into()),
            &IdlType::Named("Scope::Base".into()),
        ]
    );
    assert_eq!(
        struct_at(&file, &["Scope"], "Derived")
            .base_struct
            .as_deref(),
        Some("::Base")
    );
    let where_ = &struct_at(&file, SENSOR, "Track").fields[10];
    assert_eq!(where_.field_type, IdlType::Named("::Position".into()));

    let printed = idl_pretty(&file);
    for text in [
        "::Base      absolute;",
        "struct Derived : ::Base {",
        "::Position",
    ] {
        assert!(printed.contains(text), "missing {text}\n{printed}");
    }
    let errors: Vec<String> = hddsgen::validate(&file)
        .into_iter()
        .filter(|d| matches!(d.level, Level::Error))
        .map(|d| d.message)
        .collect();
    assert!(errors.is_empty(), "{errors:?}");

    // The index resolves the absolute name to the root declaration.
    let idx = TypeIndex::new(&file);
    let base = |name: &str| idx.struct_def(name).map(|s| field_names(s)[0].to_string());
    assert_eq!(base("::Base").as_deref(), Some("b"));
    assert_eq!(base("Scope::Base").as_deref(), Some("s"));
}

/// The deprecated constructor no longer turns a lexer error into an empty
/// AST: `parse` returns it.
#[test]
#[allow(deprecated)]
fn parser_new_reports_lexer_errors() {
    let bad = "struct S { string s; }; const string C = \"unterminated";
    assert!(Parser::try_new(bad).is_err());
    let err = Parser::new(bad).parse().expect_err("lexer error hidden");
    assert!(err.to_string().contains("Unterminated"), "{err}");
    let ok = Parser::new("struct S { long a; };").parse().expect("valid");
    assert_eq!(ok.definitions.len(), 1);
}

/// The preprocessor the binary uses is the library's.
#[test]
fn preprocessor_is_public() {
    let out = hddsgen::preprocessor::preprocess_str(
        "#pragma once\n#define N 4\n#ifdef N\nstruct S { long a[N]; };\n#endif\n",
        None,
        &[],
    )
    .expect("preprocess");
    assert!(!out.content.contains('#'), "{}", out.content);
    let file = parse(&out.content);
    assert_eq!(
        struct_at(&file, &[], "S").fields[0].field_type,
        array(long(), 4)
    );
}

// ---------------------------------------------------------------------------
// Generated code for the corpus
// ---------------------------------------------------------------------------

fn generate(backend: Backend) -> String {
    backend
        .generator()
        .generate(&parse(HOLES))
        .expect("codegen")
}

fn work_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("parser_printer_roundtrip")
        .join(name);
    fs::create_dir_all(&dir).expect("work dir");
    dir
}

fn found(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok()
}

fn compile(tool: &str, args: &[&str], dir: &Path) {
    let out = Command::new(tool)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("compiler");
    assert!(
        out.status.success(),
        "{tool} {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn generated_c_compiles() {
    let Some(cc) = ["cc", "gcc", "clang"].into_iter().find(|c| found(c)) else {
        eprintln!("no C compiler: skipped");
        return;
    };
    let dir = work_dir("c");
    fs::write(dir.join("holes.h"), generate(Backend::C)).expect("header");
    fs::write(
        dir.join("main.c"),
        "#include \"holes.h\"\n\
         int main(void) {\n\
             module_map m = {0};\n\
             Common_Later later = {0};\n\
             (void)later;\n\
             Scope_Uses u = {0};\n\
             Scope_Derived d = {0};\n\
             Limits_Sized ls = {0};\n\
             Sized rs = {0};\n\
             m.struct_ = 1; u.absolute.b = 2; u.relative.s = 3; d.base.b = 4;\n\
             if (sizeof ls.outer != 4 * sizeof(int32_t) || sizeof ls.inner != 8 * sizeof(int32_t)) return 2;\n\
             if (sizeof rs.root != 4 * sizeof(int32_t) || MAX != 4 || Limits_MAX != 8 || Limits_OUTER != 8) return 3;\n\
             if (ENABLED != 1 || NEWLINE != '\\n' || SCALE != 1500.0) return 4;\n\
             return m.struct_ + u.absolute.b + u.relative.s + d.base.b == 10 ? 0 : 1;\n\
         }\n",
    )
    .expect("main");
    compile(
        cc,
        &["-std=c11", "-Wall", "-Werror", "main.c", "-o", "main"],
        &dir,
    );
    compile(&dir.join("main").to_string_lossy(), &[], &dir);
}

#[test]
fn generated_cpp_compiles() {
    let Some(cxx) = ["c++", "g++", "clang++"].into_iter().find(|c| found(c)) else {
        eprintln!("no C++ compiler: skipped");
        return;
    };
    let dir = work_dir("cpp");
    fs::write(dir.join("holes.hpp"), generate(Backend::Cpp)).expect("header");
    fs::write(
        dir.join("main.cpp"),
        "#include \"holes.hpp\"\n\
         int main() {\n\
             module::map m; Scope::Uses u; Scope::Derived d;\n\
             m.struct_ = 1; u.absolute.b = 2; u.relative.s = 3; d.b = 4;\n\
             Limits::Sized ls; Sized rs; Choice ch; Common::Later later; (void)later;\n\
             static_assert(delete_ == 5, \"keyword constant\");\n\
             static_assert(sizeof ls.outer == 4 * sizeof(int32_t) && sizeof ls.inner == 8 * sizeof(int32_t), \"bounds\");\n\
             static_assert(sizeof rs.root == 4 * sizeof(int32_t) && MAX == 4 && Limits::MAX == 8, \"consts\");\n\
             if (Limits::Painted().color != Limits::Color::BLUE && Limits::Painted().color != Limits::Color::RED) return 2;\n\
             (void)ch;\n\
             return m.struct_ + u.absolute.b + u.relative.s + d.b == 10 ? 0 : 1;\n\
         }\n",
    )
    .expect("main");
    compile(
        cxx,
        &["-std=c++17", "-Wall", "-Werror", "main.cpp", "-o", "main"],
        &dir,
    );
    compile(&dir.join("main").to_string_lossy(), &[], &dir);
}

#[test]
fn generated_python_parses() {
    if !found("python3") {
        eprintln!("no python3: skipped");
        return;
    }
    let dir = work_dir("python");
    fs::write(dir.join("holes.py"), generate(Backend::Python)).expect("module");
    compile("python3", &["-m", "py_compile", "holes.py"], &dir);
}

#[test]
#[ignore = "needs an HDDS checkout: HDDSGEN_HDDS_DIR=<hdds> cargo test --test parser_printer_roundtrip -- --ignored"]
fn generated_rust_compiles() {
    let hdds = PathBuf::from(std::env::var_os("HDDSGEN_HDDS_DIR").unwrap_or_default());
    assert!(
        hdds.join("crates/hdds/Cargo.toml").is_file(),
        "HDDSGEN_HDDS_DIR={} must name an HDDS checkout: the Rust check did NOT run",
        hdds.display()
    );
    let dir = work_dir("rust");
    fs::create_dir_all(dir.join("src")).expect("src");
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"holes\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n\n\
             [dependencies]\nhdds = {{ path = \"{}\" }}\n[workspace]\n",
            hdds.join("crates/hdds").display()
        ),
    )
    .expect("manifest");
    fs::write(dir.join("src/generated.rs"), generate(Backend::Rust)).expect("generated");
    fs::write(
        dir.join("src/main.rs"),
        "#![allow(warnings)]\n\
         mod generated;\n\
         use generated::*;\n\
         use hdds::{Cdr2Decode, Cdr2Encode};\n\
         fn main() {\n\
             let u = Scope::Uses { absolute: Base { b: 7 }, relative: Scope::Base { s: 8 }, qualified: Scope::Base { s: 9 } };\n\
             let d = Scope::Derived { base: Base { b: 5 }, d: 6 };\n\
             let m = module::map { r#struct: 1, switch: \"x\".into(), union: module::sequence::default(), case: 2, default: [3, 4] };\n\
             let mut buf = [0u8; 256];\n\
             let n = u.encode_cdr2_le(&mut buf).unwrap();\n\
             assert_eq!(Scope::Uses::decode_cdr2_le(&buf[..n]).unwrap().0, u);\n\
             let n = d.encode_cdr2_le(&mut buf).unwrap();\n\
             assert_eq!(Scope::Derived::decode_cdr2_le(&buf[..n]).unwrap().0, d);\n\
             let n = m.encode_cdr2_le(&mut buf).unwrap();\n\
             assert_eq!(module::map::decode_cdr2_le(&buf[..n]).unwrap().0, m);\n\
             assert_eq!((MAX, Limits::MAX, Limits::OUTER, SCALE, NEWLINE, ENABLED), (4, 8, 8, 1500.0, '\\n', true));\n\
             assert_eq!(QUOTED, \"a \\\"q\\\" \\\\ b\");\n\
             let ls = Limits::Sized { outer: [0; 4], inner: [0; 8], both: [0; 8] };\n\
             let rs = Sized { root: [0; 4] };\n\
             let _ = (ls, rs);\n\
             assert_eq!(Limits::Painted::builder().build().unwrap().color, Limits::Color::BLUE);\n\
             println!(\"ok\");\n\
         }\n",
    )
    .expect("main");
    let out = Command::new(env!("CARGO"))
        .args(["run", "--offline", "--quiet"])
        .current_dir(&dir)
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .output()
        .expect("cargo run");
    assert!(
        out.status.success() && String::from_utf8_lossy(&out.stdout).contains("ok"),
        "generated Rust failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
