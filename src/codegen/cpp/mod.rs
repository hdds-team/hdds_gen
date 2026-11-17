// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! C++ code generator
//!
//! Generates C++ code compatible with DDS implementations (`FastDDS`, `Cyclone DDS`, etc.)

use crate::ast::Definition;
use crate::codegen::CodeGenerator;
use crate::error::Result;
use crate::types::{IdlType, PrimitiveType};
use crate::IdlFile;

mod annotations;
mod bitmasks;
mod bitsets;
mod codec;
mod consts;
mod enums;
mod forward;
mod helpers;
mod index;
mod key_hash;
mod structs;
mod typedefs;
mod unions;

use helpers::push_fmt;
use index::DefinitionIndex;

#[cfg(test)]
mod tests;

/// Generates C++ code compatible with DDS implementations.
pub struct CppGenerator {
    indent_level: usize,
    /// When true, generates `FastDDS`-compatible getter/setter methods
    /// instead of public member fields. This allows seamless switching
    /// between `FastDDS` and HDDS backends.
    pub(crate) fastdds_compat: bool,
    /// Dependency ASTs for cross-module type resolution in CDR2 codec.
    /// Types from these files are indexed but not emitted.
    dep_asts: Vec<IdlFile>,
    /// C++ namespace (`Outer::Inner`) every generated type is declared in;
    /// the runtime helpers stay outside it.
    namespace: Vec<String>,
    /// IDL modules enclosing the type being generated, outermost first
    /// (empty at the root): the scope of its `hdds_type_name`.
    scope: Vec<String>,
}

