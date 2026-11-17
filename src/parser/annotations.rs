// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Annotation parsing for IDL.
//!
//! Parses `@annotation` declarations and `@key`, `@mutable`, etc. usages.

use crate::ast::{AnnotationDecl, AnnotationMember, Definition};
use crate::error::{ErrorKind, ParseError, Result};
use crate::token::{Token, TokenKind};
use crate::types::{
    Annotation, AnnotationParam, AnnotationParamKind, AutoIdKind, ExtensibilityKind,
};

use super::Parser;

impl Parser {
    pub(super) fn try_parse_annotation_declaration(&mut self) -> Result<Option<Definition>> {
        if !self.check(&TokenKind::Annotation) {
            return Ok(None);
        }

        let lookahead = self.current + 1;
        if matches!(
            self.tokens.get(lookahead),
            Some(Token {
                kind: TokenKind::Identifier(name),
                ..
            }) if name == "annotation"
        ) {
            self.advance(); // consume '@'
            return self.parse_annotation_decl().map(Some);
        }

        Ok(None)
    }

    pub(super) fn collect_leading_annotations(&mut self) -> Result<Vec<Annotation>> {
        let mut annotations = Vec::new();
        while self.check(&TokenKind::Annotation) {
            self.advance(); // consume '@'
            annotations.push(self.parse_annotation()?);
        }
        Ok(annotations)
    }

    pub(super) fn attach_definition_annotations(
        mut definition: Definition,
        mut annotations: Vec<Annotation>,
    ) -> Definition {
        if annotations.is_empty() {
            return definition;
        }

        match &mut definition {
            Definition::Struct(data) => data.annotations.append(&mut annotations),
            Definition::Enum(data) => data.annotations.append(&mut annotations),
            Definition::Union(data) => data.annotations.append(&mut annotations),
            Definition::Bitset(data) => data.annotations.append(&mut annotations),
            Definition::Bitmask(data) => data.annotations.append(&mut annotations),
            Definition::Typedef(data) => data.annotations.append(&mut annotations),
            Definition::Module(data) => data.annotations.append(&mut annotations),
            _ => {}
        }

        definition
    }

