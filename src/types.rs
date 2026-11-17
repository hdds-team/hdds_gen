// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! IDL type system
//!
//! Defines the type system according to OMG IDL 4.2 specification,
//! including primitive types, annotations, and extensibility modes.

#[derive(Debug, Clone, PartialEq, Eq)]
/// IDL type representation
pub enum IdlType {
    /// Primitive types
    Primitive(PrimitiveType),

    /// User-defined type (by name, resolved later)
    Named(String),

    /// Sequence type with optional bound
    Sequence {
        inner: Box<Self>,
        bound: Option<u32>,
    },

    /// Map type (optional bound for bounded maps)
    Map {
        key: Box<Self>,
        value: Box<Self>,
        bound: Option<u32>,
    },

    /// Array type with fixed size
    Array { inner: Box<Self>, size: u32 },

    /// `string<N>` (`wide`: `wstring<N>`). Unbounded strings are
    /// `Primitive(String)` / `Primitive(WString)`; `sequence<char, N>` is a
    /// `Sequence` of `char`, a different type on the wire (DDS-XTypes 1.3
    /// 7.4.3.5.3 rules 3-4 versus 11).
    BoundedString { wide: bool, bound: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Primitive types in IDL
pub enum PrimitiveType {
    Void,
    Boolean,
    Char,
    WChar,
    Octet,
    Short,
    UnsignedShort,
    Long,
    UnsignedLong,
    LongLong,
    UnsignedLongLong,
    Float,
    Double,
    LongDouble,
    String,
    WString,
    // IDL 4.x fixed-width types
    Int8,
    Int16,
    Int32,
    Int64,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    /// Fixed-point decimal: fixed<digits, scale>
    Fixed {
        digits: u32,
        scale: u32,
    },
}

impl PrimitiveType {
    /// Get the C++ equivalent type name
    #[must_use]
    pub const fn to_cpp_name(&self) -> &'static str {
        match self {
            Self::Void => "void",
            Self::Boolean => "bool",
            Self::Char => "char",
            Self::WChar => "wchar_t",
            Self::Octet | Self::UInt8 => "uint8_t",
            Self::Short | Self::Int16 => "int16_t",
            Self::UnsignedShort | Self::UInt16 => "uint16_t",
            Self::Long | Self::Int32 => "int32_t",
            Self::UnsignedLong | Self::UInt32 => "uint32_t",
            Self::LongLong | Self::Int64 => "int64_t",
            Self::UnsignedLongLong | Self::UInt64 => "uint64_t",
            Self::Float => "float",
            Self::Double => "double",
            Self::LongDouble | Self::Fixed { .. } => "long double",
            Self::String => "std::string",
            Self::WString => "std::wstring",
            Self::Int8 => "int8_t",
        }
    }

    /// Get the original IDL type name
    #[must_use]
    pub const fn to_idl_string(&self) -> &'static str {
        match self {
            Self::Void => "void",
            Self::Boolean => "boolean",
            Self::Char => "char",
            Self::WChar => "wchar",
            Self::Octet => "octet",
            Self::Short => "short",
            Self::UnsignedShort => "unsigned short",
            Self::Long => "long",
            Self::UnsignedLong => "unsigned long",
            Self::LongLong => "long long",
            Self::UnsignedLongLong => "unsigned long long",
            Self::Float => "float",
            Self::Double => "double",
            Self::LongDouble => "long double",
            Self::String => "string",
            Self::WString => "wstring",
            Self::Int8 => "int8_t",
            Self::Int16 => "int16_t",
            Self::Int32 => "int32_t",
            Self::Int64 => "int64_t",
            Self::UInt8 => "uint8_t",
            Self::UInt16 => "uint16_t",
            Self::UInt32 => "uint32_t",
            Self::UInt64 => "uint64_t",
            Self::Fixed {
                digits: _,
                scale: _,
            } => {
                // Render as fixed<digits,scale>
                // Note: for bounded strings we reuse sequence<char>; here we keep primitive string
                // formatting to preserve intent in comments
                // This string is used by to_idl_string on IdlType::Primitive
                // and shows original IDL form in comments.
                // Use an owned String via formatting caller side when needed.
                // Here we just return a placeholder (will not be used for exact formatting).
                "fixed" // caller not using parameters directly for IDL source
            }
        }
    }
}

