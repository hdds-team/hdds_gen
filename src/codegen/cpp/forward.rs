// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Forward declaration generation for C++.
//!
//! Generates forward declarations for struct and union types.

use super::helpers::push_fmt;
use super::CppGenerator;
use crate::ast::{ForwardDecl, ForwardKind};

pub(super) fn generate_forward_decl(generator: &CppGenerator, decl: &ForwardDecl) -> String {
    let mut output = String::new();
    let indent = generator.indent();
    // An IDL union is generated as a C++ `struct` (discriminator and
    // storage), so its forward declaration is one too: a class-key
    // mismatch is an error for `union`.
    let keyword = match decl.kind {
        ForwardKind::Struct | ForwardKind::Union => "struct",
    };
    let name = &decl.name;
    push_fmt(&mut output, format_args!("{indent}{keyword} {name};\n\n"));
    output
}
