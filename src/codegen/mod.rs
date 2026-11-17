// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Code generation module
//!
//! Generates code from IDL AST to various target languages.

/// Language keyword escaping for generated identifiers.
pub mod keywords;

/// C code generator.
pub mod c;
/// Constant values as literals of each target language.
pub mod const_literal;
pub use c::CStandard;
/// C micro (header-only) code generator for embedded MCUs.
pub mod c_micro;
/// C++ code generator.
pub mod cpp;
/// The DDS-XTypes 1.3 data model shared by the backends' serializers.
pub mod data_model;
/// Example code generator for demonstration (inline main()).
pub mod examples;
/// Project generation for --example flag (publisher/subscriber projects).
pub mod examples_project;
/// Instance `KeyHash` model shared by the backends.
pub mod key_holder;
/// Least XCDR2 size of a value, shared by the decoders' count bounds.
pub(crate) mod least_wire;
/// Micro (no_std Rust) code generator for embedded targets.
pub mod micro;
/// Python code generator.
pub mod python;
/// Rust code generator.
pub mod rust_backend;
/// Absolute IDL names as Rust paths (Rust and micro backends).
pub mod rust_paths;
/// TypeScript code generator.
pub mod typescript;

use crate::ast::IdlFile;
use crate::error::Result;

/// The IDL qualified name of a type declared in the modules `scope`
/// (outermost first): `Mod::Sub::Name`, or `Name` at the root. This is the
/// type name the Rust backend announces (the `type_name` of its
/// `TypeDescriptor`, the detail name of its `TypeObject`) and the C++
/// backend repeats as `hdds_type_name`.
#[must_use]
pub(crate) fn idl_qualified_name<S: AsRef<str>>(scope: &[S], name: &str) -> String {
    let mut out = String::new();
    for module in scope {
        out.push_str(module.as_ref());
        out.push_str("::");
    }
    out.push_str(name);
    out
}

/// Code generation backend trait.
///
/// A struct, union, enum or bitmask without an extensibility annotation is
/// generated as APPENDABLE, the DDS-XTypes 1.3 default (7.3.1.2.1.8). The
/// `hddsgen gen --default-extensibility` option is applied to the AST
/// before generation; a library user does the same with
/// [`data_model::apply_default_extensibility`], which also returns the
/// names of the types it concerned:
///
/// ```
/// use hddsgen::codegen::data_model::apply_default_extensibility;
/// use hddsgen::types::ExtensibilityKind;
/// use hddsgen::{Backend, Parser};
///
/// let ast = Parser::try_new("struct P { long x; };")?.parse()?;
/// let appendable = Backend::Rust.generator().generate(&ast)?;
/// assert!(appendable.contains("begin_len()?"));
///
/// let mut ast = Parser::try_new("struct P { long x; };")?.parse()?;
/// let unannotated = apply_default_extensibility(&mut ast, ExtensibilityKind::Final);
/// assert_eq!(unannotated, ["P"]);
/// let code = Backend::Rust.generator().generate(&ast)?;
/// assert!(!code.contains("begin_len()?"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
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