impl CppGenerator {
    /// Creates a new C++ generator with default settings.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            indent_level: 0,
            fastdds_compat: false,
            dep_asts: Vec::new(),
            namespace: Vec::new(),
            scope: Vec::new(),
        }
    }

    /// Create a generator with `FastDDS` compatibility mode enabled.
    /// This generates getter/setter methods for struct fields.
    #[must_use]
    pub const fn with_fastdds_compat() -> Self {
        Self {
            indent_level: 0,
            fastdds_compat: true,
            dep_asts: Vec::new(),
            namespace: Vec::new(),
            scope: Vec::new(),
        }
    }

    /// Declare every generated type inside the C++ namespace `ns`
    /// (`Outer` or `Outer::Inner`), the runtime helpers staying outside it.
    /// An absolute IDL name (`::A::T`) is then written `::Outer::A::T`.
    #[must_use]
    pub fn with_namespace(mut self, ns: &str) -> Self {
        self.namespace = ns
            .split("::")
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect();
        self
    }

    /// Set dependency ASTs for cross-module type resolution.
    pub fn set_deps(&mut self, deps: Vec<IdlFile>) {
        self.dep_asts = deps;
    }

    /// The generator for the body of the IDL module `module`.
    #[must_use]
    pub(super) fn nested(&self, module: &str) -> Self {
        let mut scope = self.scope.clone();
        scope.push(module.to_string());
        Self {
            indent_level: self.indent_level + 1,
            fastdds_compat: self.fastdds_compat,
            dep_asts: Vec::new(), // nested generators don't need deps (index is shared)
            namespace: Vec::new(),
            scope,
        }
    }

    /// IDL qualified name (`Mod::Sub::Name`) of the type `name` declared in
    /// the current module: the name the Rust backend announces.
    #[must_use]
    pub(super) fn type_name(&self, name: &str) -> String {
        crate::codegen::idl_qualified_name(&self.scope, name)
    }

    #[must_use]
    pub(super) fn indent(&self) -> String {
        "    ".repeat(self.indent_level)
    }

    fn append_definition(&self, out: &mut String, def: &Definition, idx: &DefinitionIndex) {
        match def {
            Definition::Module(module) => {
                push_fmt(
                    out,
                    format_args!("{}namespace {} {{\n\n", self.indent(), module.name),
                );
                let nested = self.nested(&module.name);
                for inner in &module.definitions {
                    nested.append_definition(out, inner, idx);
                }
                push_fmt(
                    out,
                    format_args!("{}}}  // namespace {}\n\n", self.indent(), module.name),
                );
            }
            Definition::Struct(s) => out.push_str(&structs::generate_struct(self, s, idx)),
            Definition::Enum(e) => out.push_str(&enums::generate_enum(self, e)),
            Definition::Typedef(t) => out.push_str(&typedefs::generate_typedef(self, t)),
            Definition::Const(c) => out.push_str(&consts::generate_const(self, c)),
            Definition::Union(u) => out.push_str(&unions::generate_union(self, u, idx)),
            Definition::Bitset(b) => out.push_str(&bitsets::generate_bitset(self, b)),
            Definition::Bitmask(m) => out.push_str(&bitmasks::generate_bitmask(self, m)),
            Definition::ForwardDecl(f) => out.push_str(&forward::generate_forward_decl(self, f)),
            Definition::AnnotationDecl(a) => {
                out.push_str(&annotations::generate_annotation_decl(self, a));
            }
            #[cfg(feature = "interfaces")]
            Definition::Interface(_) => {}
            #[cfg(feature = "interfaces")]
            Definition::Exception(_) => {}
        }
    }

    fn push_header_preamble(output: &mut String, includes: &CppIncludes) {
        output.push_str("// Generated by hddsgen v");
        output.push_str(env!("HDDS_VERSION"));
        output.push_str("\n// DO NOT EDIT\n\n");
        output.push_str("#pragma once\n\n");
        // Always needed for CDR2 codec
        output.push_str("#include <cstddef>\n");
        output.push_str("#include <cstdint>\n");
        output.push_str("#include <cstring>\n");
        // Conditional includes based on IDL types used
        if includes.needs_array {
            output.push_str("#include <array>\n");
        }
        if includes.needs_cstdlib {
            output.push_str("#include <cstdlib>\n");
            output.push_str("#include <new>\n");
            output.push_str("#include <utility>\n");
        }
        if includes.needs_map {
            output.push_str("#include <map>\n");
        }
        if includes.needs_optional {
            output.push_str("#include <optional>\n");
        }
        if includes.needs_string {
            output.push_str("#include <string>\n");
        }
        // The XCDR2 codec converts through std::remove_reference_t.
        output.push_str("#include <type_traits>\n");
        if includes.needs_vector {
            output.push_str("#include <vector>\n");
        }
        output.push('\n');
    }

    fn push_fixed_support(output: &mut String) {
        output.push_str("// Fixed-point raw storage (CDR2 uses 128-bit little-endian values)\n");
        output.push_str("template<unsigned int D, unsigned int S>\n");
        output.push_str("struct Fixed {\n");
        output.push_str("    std::int64_t high;\n");
        output.push_str("    std::uint64_t low;\n\n");
        output.push_str(
            "    static constexpr Fixed from_raw(std::int64_t hi, std::uint64_t lo) noexcept {\n",
        );
        output.push_str("        return Fixed{hi, lo};\n");
        output.push_str("    }\n\n");
        output.push_str("    inline std::array<std::uint8_t, 16> to_le_bytes() const noexcept {\n");
        output.push_str("        std::array<std::uint8_t, 16> out{};\n");
        output.push_str("        std::uint64_t lo_word = low;\n");
        output.push_str("        std::uint64_t hi_word = static_cast<std::uint64_t>(high);\n");
        output.push_str("        for (std::size_t i = 0; i < 8; ++i) {\n");
        output.push_str(
            "            out[i] = static_cast<std::uint8_t>((lo_word >> (i * 8)) & 0xFFu);\n",
        );
        output.push_str(
            "            out[8 + i] = static_cast<std::uint8_t>((hi_word >> (i * 8)) & 0xFFu);\n",
        );
        output.push_str("        }\n");
        output.push_str("        return out;\n");
        output.push_str("    }\n\n");
        output.push_str("    static inline Fixed from_le_bytes(const std::array<std::uint8_t, 16>& bytes) noexcept {\n");
        output.push_str("        std::uint64_t lo_word = 0;\n");
        output.push_str("        std::uint64_t hi_word = 0;\n");
        output.push_str("        for (std::size_t i = 0; i < 8; ++i) {\n");
        output
            .push_str("            lo_word |= static_cast<std::uint64_t>(bytes[i]) << (i * 8);\n");
        output.push_str(
            "            hi_word |= static_cast<std::uint64_t>(bytes[8 + i]) << (i * 8);\n",
        );
        output.push_str("        }\n");
        output.push_str("        return Fixed{static_cast<std::int64_t>(hi_word), lo_word};\n");
        output.push_str("    }\n\n");
        output.push_str("    constexpr std::int64_t high_word() const noexcept { return high; }\n");
        output.push_str("    constexpr std::uint64_t low_word() const noexcept { return low; }\n");
        output.push_str("};\n");
        output.push_str(
            "static_assert(sizeof(Fixed<1, 0>) == 16, \"Fixed storage must be 16 bytes\");\n\n",
        );
    }
}

