// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! The generated C++ `json_unescape` decodes `\uXXXX` escapes per RFC 8259
//! section 7: a UTF-16 surrogate pair (a high surrogate immediately followed
//! by `\u` and a low surrogate) is one code point outside the Basic
//! Multilingual Plane, written as four UTF-8 bytes; a lone or mismatched
//! surrogate gives U+FFFD. Skipped, with a message, without a C++ compiler.

#![allow(clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;
use std::fs;
use std::process::Command;

use hddsgen::codegen::cpp::CppGenerator;
use hddsgen::codegen::CodeGenerator;
use hddsgen::Parser;

/// Each case: JSON text of the member (as written in the JSON document,
/// escapes included) and the UTF-8 bytes it must decode to.
const CASES: [(&str, &[u8]); 9] = [
    // U+1F600 as a pair.
    (r"\ud83d\ude00", &[0xF0, 0x9F, 0x98, 0x80]),
    // Upper-case hex digits, the pair surrounded by text.
    (r"a\uD83D\uDE00b", &[b'a', 0xF0, 0x9F, 0x98, 0x80, b'b']),
    // Lowest and highest pairs: U+10000 and U+10FFFF.
    (r"\ud800\udc00", &[0xF0, 0x90, 0x80, 0x80]),
    (r"\udbff\udfff", &[0xF4, 0x8F, 0xBF, 0xBF]),
    // Two pairs in a row.
    (
        r"\ud83d\ude00\ud83d\ude01",
        &[0xF0, 0x9F, 0x98, 0x80, 0xF0, 0x9F, 0x98, 0x81],
    ),
    // A lone high surrogate, then an ordinary character.
    (r"\ud83dX", &[0xEF, 0xBF, 0xBD, b'X']),
    // A high surrogate followed by a non-surrogate escape.
    (r"\ud83dA", &[0xEF, 0xBF, 0xBD, b'A']),
    // A high surrogate followed by another high surrogate, then a pair.
    (
        r"\ud83d\ud83d\ude00",
        &[0xEF, 0xBF, 0xBD, 0xF0, 0x9F, 0x98, 0x80],
    ),
    // A lone low surrogate.
    (r"\ude00", &[0xEF, 0xBF, 0xBD]),
];

fn find_cxx() -> Option<String> {
    ["g++", "clang++"]
        .iter()
        .find(|cxx| Command::new(cxx).arg("--version").output().is_ok())
        .map(|cxx| (*cxx).to_string())
}

fn c_bytes(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("0x{x:02X}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[test]
fn cpp_json_unescape_combines_surrogate_pairs() {
    let Some(cxx) = find_cxx() else {
        eprintln!("C++ compiler not found; skipping the surrogate pair runtime test");
        return;
    };
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("cpp_json_surrogates");
    fs::create_dir_all(&dir).expect("create temp directory");

    for (mode, compat) in [("plain", false), ("compat", true)] {
        let generator = if compat {
            CppGenerator::with_fastdds_compat()
        } else {
            CppGenerator::new()
        };
        let mut parser = Parser::try_new("struct Msg { string s; };").expect("lexer");
        let ast = parser.parse().expect("parse");
        let header = generator.generate(&ast).expect("codegen");
        fs::write(dir.join(format!("{mode}.h")), header).expect("write header");

        let member = if compat { "v.s()" } else { "v.s" };
        let mut checks = String::new();
        for (i, (json, bytes)) in CASES.iter().enumerate() {
            let code = i + 1;
            write!(
                checks,
                "    {{\n        Msg v{{}};\n        const char* json = R\"({{\"s\": \"{json}\"}})\";\n        \
                 if (!MsgPubSubType::populate_from_json(&v, json)) return {code};\n        \
                 const unsigned char want[] = {{{}}};\n        \
                 const std::string& got = {member};\n        \
                 if (got.size() != sizeof want || std::memcmp(got.data(), want, sizeof want) != 0) return {};\n    }}\n",
                c_bytes(bytes),
                code + 100,
            )
            .expect("write to a String");
        }
        let source = format!(
            "#include \"{mode}.h\"\n#include <cstring>\n#include <string>\nint main() {{\n{checks}    return 0;\n}}\n"
        );
        let src_path = dir.join(format!("{mode}.cxx"));
        fs::write(&src_path, source).expect("write probe");
        let bin = dir.join(format!("{mode}-probe"));
        let out = Command::new(&cxx)
            .args(["-std=c++17", "-Wall", "-Wextra", "-pedantic", "-Werror"])
            .arg("-I")
            .arg(&dir)
            .arg(&src_path)
            .arg("-o")
            .arg(&bin)
            .output()
            .expect("compile probe");
        assert!(
            out.status.success(),
            "{mode}: probe failed to compile\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let status = Command::new(&bin).status().expect("run probe");
        let code = status.code().unwrap_or(-1);
        assert!(
            code == 0,
            "{mode}: json_unescape gave the wrong bytes for case {} (exit {code}: 1-{} = populate refused, 101-{} = wrong bytes)",
            if code > 100 { code - 100 } else { code },
            CASES.len(),
            100 + CASES.len(),
        );
    }
}