impl IdlType {
    /// Convert type to its IDL string representation
    #[must_use]
    pub fn to_idl_string(&self) -> String {
        match self {
            Self::Primitive(p) => p.to_idl_string().to_string(),
            Self::Named(name) => name.clone(),
            Self::BoundedString { wide, bound } => {
                format!("{}<{bound}>", if *wide { "wstring" } else { "string" })
            }
            Self::Sequence { inner, bound } => bound.as_ref().map_or_else(
                || format!("sequence<{}>", inner.to_idl_string()),
                |b| format!("sequence<{}, {}>", inner.to_idl_string(), b),
            ),
            Self::Map { key, value, bound } => bound.as_ref().map_or_else(
                || format!("map<{}, {}>", key.to_idl_string(), value.to_idl_string()),
                |n| {
                    format!(
                        "map<{}, {}, {}>",
                        key.to_idl_string(),
                        value.to_idl_string(),
                        n
                    )
                },
            ),
            Self::Array { inner, size } => format!("{}[{}]", inner.to_idl_string(), size),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Annotations as defined in IDL 4.2 spec (Section 8.3)
pub enum Annotation {
    /// @key - marks a field as part of the key (Section 8.3.2.1)
    Key,

    /// @id - explicitly sets the member ID
    Id(u32),

    /// @autoid - auto-generate member IDs
    AutoId(AutoIdKind),

    /// @optional - marks a field as optional
    Optional,

    /// @position - explicitly sets field position
    Position(u32),

    /// @default(value) - sets the default value, with its literal kind
    Value(AnnotationParam),

    /// @extensibility - controls type evolution
    Extensibility(ExtensibilityKind),

    /// @final - shorthand for FINAL extensibility
    Final,

    /// @appendable - shorthand for APPENDABLE extensibility
    Appendable,

    /// @mutable - shorthand for MUTABLE extensibility
    Mutable,

    /// @`must_understand` - field must be understood by reader
    MustUnderstand,

    /// @`default_literal` - default value for discriminator
    DefaultLiteral,

    /// @default - marks default case in union
    Default,

    /// @range - specify value range
    Range {
        min: AnnotationParam,
        max: AnnotationParam,
    },

    /// @min - minimum value
    Min(AnnotationParam),

    /// @max - maximum value
    Max(AnnotationParam),

    /// @unit - specify unit of measurement
    Unit(String),

    /// @`bit_bound` - bit bound for integers
    BitBound(u32),

    /// @external - mark as external type
    External,

    /// @nested - mark as nested type
    Nested,

    /// @`data_representation` - select wire representation (e.g., XCDR1, XCDR2, `PLAIN_CDR`, `PLAIN_CDR2`)
    DataRepresentation(String),

    /// @`non_serialized` - mark a member as not serialized
    NonSerialized,

    /// @verbatim - language-specific code injection
    Verbatim {
        language: String,
        placement: String,
        text: String,
    },

    /// @service - mark interface as service
    Service,

    /// @oneway - mark operation as oneway
    Oneway,

    /// @ami - asynchronous method invocation
    Ami,

    /// @topic - marks a struct as a DDS topic type; `@topic(name="X",
    /// platform="Y")` (IDL 4.2 8.3.5.1) names the topic and the platform
    /// it applies to.
    Topic {
        name: Option<String>,
        platform: Option<String>,
    },

    /// Custom/unknown annotation
    Custom {
        name: String,
        params: Vec<AnnotationParam>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Lexical kind of an annotation parameter value, kept so the value can be
/// written back as the same IDL literal (`"x y"` stays a string, `RED` an
/// identifier, `-3` a number).
pub enum AnnotationParamKind {
    /// A (possibly scoped) name: `RED`, `Color::RED`.
    Identifier,
    /// A string literal; `value` holds the unescaped text, without quotes.
    String,
    /// An integer or floating-point literal, sign included.
    Number,
    /// `TRUE` / `FALSE`; `value` is `true` or `false`.
    Bool,
    /// A character literal; `value` holds the character, unescaped.
    Char,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One parameter of a custom annotation: `name = value`, or a positional
/// `value` (empty `name`).
pub struct AnnotationParam {
    /// Parameter name; empty for a positional parameter.
    pub name: String,
    /// Literal value: unquoted and unescaped for a string.
    pub value: String,
    /// Lexical kind of `value`.
    pub kind: AnnotationParamKind,
}

impl std::fmt::Display for AnnotationParam {
    /// The value as read (unquoted for a string).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.value)
    }
}

impl AnnotationParam {
    /// Builds a parameter.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        value: impl Into<String>,
        kind: AnnotationParamKind,
    ) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            kind,
        }
    }

    /// The value written as an IDL literal: a string is quoted and escaped,
    /// anything else is written as is.
    #[must_use]
    pub fn value_to_idl(&self) -> String {
        match self.kind {
            AnnotationParamKind::String => quote_idl_string(&self.value),
            AnnotationParamKind::Char => quote_idl_char(&self.value),
            AnnotationParamKind::Bool => {
                if self.value.eq_ignore_ascii_case("true") {
                    "TRUE".to_string()
                } else {
                    "FALSE".to_string()
                }
            }
            AnnotationParamKind::Identifier | AnnotationParamKind::Number => self.value.clone(),
        }
    }
}

/// `value` (one character) as an IDL character literal, escaped as the
/// lexer reads it back.
#[must_use]
pub fn quote_idl_char(value: &str) -> String {
    let mut out = String::from("'");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// `value` as an IDL string literal: quoted, with `\\`, `\"`, newline,
/// tab and carriage return escaped (the escapes the lexer reads back).
#[must_use]
pub fn quote_idl_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// `AutoId` kinds (Section 8.3.1.2)
pub enum AutoIdKind {
    Sequential,
    Hash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Extensibility kinds (Section 8.3.1.6-8.3.1.9)
pub enum ExtensibilityKind {
    /// FINAL - no new members can be added
    Final,
    /// APPENDABLE - new members can be appended
    Appendable,
    /// MUTABLE - members can be added/removed freely
    Mutable,
}

impl std::fmt::Display for ExtensibilityKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Final => write!(f, "FINAL"),
            Self::Appendable => write!(f, "APPENDABLE"),
            Self::Mutable => write!(f, "MUTABLE"),
        }
    }
}
