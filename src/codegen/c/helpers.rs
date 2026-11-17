// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Helper utilities for C code generation.
//!
//! Name manipulation, type mapping, and formatting helpers.

use std::fmt::Write;

#[must_use]
pub fn last_ident(name: &str) -> &str {
    if let Some((_, tail)) = name.rsplit_once("::") {
        tail
    } else if let Some((_, tail)) = name.rsplit_once('.') {
        tail
    } else {
        name
    }
}

#[must_use]
pub fn c_name(name: &str) -> String {
    last_ident(name)
        .to_ascii_lowercase()
        .replace("::", "_")
        .replace('.', "_")
}

/// Append formatted text to `dst`.
pub fn push_fmt(dst: &mut String, args: std::fmt::Arguments<'_>) {
    let _ = dst.write_fmt(args);
}

#[must_use]
pub fn to_upper_ascii(s: &str) -> String {
    s.chars().map(|c| c.to_ascii_uppercase()).collect()
}