impl Default for CppGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGenerator for CppGenerator {
    fn generate(&self, ast: &IdlFile) -> Result<String> {
        // References whose spelling a scope-less lookup would misread are
        // written as their absolute global name (IDL 4.2 7.5), and module
        // `@autoid` is carried by each type.
        let ast = &crate::scope::for_codegen(ast);
        let mut output = String::new();
        let includes = CppIncludes::from_ast(ast);

        Self::push_header_preamble(&mut output, &includes);

        // Add CDR2 helpers
        output.push_str(&codec::generate_cdr2_helpers());

        if uses_fixed_cpp(ast) {
            Self::push_fixed_support(&mut output);
        }

        // Build definition index for codec generation
        // Include dependency types for cross-module CDR2 encode/decode resolution
        let (mut idx, _) = DefinitionIndex::from_file(ast);
        if !self.dep_asts.is_empty() {
            idx.index_deps(&self.dep_asts);
        }

        // The KeyHash runtime, before the first type that uses it.
        if idx.has_key_holder() {
            output.push_str(&key_hash::runtime_section());
        }

        let mut types = String::new();
        for def in &ast.definitions {
            self.append_definition(&mut types, def, &idx);
        }
        if self.namespace.is_empty() {
            output.push_str(&types);
        } else {
            let mut roots: Vec<String> = root_names(ast);
            for dep in &self.dep_asts {
                roots.extend(root_names(dep));
            }
            for part in &self.namespace {
                push_fmt(&mut output, format_args!("namespace {part} {{\n"));
            }
            output.push('\n');
            output.push_str(&prefix_absolute(&types, &roots, &self.namespace.join("::")));
            for part in self.namespace.iter().rev() {
                push_fmt(&mut output, format_args!("}} // namespace {part}\n"));
            }
        }

        Ok(output)
    }
}

/// Tracks which C++ standard library headers are needed based on IDL types.
// Each bool maps to a single C++ header inclusion flag
#[allow(clippy::struct_excessive_bools)]
#[derive(Default)]
struct CppIncludes {
    needs_array: bool,    // std::array (Array or bounded string)
    needs_cstdlib: bool,  // std::malloc/free (PubSubType for structs)
    needs_map: bool,      // std::map (Map types)
    needs_optional: bool, // std::optional (optional fields, unions)
    needs_string: bool,   // std::string (string/wstring types)
    needs_vector: bool,   // std::vector (unbounded Sequence)
}

impl CppIncludes {
    fn from_ast(ast: &IdlFile) -> Self {
        let mut inc = Self::default();
        for def in &ast.definitions {
            inc.scan_definition(def);
        }
        inc
    }

    fn scan_definition(&mut self, def: &Definition) {
        match def {
            Definition::Struct(s) => {
                self.needs_cstdlib = true; // PubSubType uses malloc/free
                self.needs_string = true; // populate_from_json uses json_unescape returning std::string
                for field in &s.fields {
                    if field.is_optional() {
                        self.needs_optional = true;
                    }
                    self.scan_type(&field.field_type);
                }
            }
            Definition::Union(u) => {
                self.needs_optional = true;
                self.scan_type(&u.discriminator);
                for case in &u.cases {
                    self.scan_type(&case.field.field_type);
                }
            }
            Definition::Enum(_) => {}
            Definition::Typedef(t) => {
                self.scan_type(&t.base_type);
            }
            Definition::Module(m) => {
                for inner in &m.definitions {
                    self.scan_definition(inner);
                }
            }
            Definition::Bitset(_) => {}
            Definition::Bitmask(_) | Definition::Const(_) => {}
            Definition::ForwardDecl(_) | Definition::AnnotationDecl(_) => {}
            #[cfg(feature = "interfaces")]
            Definition::Interface(_) => {}
            #[cfg(feature = "interfaces")]
            Definition::Exception(_) => {}
        }
    }

