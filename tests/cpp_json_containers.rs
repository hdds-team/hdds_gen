// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Runtime regression tests for generated C++ JSON container population.

#![allow(clippy::expect_used, clippy::too_many_lines)]

use std::fs;
use std::process::Command;

fn find_cxx() -> Option<String> {
    ["clang++", "g++"]
        .iter()
        .find(|cxx| Command::new(cxx).arg("--version").output().is_ok())
        .map(|cxx| (*cxx).to_string())
}

/// Every C++ compiler found (g++ and clang++ warn differently).
fn find_cxxs() -> Vec<String> {
    ["g++", "clang++"]
        .iter()
        .filter(|cxx| Command::new(cxx).arg("--version").output().is_ok())
        .map(|cxx| (*cxx).to_string())
        .collect()
}

/// Whether `cxx` builds and runs a program under
/// `-fsanitize=undefined`.
fn supports_ubsan(cxx: &str, dir: &std::path::Path) -> bool {
    let src = dir.join("ubsan_probe.cxx");
    let exe = dir.join("ubsan_probe");
    fs::write(&src, "int main() { return 0; }\n").is_ok()
        && Command::new(cxx)
            .args(["-fsanitize=undefined", "-o"])
            .arg(&exe)
            .arg(&src)
            .output()
            .is_ok_and(|o| o.status.success())
        && Command::new(&exe)
            .output()
            .is_ok_and(|o| o.status.success())
}

