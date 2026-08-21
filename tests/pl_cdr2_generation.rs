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

    // MemberIds (autoid hash: MD5(name)[0..4] LE & 0x0FFF_FFFF, XTypes 1.3 7.3.1.2.1.2)
    assert!(
        code.contains("0x0E81AB0A"),
        "points MemberId should be present"
    );
    assert!(
        code.contains("0x0093D814"),
        "altitude MemberId should be present"
    );

    // PL_CDR2 framing (delimiter length)
    assert!(
        code.contains("let payload_len = u32::try_from(offset - 4)"),
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

    // @key fields are implicitly must_understand -> M bit (0x8000_0000) must be set
    assert!(
        code.contains("0x8000_0000"),
        "must_understand bit should be present in generated code for @key or @must_understand fields"
    );

    // Unknown member handling: must check must_understand flag before rejecting
    assert!(
        code.contains("must_understand"),
        "decoder should check must_understand flag for unknown members"
    );

    // Unknown member handling: should skip if not must_understand
    assert!(
        code.contains("offset = member_end"),
        "decoder should skip unknown non-must_understand members"
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
        !code.contains("0x8000_0000"),
        "normal field should not have must_understand bit set"
    );

    // member_len should use LC-based sizes, not fallback
    assert!(
        code.contains("match lc { 0 => 1, 1 => 2, 2 => 4, 3 => 8,"),
        "decoder should compute member_len from LC code"
    );
}
