// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! IDL 4.2 Parser and Code Generator
//!
//! This crate provides parsing and code generation for OMG IDL 4.2 specification.
//! It supports generating code for multiple target languages including Rust, C++, C, and Python.
//!
//! # Example
//!
//! ```
//! use hddsgen::Parser;
//!
//! let idl = r#"
//!     struct Point {
//!         int32_t x;
//!         int32_t y;
//!     };
//! "#;
//!
//! let mut parser = Parser::try_new(idl).expect("Lexer error");
//! let ast = parser.parse().expect("Failed to parse IDL");
//! ```

#![warn(clippy::all)]
#![allow(clippy::module_inception)]

pub mod ast;
pub mod codegen;
pub mod core;
pub mod error;
mod lexer;
pub mod parser;
pub mod preprocessor;
pub mod pretty;
pub mod scope;
pub mod token;
pub mod types;
pub mod validate;

#[cfg(test)]
mod test_extra;

/// Full version string: "MAJOR.MINOR.BUILD" (BUILD auto-incremented on each `cargo build`)
pub const VERSION: &str = env!("HDDS_VERSION");

/// Build number only (auto-incremented)
pub const BUILD_NUMBER: &str = env!("HDDS_BUILD_NUMBER");

// Re-exports for convenience
pub use ast::{Definition, Field, ForwardDecl, ForwardKind, IdlFile, Module, Struct};
pub use codegen::{Backend, CodeGenerator};
pub use error::{ParseError, Result};
pub use parser::Parser;
pub use pretty::to_idl as idl_pretty;
pub use types::{Annotation, IdlType, PrimitiveType};
pub use validate::validate;
