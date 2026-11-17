// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Generated-code smoke test for `PL_CDR2` mutable structs with optional members.

#![allow(clippy::expect_used)]

use std::fs;
use std::process::Command;

fn run_idl_gen(args: &[&str]) -> bool {
    let status = Command::new("cargo")
        .args(["run", "--quiet", "--bin", "hddsgen", "--"])
        .args(args)
        .status();
    matches!(status, Ok(s) if s.success())
}

#[test]
fn mutable_optional_poly3d_contains_member_ids_and_option() {
    let dir = std::env::temp_dir().join(format!("hdds_gen_plcdr2_{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);

    let idl = r"
        module Xcdr2Test {
            @autoid @mutable
            struct Point3D {
                double x;
                double y;
                double z;
            };

            @autoid @mutable
            struct Poly3D {
                sequence<Point3D> points;
                @optional double altitude;
            };
        };
    ";
    let idl_path = dir.join("xcdr2.idl");
    fs::write(&idl_path, idl).expect("write idl");

    let out_rs = dir.join("generated.rs");
    assert!(
        run_idl_gen(&[
            "gen",
            "rust",
            idl_path.to_str().expect("utf-8 path"),
            "-o",
            out_rs.to_str().expect("utf-8 path"),
        ]),
        "hddsgen rust failed"
    );

    let code = fs::read_to_string(&out_rs).expect("read generated");

    // Fields
    assert!(
        code.contains("pub altitude: Option<f64>"),
        "altitude should be optional"
    );

    // Member ids under @autoid (HASH): the first four bytes of the MD5 of
    // the name, little endian, masked to 28 bits (XTypes 1.3 7.3.1.2.1.1).
    assert!(
        code.contains("__e.emheader(false, 5, 0xe81ab0a)?;"),
        "points: MD5 member id, LC 5 (a sequence of structures has a DHEADER)"
    );
    assert!(
        code.contains("__e.emheader(false, 3, 0x93d814)?;"),
        "altitude: MD5 member id, LC 3 (8 bytes)"
    );
    assert!(
        code.contains("struct_flags: ::hdds::xtypes::StructTypeFlag(0x0014)"),
        "TypeObject: IS_MUTABLE | IS_AUTOID_HASH"
    );

    // PL_CDR2 framing: a DHEADER patched with the member bytes.
    assert!(
        code.contains("let __dh = __e.begin_len()?;") && code.contains("__e.end_len(__dh)?;"),
        "should compute delimiter length"
    );
}

#[test]
fn must_understand_bit_set_in_emheader() {
    let dir = std::env::temp_dir().join(format!("hdds_gen_mu_{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);

    let idl = r"
        module MuTest {
            @autoid @mutable
            struct Sensor {
                @key long id;
                @must_understand double critical_value;
                @optional float normal_value;
            };
        };
    ";
    let idl_path = dir.join("mu_test.idl");
    fs::write(&idl_path, idl).expect("write idl");

    let out_rs = dir.join("generated.rs");
    assert!(
        run_idl_gen(&[
            "gen",
            "rust",
            idl_path.to_str().expect("utf-8 path"),
            "-o",
            out_rs.to_str().expect("utf-8 path"),
        ]),
        "hddsgen rust failed"
    );

    let code = fs::read_to_string(&out_rs).expect("read generated");

    // @key and @must_understand members carry the M flag of EMHEADER1.
    assert!(
        code.contains("__e.emheader(true, 2, 0x4b70bb8)?;"),
        "@key id: M flag"
    );
    assert!(
        code.contains("__e.emheader(true, 3, 0xd557b69)?;"),
        "@must_understand critical_value: M flag"
    );
    assert!(
        code.contains("__e.emheader(false, 2, 0x22fe000)?;"),
        "normal_value: no M flag"
    );

    // Unknown member handling: rejected with the M flag, skipped without.
    assert!(
        code.contains("_ => { if __mu { return Err(::hdds::CdrError::InvalidEncoding); } }"),
        "decoder should check must_understand flag for unknown members"
    );
    assert!(
        code.contains("__d.skip_to(__end)?;"),
        "decoder should skip to the member end"
    );
}

#[test]
fn unknown_member_skip_without_must_understand() {
    let dir = std::env::temp_dir().join(format!("hdds_gen_skip_{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);

    let idl = r"
        module SkipTest {
            @autoid @mutable
            struct Simple {
                long value;
                @optional float extra;
            };
        };
    ";
    let idl_path = dir.join("skip_test.idl");
    fs::write(&idl_path, idl).expect("write idl");

    let out_rs = dir.join("generated.rs");
    assert!(
        run_idl_gen(&[
            "gen",
            "rust",
            idl_path.to_str().expect("utf-8 path"),
            "-o",
            out_rs.to_str().expect("utf-8 path"),
        ]),
        "hddsgen rust failed"
    );

    let code = fs::read_to_string(&out_rs).expect("read generated");

    // Non-@key, non-@must_understand field should NOT have M bit
    assert!(
        !code.contains("__e.emheader(true"),
        "normal field should not have must_understand bit set"
    );

    // The member length follows the length code (XTypes 1.3 7.4.3.4.2).
    assert!(
        code.contains("let size = match (h >> 28) & 7 {")
            && code.contains("let unit = match lc { 5 => 1, 6 => 4, _ => 8 };"),
        "decoder should compute the member length from the length code"
    );
}
