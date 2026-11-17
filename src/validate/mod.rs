// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Validation pass for IDL AST (selected rules).

mod collect;
mod collisions;
mod diagnostics;
mod engine;
mod references;
mod rules;
#[cfg(test)]
mod tests;

pub use diagnostics::{Level, ValidationDiag};
pub use engine::validate;