fn generate(args: &[&str]) -> bool {
    Command::new("cargo")
        .args(["run", "--quiet", "--bin", "hddsgen", "--"])
        .args(args)
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
fn map_elements_in_arrays_and_sequences_populate_distinct_entries() {
    let Some(cxx) = find_cxx() else {
        eprintln!("C++ compiler not found; skipping JSON container runtime test");
        return;
    };

    let dir = std::env::temp_dir().join(format!("hdds_gen_json_{}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp directory");
    let idl_path = dir.join("containers.idl");
    fs::write(
        &idl_path,
        "struct Containers { map<string, long> fixed_entries[2]; sequence<map<string, long>> dynamic_entries; };\n",
    )
    .expect("write container idl");

    for (mode, compat) in [("plain", false), ("compat", true)] {
        let header_path = dir.join(format!("{mode}.h"));
        let mut args = vec!["gen", "cpp"];
        if compat {
            args.push("--fastdds-compat");
        }
        args.extend([
            idl_path.to_str().expect("utf-8 idl path"),
            "-o",
            header_path.to_str().expect("utf-8 header path"),
        ]);
        assert!(generate(&args), "{mode} generation failed");

        let fixed = if compat {
            "value.fixed_entries()"
        } else {
            "value.fixed_entries"
        };
        let dynamic = if compat {
            "value.dynamic_entries()"
        } else {
            "value.dynamic_entries"
        };
        let source_path = dir.join(format!("{mode}.cxx"));
        let source = format!(
            "#include \"{mode}.h\"\nint main() {{\n  Containers value{{}};\n  const char* json = R\"({{\"fixed_entries\":[{{\"a\":1}},{{\"b\":2}}],\"dynamic_entries\":[{{\"c\":3}},{{\"d\":4}}]}})\";\n  if (!ContainersPubSubType::populate_from_json(&value, json)) return 1;\n  if ({fixed}[0].at(\"a\") != 1 || {fixed}[1].at(\"b\") != 2) return 2;\n  if ({dynamic}.size() != 2) return 3;\n  if ({dynamic}[0].at(\"c\") != 3 || {dynamic}[1].at(\"d\") != 4) return 4;\n  return 0;\n}}\n"
        );
        fs::write(&source_path, source).expect("write C++ runtime probe");
        let binary_path = dir.join(format!("{mode}-probe"));
        let status = Command::new(&cxx)
            .args(["-std=c++17", "-Wall", "-Wextra", "-pedantic", "-Werror"])
            .arg("-I")
            .arg(&dir)
            .arg(&source_path)
            .arg("-o")
            .arg(&binary_path)
            .status()
            .expect("compile C++ runtime probe");
        assert!(
            status.success(),
            "{mode} container header failed to compile"
        );
        let status = Command::new(&binary_path).status().expect("run C++ probe");
        assert!(
            status.success(),
            "{mode} container JSON runtime check failed"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

/// `@optional` members of every container kind, of `string<N>`, `wstring`
/// and a nested structure: `populate_from_json` fills the value (emplacing
/// it), `null` resets it, a `string<N>` longer than N is refused; a map
/// whose values are structures is filled; in both flavours. A MUTABLE
/// structure with an optional structure member is also encoded and decoded
/// (its decoder resets the optional members first). Built with every
/// compiler found, at `-O1` under `-fsanitize=undefined` when supported,
/// warnings as errors (g++ reported `-Wmaybe-uninitialized` on an empty
/// `std::optional` assigned to such a member).
#[test]
fn optional_members_populate_from_json() {
    use hddsgen::codegen::cpp::CppGenerator;
    use hddsgen::codegen::CodeGenerator;

    let cxxs = find_cxxs();
    if cxxs.is_empty() {
        eprintln!("C++ compiler not found; skipping optional JSON runtime test");
        return;
    }
    let idl = "struct Inner { long x; string n; };
struct Opts {
    @optional sequence<long> sq;
    @optional long arr[3];
    @optional map<string, long> mp;
    @optional string<5> bs;
    @optional wstring ws;
    @optional wstring<4> bws;
    @optional Inner inn;
    @optional long l;
    @optional string s;
    long keep;
    map<long, Inner> mi;
};
@mutable struct MOpts { @optional Inner inn; @optional string s; long keep; };
";
    let dir = std::env::temp_dir().join(format!("hdds_gen_json_opt_{}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp directory");
    for compat in [false, true] {
        let mode = if compat { "compat" } else { "plain" };
        let generator = if compat {
            CppGenerator::with_fastdds_compat()
        } else {
            CppGenerator::new()
        };
        let mut parser = hddsgen::Parser::try_new(idl).expect("lexer");
        let ast = parser.parse().expect("parse");
        let header = generator.generate(&ast).expect("codegen");
        fs::write(dir.join(format!("opt_{mode}.h")), header).expect("write header");
        let m = |name: &str| {
            if compat {
                format!("v.{name}()")
            } else {
                format!("v.{name}")
            }
        };
        let (sq, arr, mp, bs, ws, bws, inn, l, s, keep) = (
            m("sq"),
            m("arr"),
            m("mp"),
            m("bs"),
            m("ws"),
            m("bws"),
            m("inn"),
            m("l"),
            m("s"),
            m("keep"),
        );
        let inner_x = if compat { "inn_->x()" } else { "inn_->x" };
        let inner_n = if compat { "inn_->n()" } else { "inn_->n" };
        let (mi, mi_x) = if compat {
            ("v.mi()", "v.mi().at(3).x()")
        } else {
            ("v.mi", "v.mi.at(3).x")
        };
        let (b_inn, b_x, b_n, b_s, b_keep) = if compat {
            (
                "b.inn()",
                "b.inn()->x()",
                "b.inn()->n()",
                "b.s()",
                "b.keep()",
            )
        } else {
            ("b.inn", "b.inn->x", "b.inn->n", "b.s", "b.keep")
        };
        let source = format!(
            r#"#include "opt_{mode}.h"
int main() {{
    Opts v{{}};
    const char* full = R"({{"sq": [1, 2], "arr": [4, 5, 6], "mp": {{"a": 7}}, "bs": "hello", "ws": "h\u00e9", "bws": "ab", "inn": {{"x": 9, "n": "z"}}, "l": 3, "s": "q", "keep": 1, "mi": {{"3": {{"x": 5, "n": "m"}}}}}})";
    if (!OptsPubSubType::populate_from_json(&v, full)) return 1;
    if (!{sq} || {sq}->size() != 2 || (*{sq})[1] != 2) return 2;
    if (!{arr} || (*{arr})[0] != 4 || (*{arr})[2] != 6) return 3;
    if (!{mp} || {mp}->size() != 1 || {mp}->at("a") != 7) return 4;
    if (!{bs} || std::string({bs}->data(), 5) != "hello") return 5;
    if (!{ws} || *{ws} != std::wstring(L"h\u00e9")) return 6;
    if (!{bws} || (*{bws})[0] != L'a' || (*{bws})[1] != L'b' || (*{bws})[2] != 0) return 7;
    const auto& inn_ = {inn};
    if (!inn_ || {inner_x} != 9 || {inner_n} != "z") return 8;
    if ({l} != 3 || {s} != std::string("q") || {keep} != 1) return 9;
    if ({mi}.size() != 1 || {mi_x} != 5) return 13;
    const char* nulls = R"({{"sq": null, "arr": null, "mp": null, "bs": null, "ws": null, "bws": null, "inn": null, "l": null, "s": null}})";
    if (!OptsPubSubType::populate_from_json(&v, nulls)) return 10;
    if ({sq} || {arr} || {mp} || {bs} || {ws} || {bws} || {inn} || {l} || {s}) return 11;
    if (OptsPubSubType::populate_from_json(&v, R"({{"bs": "toolong"}})")) return 12;
    MOpts a{{}};
    MOpts b{{}};
    std::uint8_t buf[256];
    if (!MOptsPubSubType::populate_from_json(&a, R"({{"inn": {{"x": 4, "n": "k"}}, "s": "t", "keep": 6}})")) return 14;
    const int n = a.encode_cdr2_le(buf, sizeof buf);
    if (n < 0) return 15;
    if (b.decode_cdr2_le(buf, static_cast<std::size_t>(n)) != n) return 16;
    if (!{b_inn} || {b_x} != 4 || {b_n} != "k" || !{b_s} || *{b_s} != "t" || {b_keep} != 6) return 17;
    return 0;
}}
"#
        );
        let source_path = dir.join(format!("opt_{mode}.cxx"));
        fs::write(&source_path, source).expect("write C++ probe");
        for cxx in &cxxs {
            let binary_path = dir.join(format!("opt_{mode}-{cxx}-probe"));
            let mut cmd = Command::new(cxx);
            cmd.args(["-std=c++17", "-Wall", "-Wextra", "-pedantic", "-Werror"]);
            // Not `-fno-sanitize-recover`: its instrumentation hides the
            // g++ warning; a report halts the run through UBSAN_OPTIONS.
            if supports_ubsan(cxx, &dir) {
                cmd.args(["-O1", "-fsanitize=undefined"]);
            } else {
                eprintln!("{cxx}: no -fsanitize=undefined; built without it");
            }
            let out = cmd
                .arg("-I")
                .arg(&dir)
                .arg(&source_path)
                .arg("-o")
                .arg(&binary_path)
                .output()
                .expect("compile C++ probe");
            assert!(
                out.status.success(),
                "{mode} ({cxx}): optional JSON header failed to compile\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let status = Command::new(&binary_path)
                .env("UBSAN_OPTIONS", "halt_on_error=1:print_stacktrace=1")
                .status()
                .expect("run C++ probe");
            assert!(
                status.success(),
                "{mode} ({cxx}): optional JSON runtime check failed: {status}"
            );
        }
    }
    let _ = fs::remove_dir_all(&dir);
}