    fn scan_type(&mut self, t: &IdlType) {
        match t {
            // `string<N>`: a `std::array` of characters.
            IdlType::BoundedString { .. } => self.needs_array = true,
            IdlType::Primitive(p) => match p {
                PrimitiveType::String | PrimitiveType::WString => {
                    self.needs_string = true;
                }
                PrimitiveType::Fixed { .. } => {
                    self.needs_array = true;
                }
                _ => {}
            },
            IdlType::Sequence { inner, .. } => {
                self.needs_vector = true;
                self.scan_type(inner);
            }
            IdlType::Array { inner, .. } => {
                self.needs_array = true;
                self.scan_type(inner);
            }
            IdlType::Map { key, value, .. } => {
                self.needs_map = true;
                self.scan_type(key);
                self.scan_type(value);
            }
            IdlType::Named(_) => {}
        }
    }
}

fn uses_fixed_cpp(ast: &IdlFile) -> bool {
    fn type_has_fixed(t: &IdlType) -> bool {
        match t {
            IdlType::Primitive(PrimitiveType::Fixed { .. }) => true,
            IdlType::Sequence { inner, .. } | IdlType::Array { inner, .. } => type_has_fixed(inner),
            IdlType::Map { key, value, .. } => type_has_fixed(key) || type_has_fixed(value),
            _ => false,
        }
    }
    for def in &ast.definitions {
        match def {
            Definition::Struct(s) => {
                if s.fields.iter().any(|f| type_has_fixed(&f.field_type)) {
                    return true;
                }
            }
            Definition::Typedef(t) => {
                if type_has_fixed(&t.base_type) {
                    return true;
                }
            }
            Definition::Union(u) => {
                if u.cases.iter().any(|c| type_has_fixed(&c.field.field_type)) {
                    return true;
                }
            }
            Definition::Module(m) => {
                let sub = IdlFile {
                    definitions: m.definitions.clone(),
                };
                if uses_fixed_cpp(&sub) {
                    return true;
                }
            }
            Definition::Bitmask(_) | Definition::Bitset(_) | Definition::Const(_) => {}
            Definition::Enum(_) => {}
            Definition::ForwardDecl(_) => {}
            Definition::AnnotationDecl(_) => {}
            #[cfg(feature = "interfaces")]
            Definition::Interface(_) => {}
            #[cfg(feature = "interfaces")]
            Definition::Exception(_) => {}
        }
    }
    false
}

/// The names `ast` declares at the root (modules and types), which an
/// absolute name starts with.
fn root_names(ast: &IdlFile) -> Vec<String> {
    ast.definitions
        .iter()
        .filter_map(|d| match d {
            Definition::Module(m) => Some(m.name.clone()),
            other => crate::scope::declared_name(other).map(|(n, _)| n.to_string()),
        })
        .collect()
}

/// `code` with every absolute name that starts at an IDL root name
/// (`::A::T`, `::T`) moved under the namespace `ns` (`::Outer::A::T`). An
/// occurrence counts when `::` is not itself preceded by an identifier
/// character or a colon, and the root name is followed by a non-identifier
/// character (or by `PubSubType`, the support class of a root type);
/// `::std::`, `::hdds::` and the like are untouched unless the IDL declares
/// such a root name.
fn prefix_absolute(code: &str, roots: &[String], ns: &str) -> String {
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let bytes = code.as_bytes();
    let mut out = String::with_capacity(code.len());
    let mut i = 0;
    let mut copied = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b':'
            && bytes[i + 1] == b':'
            && (i == 0 || !(ident(bytes[i - 1]) || bytes[i - 1] == b':'))
        {
            let rest = &code[i + 2..];
            let hit = roots.iter().any(|r| {
                rest.starts_with(r.as_str())
                    && (rest.as_bytes().get(r.len()).is_none_or(|&c| !ident(c))
                        || rest[r.len()..].starts_with("PubSubType"))
            });
            if hit {
                out.push_str(&code[copied..i]);
                out.push_str("::");
                out.push_str(ns);
                copied = i;
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&code[copied..]);
    out
}
