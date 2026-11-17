// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Abstract Syntax Tree (AST) types
//!
//! Defines the structure of parsed IDL code.

use crate::types::{Annotation, ExtensibilityKind, IdlType};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Root of the AST - a complete IDL file
pub struct IdlFile {
    pub definitions: Vec<Definition>,
}

impl IdlFile {
    /// Creates an empty IDL file with no definitions.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            definitions: Vec::new(),
        }
    }

    /// Adds a top-level definition to the IDL file.
    pub fn add_definition(&mut self, def: Definition) {
        self.definitions.push(def);
    }
}

impl Default for IdlFile {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Top-level definition in IDL
pub enum Definition {
    /// Module (namespace)
    Module(Module),

    /// Struct definition
    Struct(Struct),

    /// Typedef
    Typedef(Typedef),

    /// Enum definition
    Enum(Enum),

    /// Union definition
    Union(Union),

    /// Constant definition
    Const(Const),

    /// Bitset definition (IDL 4.2)
    Bitset(Bitset),

    /// Bitmask definition (IDL 4.2)
    Bitmask(Bitmask),

    /// Custom annotation declaration (IDL 4.2): @annotation Name { type member [default value]; };
    AnnotationDecl(AnnotationDecl),

    /// Forward declaration (struct Foo; or union Bar;)
    ForwardDecl(ForwardDecl),

    /// Interface declaration (feature: interfaces)
    #[cfg(feature = "interfaces")]
    Interface(Interface),
    /// Exception declaration (feature: interfaces)
    #[cfg(feature = "interfaces")]
    Exception(Exception),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Module (namespace) definition
pub struct Module {
    pub name: String,
    pub definitions: Vec<Definition>,
    /// Annotations on the module, as `@autoid(HASH) module M { ... }`.
    pub annotations: Vec<Annotation>,
}

impl Module {
    /// Creates a new empty module with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            definitions: Vec::new(),
            annotations: Vec::new(),
        }
    }

    /// Adds a definition to this module.
    pub fn add_definition(&mut self, def: Definition) {
        self.definitions.push(def);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Struct definition
pub struct Struct {
    pub name: String,
    pub base_struct: Option<String>, // For inheritance: struct Foo : Bar
    pub annotations: Vec<Annotation>,
    pub fields: Vec<Field>,
    pub extensibility: Option<ExtensibilityKind>,
}

impl Struct {
    /// Creates a new empty struct with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            base_struct: None,
            annotations: Vec::new(),
            fields: Vec::new(),
            extensibility: None,
        }
    }

    /// Adds a field to this struct.
    pub fn add_field(&mut self, field: Field) {
        self.fields.push(field);
    }

    /// Adds an annotation to this struct.
    pub fn add_annotation(&mut self, annotation: Annotation) {
        self.annotations.push(annotation);
    }

