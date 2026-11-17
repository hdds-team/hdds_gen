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

/// Hashed member ID (`XTypes` 1.3 ss7.3.1.2.1.2): first 4 bytes of MD5(name), LE, masked to 28 bits.
pub(crate) fn hash_member_id(name: &str) -> u32 {
    use md5::{Digest, Md5};
    let d = Md5::digest(name.as_bytes());
    u32::from_le_bytes([d[0], d[1], d[2], d[3]]) & 0x0fff_ffff
}

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

#[cfg(test)]
mod tests {
    use super::{hash_member_id, Backend};

    #[test]
    fn hashed_member_ids_use_md5_per_xtypes_7_3_1_2_1_2() {
        // Expected values: MD5(name)[0..4] as LE u32, masked to 28 bits.
        assert_eq!(hash_member_id("sensor_id"), 0x0A86_6DD9);
        assert_eq!(hash_member_id("temperature"), 0x0738_F4EE);
        assert_eq!(hash_member_id("points"), 0x0E81_AB0A);
    }

    #[test]
    fn all_backends_emit_the_same_hashed_member_ids() {
        let idl = "@mutable struct Reading { long sensor_id; double temperature; };";
        let ast = crate::Parser::try_new(idl)
            .expect("lexer error")
            .parse()
            .expect("parse error");
        for backend in [
            Backend::Rust,
            Backend::Cpp,
            Backend::Python,
            Backend::TypeScript,
        ] {
            let code = backend.generator().generate(&ast).expect("codegen error");
            assert!(code.contains("0x0A866DD9"), "sensor_id id missing:\n{code}");
            assert!(
                code.contains("0x0738F4EE"),
                "temperature id missing:\n{code}"
            );
        }
    }
}