    pub(super) fn parse_annotation_decl(&mut self) -> Result<Definition> {
        self.expect(
            &TokenKind::Identifier("annotation".to_string()),
            "Expected 'annotation'",
        )?;

        let name = if let TokenKind::Identifier(n) = &self.current_token().kind {
            n.clone()
        } else {
            return Err(ParseError::new(
                ErrorKind::InvalidIdentifier,
                self.current_position(),
                "Expected annotation name",
            ));
        };
        self.advance();
        self.expect(&TokenKind::LeftBrace, "Expected '{' after annotation name")?;

        let mut decl = AnnotationDecl::new(name);
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let ty = self.parse_type()?;
            let ty_str = ty.to_idl_string();
            let member_name = if let TokenKind::Identifier(n) = &self.current_token().kind {
                n.clone()
            } else {
                return Err(ParseError::new(
                    ErrorKind::InvalidIdentifier,
                    self.current_position(),
                    "Expected annotation member name",
                ));
            };
            self.advance();
            let default = if self.check(&TokenKind::Default) {
                self.advance();
                match self.peek() {
                    TokenKind::Identifier(id) => {
                        let val = id.clone();
                        self.advance();
                        Some(val)
                    }
                    TokenKind::IntegerLiteral(n) => {
                        let val = n.to_string();
                        self.advance();
                        Some(val)
                    }
                    TokenKind::StringLiteral(s) => {
                        let val = s.clone();
                        self.advance();
                        Some(val)
                    }
                    _ => {
                        return Err(ParseError::new(
                            ErrorKind::InvalidSyntax,
                            self.current_position(),
                            "Expected annotation member default",
                        ));
                    }
                }
            } else {
                None
            };
            self.expect(
                &TokenKind::Semicolon,
                "Expected ';' after annotation member",
            )?;

            let mut m = AnnotationMember::new(ty_str, member_name);
            m.default = default;
            decl.members.push(m);
        }
        self.expect(
            &TokenKind::RightBrace,
            "Expected '}' after annotation declaration",
        )?;
        self.expect(
            &TokenKind::Semicolon,
            "Expected ';' after annotation declaration",
        )?;
        Ok(Definition::AnnotationDecl(decl))
    }

    /// One annotation parameter value: a (possibly scoped, possibly
    /// absolute) name, a number with an optional sign, a string (adjacent
    /// literals concatenated, C-style) or a boolean, with its lexical kind.
    fn parse_annotation_value(&mut self) -> Result<(String, AnnotationParamKind)> {
        match self.peek().clone() {
            TokenKind::Identifier(_) | TokenKind::DoubleColon => {
                let mut v = String::new();
                if self.check(&TokenKind::DoubleColon) {
                    self.advance();
                    v.push_str("::");
                }
                loop {
                    let TokenKind::Identifier(part) = self.peek().clone() else {
                        return Err(ParseError::new(
                            ErrorKind::InvalidSyntax,
                            self.current_position(),
                            "Expected identifier after '::' in annotation parameter",
                        ));
                    };
                    self.advance();
                    v.push_str(&part);
                    if !self.check(&TokenKind::DoubleColon) {
                        break;
                    }
                    self.advance();
                    v.push_str("::");
                }
                Ok((v, AnnotationParamKind::Identifier))
            }
            TokenKind::IntegerLiteral(n) => {
                self.advance();
                Ok((n.to_string(), AnnotationParamKind::Number))
            }
            TokenKind::FloatLiteral(f) => {
                self.advance();
                Ok((f.to_string(), AnnotationParamKind::Number))
            }
            TokenKind::StringLiteral(first) => {
                let mut v = first;
                self.advance();
                while let TokenKind::StringLiteral(next) = self.peek() {
                    v.push_str(next);
                    self.advance();
                }
                Ok((v, AnnotationParamKind::String))
            }
            TokenKind::BoolLiteral(b) => {
                self.advance();
                Ok((
                    if b { "true" } else { "false" }.to_string(),
                    AnnotationParamKind::Bool,
                ))
            }
            TokenKind::CharLiteral(c) => {
                self.advance();
                Ok((c.to_string(), AnnotationParamKind::Char))
            }
            TokenKind::Minus => {
                self.advance();
                let v = match self.peek() {
                    TokenKind::IntegerLiteral(n) => format!("-{n}"),
                    TokenKind::FloatLiteral(f) => format!("-{f}"),
                    _ => {
                        return Err(ParseError::new(
                            ErrorKind::InvalidSyntax,
                            self.current_position(),
                            "Expected numeric value after '-'",
                        ));
                    }
                };
                self.advance();
                Ok((v, AnnotationParamKind::Number))
            }
            _ => Err(ParseError::new(
                ErrorKind::InvalidSyntax,
                self.current_position(),
                "Expected parameter value",
            )),
        }
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn parse_annotation(&mut self) -> Result<Annotation> {
        let name = match &self.current_token().kind {
            TokenKind::Identifier(name) => name.clone(),
            TokenKind::Default => "default".to_string(),
            _ => {
                return Err(ParseError::new(
                    ErrorKind::UnknownAnnotation,
                    self.current_position(),
                    "Expected annotation name",
                ));
            }
        };
        self.advance();

        let params = if self.check(&TokenKind::LeftParen) {
            self.advance();
            let mut params = Vec::new();

            while !self.check(&TokenKind::RightParen) && !self.is_at_end() {
                let (first, first_kind) = self.parse_annotation_value()?;
                let param = if first_kind == AnnotationParamKind::Identifier
                    && !first.contains("::")
                    && self.check(&TokenKind::Equal)
                {
                    self.advance();
                    let (value, kind) = self.parse_annotation_value()?;
                    AnnotationParam::new(first, value, kind)
                } else {
                    AnnotationParam::new(String::new(), first, first_kind)
                };
                params.push(param);

                if self.check(&TokenKind::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }

            self.expect(
                &TokenKind::RightParen,
                "Expected ')' after annotation params",
            )?;
            params
        } else {
            Vec::new()
        };

        let annotation = match name.as_str() {
            "key" => Annotation::Key,
            "optional" => Annotation::Optional,
            "nested" => Annotation::Nested,
            "must_understand" => Annotation::MustUnderstand,
            "external" => Annotation::External,
            "default" => {
                // @default without params marks union default case
                // @default(value) sets field default value
                params
                    .first()
                    .map_or(Annotation::Default, |p| Annotation::Value(p.clone()))
            }
            "default_literal" | "default_init" => Annotation::DefaultLiteral,
            "oneway" => Annotation::Oneway,
            "service" => Annotation::Service,
            "ami" => Annotation::Ami,
            "autoid" => {
                if let Some(AnnotationParam { value: val, .. }) = params.first() {
                    match val.to_ascii_uppercase().as_str() {
                        "SEQUENTIAL" => Annotation::AutoId(AutoIdKind::Sequential),
                        "HASH" => Annotation::AutoId(AutoIdKind::Hash),
                        _ => Annotation::Custom {
                            name: name.clone(),
                            params,
                        },
                    }
                } else {
                    Annotation::AutoId(AutoIdKind::Hash)
                }
            }
            "id" => {
                if let Some(AnnotationParam { value: val, .. }) = params.first() {
                    val.parse::<u32>().map_or_else(
                        |_| Annotation::Custom {
                            name: name.clone(),
                            params,
                        },
                        Annotation::Id,
                    )
                } else {
                    Annotation::Custom {
                        name: name.clone(),
                        params,
                    }
                }
            }
            "position" => {
                if let Some(AnnotationParam { value: val, .. }) = params.first() {
                    val.parse::<u32>().map_or_else(
                        |_| Annotation::Custom {
                            name: name.clone(),
                            params,
                        },
                        Annotation::Position,
                    )
                } else {
                    Annotation::Custom {
                        name: name.clone(),
                        params,
                    }
                }
            }
            "bit_bound" => {
                if let Some(AnnotationParam { value: val, .. }) = params.first() {
                    val.parse::<u32>().map_or_else(
                        |_| Annotation::Custom {
                            name: name.clone(),
                            params,
                        },
                        Annotation::BitBound,
                    )
                } else {
                    Annotation::Custom {
                        name: name.clone(),
                        params,
                    }
                }
            }
            "unit" => {
                if let Some(AnnotationParam { value: val, .. }) = params.first() {
                    Annotation::Unit(val.clone())
                } else {
                    Annotation::Custom {
                        name: name.clone(),
                        params,
                    }
                }
            }
            "min" => params.first().cloned().map_or_else(
                || Annotation::Custom {
                    name: name.clone(),
                    params,
                },
                Annotation::Min,
            ),
            "max" => params.first().cloned().map_or_else(
                || Annotation::Custom {
                    name: name.clone(),
                    params,
                },
                Annotation::Max,
            ),
            "range" => {
                let find = |key: &str| params.iter().find(|p| p.name == key).cloned();
                if let (Some(min), Some(max)) = (find("min"), find("max")) {
                    Annotation::Range { min, max }
                } else {
                    Annotation::Custom {
                        name: name.clone(),
                        params,
                    }
                }
            }
            "data_representation" => {
                if let Some(AnnotationParam { value: val, .. }) = params.first() {
                    Annotation::DataRepresentation(val.clone())
                } else {
                    Annotation::Custom {
                        name: name.clone(),
                        params,
                    }
                }
            }
            "extensibility" => {
                if let Some(AnnotationParam { value: val, .. }) = params.first() {
                    match val.to_ascii_uppercase().as_str() {
                        "FINAL" => Annotation::Extensibility(ExtensibilityKind::Final),
                        "APPENDABLE" => Annotation::Extensibility(ExtensibilityKind::Appendable),
                        "MUTABLE" => Annotation::Extensibility(ExtensibilityKind::Mutable),
                        _ => Annotation::Custom {
                            name: name.clone(),
                            params,
                        },
                    }
                } else {
                    Annotation::Custom {
                        name: name.clone(),
                        params,
                    }
                }
            }
            "final" => Annotation::Final,
            "appendable" => Annotation::Appendable,
            "mutable" => Annotation::Mutable,
            "non_serialized" => Annotation::NonSerialized,
            // `@topic`, `@topic("X")`, `@topic(name="X", platform="Y")`; any
            // other parameter keeps the whole annotation as written (custom).
            "topic" => {
                let mut topic_name = None;
                let mut platform = None;
                let mut other = false;
                for p in &params {
                    match p.name.as_str() {
                        "name" | "" if topic_name.is_none() => topic_name = Some(p.value.clone()),
                        "platform" if platform.is_none() => platform = Some(p.value.clone()),
                        _ => other = true,
                    }
                }
                if other {
                    Annotation::Custom { name, params }
                } else {
                    Annotation::Topic {
                        name: topic_name,
                        platform,
                    }
                }
            }
            "verbatim" => {
                let mut language = String::new();
                let mut placement = String::new();
                let mut text = String::new();
                for AnnotationParam {
                    name: k, value: v, ..
                } in &params
                {
                    match k.as_str() {
                        "language" | "" if language.is_empty() => v.clone_into(&mut language),
                        "placement" => v.clone_into(&mut placement),
                        "text" => v.clone_into(&mut text),
                        _ => {}
                    }
                }
                Annotation::Verbatim {
                    language,
                    placement,
                    text,
                }
            }
            "shared" | "bounded" => Annotation::Custom {
                name: name.clone(),
                params,
            },
            _ => Annotation::Custom { name, params },
        };

        Ok(annotation)
    }
}