    /// Get key fields (fields marked with @key annotation)
    #[must_use]
    pub fn key_fields(&self) -> Vec<&Field> {
        self.fields
            .iter()
            .filter(|f| f.annotations.iter().any(|a| matches!(a, Annotation::Key)))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Field in a struct or union
pub struct Field {
    pub name: String,
    pub field_type: IdlType,
    pub annotations: Vec<Annotation>,
}

impl Field {
    /// Creates a new field with the given name and type.
    #[must_use]
    pub fn new(name: impl Into<String>, field_type: IdlType) -> Self {
        Self {
            name: name.into(),
            field_type,
            annotations: Vec::new(),
        }
    }

    /// Returns the field with an annotation added (builder pattern).
    #[must_use]
    pub fn with_annotation(mut self, annotation: Annotation) -> Self {
        self.annotations.push(annotation);
        self
    }

    /// Returns true if this field has the `@key` annotation.
    #[must_use]
    pub fn is_key(&self) -> bool {
        self.annotations
            .iter()
            .any(|a| matches!(a, Annotation::Key))
    }

    /// Returns true if this field has the `@optional` annotation.
    #[must_use]
    pub fn is_optional(&self) -> bool {
        self.annotations
            .iter()
            .any(|a| matches!(a, Annotation::Optional))
    }

    /// Returns true if this field has the `@non_serialized` annotation.
    #[must_use]
    pub fn is_non_serialized(&self) -> bool {
        self.annotations
            .iter()
            .any(|a| matches!(a, Annotation::NonSerialized))
    }

    /// Returns true if this field has the `@must_understand` annotation.
    #[must_use]
    pub fn is_must_understand(&self) -> bool {
        self.annotations
            .iter()
            .any(|a| matches!(a, Annotation::MustUnderstand))
    }

    /// Returns true if this field has the `@external` annotation.
    #[must_use]
    pub fn is_external(&self) -> bool {
        self.annotations
            .iter()
            .any(|a| matches!(a, Annotation::External))
    }

    /// Get default value from @default(value) annotation, if present
    #[must_use]
    pub fn get_default(&self) -> Option<&str> {
        self.annotations.iter().find_map(|a| match a {
            Annotation::Value(v) => Some(v.value.as_str()),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Typedef definition
pub struct Typedef {
    pub name: String,
    pub base_type: IdlType,
    pub annotations: Vec<Annotation>,
}

impl Typedef {
    /// Creates a new typedef with the given name and base type.
    #[must_use]
    pub fn new(name: impl Into<String>, base_type: IdlType) -> Self {
        Self {
            name: name.into(),
            base_type,
            annotations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Enum definition
pub struct Enum {
    pub name: String,
    pub variants: Vec<EnumVariant>,
    pub annotations: Vec<Annotation>,
}

impl Enum {
    /// Creates a new empty enum with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            variants: Vec::new(),
            annotations: Vec::new(),
        }
    }

    /// Adds a variant to this enum.
    pub fn add_variant(&mut self, variant: EnumVariant) {
        self.variants.push(variant);
    }

    /// The value of every enumerator, in declaration order.
    ///
    /// An enumerator with an explicit value has it; one without takes the
    /// previous enumerator's value plus one, and the first takes 0 (as in C
    /// and C++, and DDS-XTypes 1.3 7.3.1.2.1.5). Every backend numbers an
    /// enumeration through this, so two peers generated from the same IDL
    /// agree on the wire.
    #[must_use]
    pub fn resolved_values(&self) -> Vec<i64> {
        let mut next: i64 = 0;
        self.variants
            .iter()
            .map(|v| {
                let value = v.value.unwrap_or(next);
                next = value.saturating_add(1);
                value
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Enum variant
pub struct EnumVariant {
    pub name: String,
    pub value: Option<i64>,
}

impl EnumVariant {
    /// Creates a new enum variant with an optional explicit value.
    #[must_use]
    pub fn new(name: impl Into<String>, value: Option<i64>) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Union definition
pub struct Union {
    pub name: String,
    pub discriminator: IdlType,
    pub cases: Vec<UnionCase>,
    pub annotations: Vec<Annotation>,
    /// Annotations on the discriminator, as in `switch (@key Kind)`.
    pub discriminator_annotations: Vec<Annotation>,
}

impl Union {
    /// Creates a new union with the given name and discriminator type.
    #[must_use]
    pub fn new(name: impl Into<String>, discriminator: IdlType) -> Self {
        Self {
            name: name.into(),
            discriminator,
            cases: Vec::new(),
            annotations: Vec::new(),
            discriminator_annotations: Vec::new(),
        }
    }

    /// Whether the discriminator is a key (`switch (@key Kind)`), which makes
    /// the union's `KeyHolder` the discriminator alone (`XTypes` 1.3 7.2.2.4.7).
    #[must_use]
    pub fn discriminator_is_key(&self) -> bool {
        self.discriminator_annotations
            .iter()
            .any(|a| matches!(a, Annotation::Key))
    }

    /// Adds a case to this union.
    pub fn add_case(&mut self, case: UnionCase) {
        self.cases.push(case);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Union case
pub struct UnionCase {
    pub labels: Vec<UnionLabel>,
    pub field: Field,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Union case label
pub enum UnionLabel {
    Value(String),
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Constant definition
pub struct Const {
    pub name: String,
    pub const_type: IdlType,
    pub value: String,
}

impl Const {
    /// Creates a new constant with the given name, type, and value.
    #[must_use]
    pub fn new(name: impl Into<String>, const_type: IdlType, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            const_type,
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Forward declaration entry
pub struct ForwardDecl {
    pub kind: ForwardKind,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Forward declaration kind
pub enum ForwardKind {
    Struct,
    Union,
}

// ===== Interfaces & Exceptions (feature-gated) =====
#[cfg(feature = "interfaces")]
#[derive(Debug, Clone, PartialEq, Eq)]
/// Interface definition used when the `interfaces` feature is enabled.
pub struct Interface {
    pub name: String,
    pub base: Option<String>,
    pub operations: Vec<Operation>,
    pub attributes: Vec<Attribute>,
}

#[cfg(feature = "interfaces")]
impl Interface {
    /// Creates a new empty interface with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            base: None,
            operations: Vec::new(),
            attributes: Vec::new(),
        }
    }
}

#[cfg(feature = "interfaces")]
#[derive(Debug, Clone, PartialEq, Eq)]
/// Operation defined inside an IDL interface.
pub struct Operation {
    pub oneway: bool,
    pub name: String,
    pub return_type: IdlType,
    pub params: Vec<Parameter>,
    pub raises: Vec<String>, // exception names
}

#[cfg(feature = "interfaces")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Direction qualifier for interface parameters.
pub enum ParamDir {
    In,
    Out,
    InOut,
}

#[cfg(feature = "interfaces")]
#[derive(Debug, Clone, PartialEq, Eq)]
/// Parameter defined in an interface operation.
pub struct Parameter {
    pub dir: ParamDir,
    pub name: String,
    pub ty: IdlType,
}

#[cfg(feature = "interfaces")]
#[derive(Debug, Clone, PartialEq, Eq)]
/// Attribute declaration inside an interface.
pub struct Attribute {
    pub readonly: bool,
    pub name: String,
    pub ty: IdlType,
}

#[cfg(feature = "interfaces")]
#[derive(Debug, Clone, PartialEq, Eq)]
/// Exception type exposed by an interface.
pub struct Exception {
    pub name: String,
    pub members: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Bitset definition (collection of bitfields with widths and optional annotations)
pub struct Bitset {
    pub name: String,
    pub annotations: Vec<Annotation>,
    pub fields: Vec<BitfieldDecl>,
}

impl Bitset {
    /// Creates a new empty bitset with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            annotations: Vec::new(),
            fields: Vec::new(),
        }
    }
    /// Adds a bitfield to this bitset.
    pub fn add_field(&mut self, f: BitfieldDecl) {
        self.fields.push(f);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Bitfield declaration inside a bitset
pub struct BitfieldDecl {
    pub width: u32,
    pub name: String,
    pub annotations: Vec<Annotation>,
}

impl BitfieldDecl {
    /// Creates a new bitfield with the given width and name.
    #[must_use]
    pub fn new(width: u32, name: impl Into<String>) -> Self {
        Self {
            width,
            name: name.into(),
            annotations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Bitmask definition (enumerator-like flags, optionally with @position)
pub struct Bitmask {
    pub name: String,
    pub annotations: Vec<Annotation>,
    pub flags: Vec<BitmaskFlag>,
}

impl Bitmask {
    /// Creates a new empty bitmask with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            annotations: Vec::new(),
            flags: Vec::new(),
        }
    }
    /// Adds a flag to this bitmask.
    pub fn add_flag(&mut self, f: BitmaskFlag) {
        self.flags.push(f);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Bitmask flag entry
pub struct BitmaskFlag {
    pub name: String,
    pub annotations: Vec<Annotation>,
}

impl BitmaskFlag {
    /// Creates a new bitmask flag with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            annotations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Custom annotation declaration
pub struct AnnotationDecl {
    pub name: String,
    pub members: Vec<AnnotationMember>,
}

impl AnnotationDecl {
    /// Creates a new annotation declaration with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            members: Vec::new(),
        }
    }
    /// Adds a member to this annotation declaration.
    pub fn add_member(&mut self, m: AnnotationMember) {
        self.members.push(m);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Member of an annotation declaration
pub struct AnnotationMember {
    pub ty: String, // textual type name (e.g., "int32_t", "string", "boolean")
    pub name: String,
    pub default: Option<String>, // textual default value (already literal without quotes)
}

impl AnnotationMember {
    /// Creates a new annotation member with the given type and name.
    pub fn new(ty: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            ty: ty.into(),
            name: name.into(),
            default: None,
        }
    }
}
