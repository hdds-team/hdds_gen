// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Code generation module
//!
//! Generates code from IDL AST to various target languages.

/// Language keyword escaping for generated identifiers.
pub mod keywords;

/// C code generator.
pub mod c;
pub use c::CStandard;
/// C micro (header-only) code generator for embedded MCUs.
pub mod c_micro;
/// C++ code generator.
pub mod cpp;
/// Example code generator for demonstration (inline main()).
pub mod examples;
/// Project generation for --example flag (publisher/subscriber projects).
pub mod examples_project;
/// Micro (no_std Rust) code generator for embedded targets.
pub mod micro;
/// Python code generator.
pub mod python;
/// Rust code generator.
pub mod rust_backend;
/// TypeScript code generator.
pub mod typescript;

use crate::ast::IdlFile;
use crate::error::Result;

/// Code generation backend trait
pub trait CodeGenerator {
    /// Generate code from IDL AST
    ///
    /// # Errors
    ///
    /// Returns an error if the target backend cannot render the provided AST.
    fn generate(&self, ast: &IdlFile) -> Result<String>;
}

/// Available code generation backends
pub enum Backend {
    /// C++ code compatible with DDS implementations.
    Cpp,
    /// Idiomatic Rust with CDR2 serialization.
    Rust,
    /// Python dataclasses with type hints.
    Python,
    /// C89/C99/C11 header-only code.
    C,
    /// Rust `no_std` for embedded (heapless).
    Micro,
    /// C header-only for MCUs (STM32, AVR, PIC).
    CMicro,
    /// TypeScript interfaces with CDR2 serialization.
    TypeScript,
}

impl Backend {
    /// Get the appropriate code generator for this backend
    #[must_use]
    pub fn generator(&self) -> Box<dyn CodeGenerator> {
        match self {
            Self::Cpp => Box::new(cpp::CppGenerator::new()),
            Self::Rust => Box::new(rust_backend::RustGenerator::new()),
            Self::Python => Box::new(python::PythonGenerator::new()),
            Self::C => Box::new(c::CGenerator::new()),
            Self::Micro => Box::new(micro::MicroGenerator::new()),
            Self::CMicro => Box::new(c_micro::CMicroGenerator::new()),
            Self::TypeScript => Box::new(typescript::TypeScriptGenerator::new()),
        }
    }
}
