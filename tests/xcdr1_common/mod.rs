// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! XCDR1 test vectors shared by the data path tests of the C, C++, Python
//! and TypeScript backends, never produced by an hddsgen encoder:
//!
//! - `cyclone_vectors.txt`: the XCDR1 bodies Cyclone DDS 0.11 serializes
//!   for the corpus samples (`cyclone_probe.c`);
//! - `spec_xcdr1_vectors.txt`: the parameter lists of the same samples in
//!   the form DDS-XTypes 1.3 7.4.1.2.2 specifies (exact member lengths,
//!   `FLAG_2` on key members), derived from Cyclone's by Cyclone's reader;
//! - `spec_mutable_union*.txt`: @mutable unions (which Cyclone does not
//!   support) built by `spec_vectors.py` from the specification text, in
//!   two forms (extended headers with exact lengths, short headers with
//!   padded lengths);
//! - `spec_plprobe.txt`: one @mutable two-key sample laid out the ways a
//!   writer may lay it out (short, extended, flagged, unflagged headers,
//!   unknown members), each with the specification's verdict.
//!
//! A decoder reads each body; the value it reads must encode (XCDR2) to the
//! XCDR2 bytes of the same sample, which the data path tests already check
//! against Cyclone.

#![allow(dead_code)]

const VECTORS: &str = include_str!("../fixtures/xtypes_datapath/cyclone_vectors.txt");
const SPEC_XCDR1: &str = include_str!("../fixtures/xtypes_datapath/spec_xcdr1_vectors.txt");
const UNION: &str = include_str!("../fixtures/xtypes_datapath/spec_mutable_union.txt");
const UNION_H: &str = include_str!("../fixtures/xtypes_datapath/spec_mutable_union_hashed.txt");
const PLPROBE: &str = include_str!("../fixtures/xtypes_datapath/spec_plprobe.txt");

/// The type of `spec_plprobe.txt`.
pub const PLPROBE_IDL: &str = "@mutable struct PlProbe { @key @id(1) long id; @id(2) short s; \
    @id(3) string name; @id(16144) long long big; @key @id(5) octet tag; };\n";

/// Body of an encapsulated buffer (hex): the 4-byte header and the
/// trailing padding its options count removed.
#[must_use]
pub fn body_of(encapsulated: &str) -> &str {
    let pad = usize::from(u8::from_str_radix(&encapsulated[6..8], 16).unwrap_or(0) & 3);
    &encapsulated[8..encapsulated.len() - 2 * pad]
}

/// Every XCDR1 body of `sample` (label, hex): Cyclone's, the specification
/// form when it differs, both forms of a @mutable union sample.
#[must_use]
pub fn xcdr1_bodies(sample: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for l in VECTORS.lines() {
        let mut it = l.split_whitespace();
        if it.next() != Some(sample) || it.next() != Some("xcdr1") {
            continue;
        }
        let _hdr = it.next();
        if let Some(b) = it.next().and_then(|b| b.strip_prefix("body=")) {
            out.push((format!("{sample}/cyclone"), b.to_string()));
        }
    }
    for l in SPEC_XCDR1.lines().filter(|l| !l.starts_with('#')) {
        let mut it = l.split_whitespace();
        if it.next() != Some(sample) || it.next() != Some("xcdr1") {
            continue;
        }
        if let Some(b) = it.next().and_then(|b| b.strip_prefix("spec=")) {
            out.push((format!("{sample}/spec"), b.to_string()));
        }
    }
    for l in UNION
        .lines()
        .chain(UNION_H.lines())
        .filter(|l| !l.starts_with('#'))
    {
        let mut it = l.split_whitespace();
        if it.next() != Some(sample) || it.next() != Some("xcdr1") {
            continue;
        }
        for _ in 0..2 {
            let Some(f) = it.next() else { break };
            if let Some((form, hex)) = f.split_once('=') {
                out.push((format!("{sample}/{form}"), body_of(hex).to_string()));
            }
        }
    }
    out
}

/// The XCDR1 body Cyclone wrote for `sample`.
#[must_use]
pub fn cyclone_xcdr1(sample: &str) -> String {
    xcdr1_bodies(sample)
        .into_iter()
        .find(|(l, _)| l.ends_with("/cyclone"))
        .map_or_else(|| panic!("no Cyclone XCDR1 body for {sample}"), |(_, b)| b)
}

/// The specification-form XCDR1 body of `sample`.
#[must_use]
pub fn spec_xcdr1(sample: &str) -> String {
    xcdr1_bodies(sample)
        .into_iter()
        .find(|(l, _)| l.ends_with("/spec"))
        .map_or_else(
            || panic!("no specification XCDR1 body for {sample}"),
            |(_, b)| b,
        )
}

/// One layout of `spec_plprobe.txt`.
pub struct PlProbe {
    pub label: String,
    pub accept: bool,
    /// XCDR1 (`PL_CDR_LE`) or XCDR2 (`PL_CDR2_LE`).
    pub xcdr1: bool,
    /// The body, without the encapsulation header.
    pub body: String,
}

/// Every layout of `spec_plprobe.txt`.
#[must_use]
pub fn plprobe() -> Vec<PlProbe> {
    PLPROBE
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let (label, verdict, hex) = (it.next()?, it.next()?, it.next()?);
            Some(PlProbe {
                label: label.to_string(),
                accept: verdict == "ACCEPT",
                xcdr1: hex.starts_with("0003"),
                body: hex[8..].to_string(),
            })
        })
        .collect()
}

/// The XCDR2 body hddsgen writes for the `spec_plprobe.txt` sample.
#[must_use]
pub fn plprobe_xcdr2() -> String {
    plprobe()
        .into_iter()
        .find(|p| p.label == "X2-S1")
        .map(|p| p.body)
        .expect("X2-S1")
}

/// `body` (a parameter list) behind a `PID_IGNORE` parameter carrying
/// `FLAG_M` (0x7F03, length 0), which a reader skips (7.4.1.2.2).
#[must_use]
pub fn with_pid_ignore(body: &str) -> String {
    format!("037f0000{body}")
}
