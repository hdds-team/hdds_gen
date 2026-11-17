// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Absolute IDL names in the Rust backends.
//!
//! An absolute IDL name (`::T`, `::a::T`, IDL 4.2 7.5) names a type from the
//! root scope, even inside a module that declares its own `T`. Rust has no
//! path for "the root of this generated file" (`::T` is an extern crate,
//! `crate::T` is wrong once the file is a submodule or `include!`d), so the
//! file declares, at its root, a module re-exporting the root's items;
//! every module reaches it through its `use super::*` chain, and `::a::T`
//! becomes `__hdds_root::a::T`.

/// The module through which absolute names are reached.
pub const ROOT: &str = "__hdds_root";

/// `name` as a Rust path: an absolute IDL name goes through [`ROOT`],
/// anything else is unchanged.
#[must_use]
pub fn path(name: &str) -> String {
    name.strip_prefix("::")
        .map_or_else(|| name.to_string(), |abs| format!("{ROOT}::{abs}"))
}

/// The [`ROOT`] module, declared at the root of every generated file:
/// absolute names and the `TypeObject` of a nested type (spelled from the
/// root, wherever the referencing type is) go through it.
#[must_use]
pub fn root_module() -> String {
    format!(
        "/// The root of this file, for the absolute IDL names (`::T`).\n\
         #[doc(hidden)]\n\
         #[allow(unused_imports)]\n\
         pub mod {ROOT} {{\n    pub use super::*;\n}}\n\n"
    )
}
