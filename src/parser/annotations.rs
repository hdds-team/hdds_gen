// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Annotation parsing for IDL.
//!
//! Parses `@annotation` declarations and `@key`, `@mutable`, etc. usages.

use crate::ast::{AnnotationDecl, AnnotationMember, Definition};
use crate::error::{ErrorKind, ParseError, Result};
use crate::token::{Token, TokenKind};
use crate::types::{Annotation, AutoIdKind, ExtensibilityKind};

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
                let (key, value) = match self.peek() {
                    TokenKind::Identifier(id) => {
                        let k = id.clone();
                        self.advance();

                        if self.check(&TokenKind::Equal) {
                            self.advance();
                            let v = match self.peek() {
                                TokenKind::Identifier(val) => {
                                    let v = val.clone();
                                    self.advance();
                                    v
                                }
                                TokenKind::IntegerLiteral(n) => {
                                    let v = n.to_string();
                                    self.advance();
                                    v
                                }
                                TokenKind::FloatLiteral(f) => {
                                    let v = f.to_string();
                                    self.advance();
                                    v
                                }
                                TokenKind::StringLiteral(s) => {
                                    // Handle C-style string concatenation: "str1" "str2" -> "str1str2"
                                    let mut v = s.clone();
                                    self.advance();
                                    while let TokenKind::StringLiteral(next) = self.peek() {
                                        v.push_str(next);
                                        self.advance();
                                    }
                                    v
                                }
                                TokenKind::BoolLiteral(b) => {
                                    let v = if *b { "true" } else { "false" }.to_string();
                                    self.advance();
                                    v
                                }
                                TokenKind::Minus => {
                                    self.advance();
                                    match self.peek() {
                                        TokenKind::IntegerLiteral(n) => {
                                            let v = format!("-{n}");
                                            self.advance();
                                            v
                                        }
                                        TokenKind::FloatLiteral(f) => {
                                            let v = format!("-{f}");
                                            self.advance();
                                            v
                                        }
                                        _ => {
                                            return Err(ParseError::new(
                                                ErrorKind::InvalidSyntax,
                                                self.current_position(),
                                                "Expected numeric value after '-'",
                                            ));
                                        }
                                    }
                                }
                                _ => {
                                    return Err(ParseError::new(
                                        ErrorKind::InvalidSyntax,
                                        self.current_position(),
                                        "Expected parameter value",
                                    ));
                                }
                            };
                            (k, v)
                        } else {
                            (String::new(), k)
                        }
                    }
                    TokenKind::IntegerLiteral(n) => {
                        let v = n.to_string();
                        self.advance();
                        (String::new(), v)
                    }
                    TokenKind::FloatLiteral(f) => {
                        let v = f.to_string();
                        self.advance();
                        (String::new(), v)
                    }
                    TokenKind::StringLiteral(s) => {
                        // Handle C-style string concatenation: "str1" "str2" -> "str1str2"
                        let mut v = s.clone();
                        self.advance();
                        while let TokenKind::StringLiteral(next) = self.peek() {
                            v.push_str(next);
                            self.advance();
                        }
                        (String::new(), v)
                    }
                    TokenKind::BoolLiteral(b) => {
                        let v = if *b { "true" } else { "false" }.to_string();
                        self.advance();
                        (String::new(), v)
                    }
                    _ => {
                        return Err(ParseError::new(
                            ErrorKind::InvalidSyntax,
                            self.current_position(),
                            "Expected parameter",
                        ));
                    }
                };

                params.push((key, value));

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
                if let Some((_, val)) = params.first() {
                    Annotation::Value(val.clone())
                } else {
                    Annotation::Default
                }
            }
            "default_literal" | "default_init" => Annotation::DefaultLiteral,
            "oneway" => Annotation::Oneway,
            "service" => Annotation::Service,
            "ami" => Annotation::Ami,
            "autoid" => {
                if let Some((_, val)) = params.first() {
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
                if let Some((_, val)) = params.first() {
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
                if let Some((_, val)) = params.first() {
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
                if let Some((_, val)) = params.first() {
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
                if let Some((_, val)) = params.first() {
                    Annotation::Unit(val.clone())
                } else {
                    Annotation::Custom {
                        name: name.clone(),
                        params,
                    }
                }
            }
            "min" => {
                if let Some((_, val)) = params.first() {
                    Annotation::Min(val.clone())
                } else {
                    Annotation::Custom {
                        name: name.clone(),
                        params,
                    }
                }
            }
            "max" => {
                if let Some((_, val)) = params.first() {
                    Annotation::Max(val.clone())
                } else {
                    Annotation::Custom {
                        name: name.clone(),
                        params,
                    }
                }
            }
            "range" => {
                let mut min_val: Option<String> = None;
                let mut max_val: Option<String> = None;
                for (k, v) in &params {
                    match k.as_str() {
                        "min" => min_val = Some(v.clone()),
                        "max" => max_val = Some(v.clone()),
                        _ => {}
                    }
                }
                if let (Some(min), Some(max)) = (min_val, max_val) {
                    Annotation::Range { min, max }
                } else {
                    Annotation::Custom {
                        name: name.clone(),
                        params,
                    }
                }
            }
            "data_representation" => {
                if let Some((_, val)) = params.first() {
                    Annotation::DataRepresentation(val.clone())
                } else {
                    Annotation::Custom {
                        name: name.clone(),
                        params,
                    }
                }
            }
            "extensibility" => {
                if let Some((_, val)) = params.first() {
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
            "topic" => Annotation::Topic,
            "verbatim" => {
                let mut language = String::new();
                let mut placement = String::new();
                let mut text = String::new();
                for (k, v) in &params {
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
