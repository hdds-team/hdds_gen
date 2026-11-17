// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Union definition parsing.
//!
//! Parses IDL discriminated union declarations with case labels.

use crate::ast::{Union, UnionCase, UnionLabel};
use crate::error::{ErrorKind, ParseError, Result};
use crate::token::TokenKind;
use crate::types::{Annotation, IdlType};

use super::Parser;
use crate::parser::const_expr::ConstValue;

impl Parser {
    pub(super) fn parse_union(&mut self) -> Result<Union> {
        self.expect(&TokenKind::Union, "Expected 'union'")?;

        let name = self.parse_union_name()?;
        let (discriminator_annotations, discriminator) = self.parse_union_discriminator()?;
        self.expect(
            &TokenKind::LeftBrace,
            "Expected '{' after union discriminator",
        )?;

        let mut union_def = Union::new(name, discriminator);
        union_def.discriminator_annotations = discriminator_annotations;

        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let case = self.parse_union_case()?;
            union_def.add_case(case);
        }

        self.expect(&TokenKind::RightBrace, "Expected '}' after union body")?;
        self.expect(&TokenKind::Semicolon, "Expected ';' after union")?;

        Ok(union_def)
    }

    fn parse_union_name(&mut self) -> Result<String> {
        if let TokenKind::Identifier(name) = &self.current_token().kind {
            let result = name.clone();
            self.advance();
            Ok(result)
        } else {
            Err(ParseError::new(
                ErrorKind::InvalidIdentifier,
                self.current_position(),
                "Expected union name",
            ))
        }
    }

    /// `switch ( [annotations] type )`; the annotations (`@key`) apply to
    /// the discriminator.
    fn parse_union_discriminator(&mut self) -> Result<(Vec<Annotation>, IdlType)> {
        self.expect(&TokenKind::Switch, "Expected 'switch' after union name")?;
        self.expect(&TokenKind::LeftParen, "Expected '(' after 'switch'")?;

        let mut annotations = Vec::new();
        while self.check(&TokenKind::Annotation) {
            self.advance();
            annotations.push(self.parse_annotation()?);
        }
        let discriminator = self.parse_type()?;

        self.expect(
            &TokenKind::RightParen,
            "Expected ')' after discriminator type",
        )?;

        Ok((annotations, discriminator))
    }

    fn parse_union_case(&mut self) -> Result<UnionCase> {
        let saw_default_annotation = self.consume_union_case_annotations()?;
        let mut labels = self.parse_union_case_labels()?;

        if labels.is_empty() && saw_default_annotation {
            labels.push(UnionLabel::Default);
        }

        if labels.is_empty() {
            return Err(ParseError::new(
                ErrorKind::InvalidSyntax,
                self.current_position(),
                "Expected case or default label",
            ));
        }

        let field = self.parse_field()?;
        Ok(UnionCase { labels, field })
    }

    fn consume_union_case_annotations(&mut self) -> Result<bool> {
        let mut saw_default = false;
        while self.check(&TokenKind::Annotation) {
            self.advance();
            let annotation = self.parse_annotation()?;
            if matches!(annotation, Annotation::Default | Annotation::DefaultLiteral) {
                saw_default = true;
            }
        }
        Ok(saw_default)
    }

    fn parse_union_case_labels(&mut self) -> Result<Vec<UnionLabel>> {
        let mut labels = Vec::new();
        loop {
            if self.check(&TokenKind::Case) {
                self.advance();
                let value = self.parse_union_case_value()?;
                labels.push(UnionLabel::Value(value));
            } else if self.check(&TokenKind::Default) {
                self.advance();
                self.expect(&TokenKind::Colon, "Expected ':' after 'default'")?;
                labels.push(UnionLabel::Default);
                break;
            } else {
                break;
            }
        }
        Ok(labels)
    }

    /// A case label: an integer (possibly negative), TRUE / FALSE, a
    /// character literal, or an identifier (possibly scoped, `m::A`).
    /// Normalized to the text `data_model::union_label_value` reads.
    fn parse_union_case_value(&mut self) -> Result<String> {
        // A label naming a constant (`case N:`, IDL 4.2 7.4.1.4.4.4: a
        // `const_expr`) is written as the constant's value: what the
        // backends read is a literal or an enumerator.
        if matches!(
            self.peek(),
            TokenKind::Identifier(_) | TokenKind::DoubleColon
        ) {
            let start = self.current;
            let pos = self.current_position();
            let name = self.parse_scoped_name("case label")?;
            if let Ok(value) = self.lookup_const(&name, pos) {
                let text = match value {
                    ConstValue::Int(n) => n.to_string(),
                    ConstValue::Bool(b) => if b { "TRUE" } else { "FALSE" }.to_string(),
                    ConstValue::Char(c) => format!("'{c}'"),
                    ConstValue::Float(_) | ConstValue::Str(_) => {
                        return Err(ParseError::new(
                            ErrorKind::InvalidSyntax,
                            pos,
                            format!("Case label constant '{name}' is not an integer, a boolean or a character"),
                        ));
                    }
                };
                self.expect(&TokenKind::Colon, "Expected ':' after case label")?;
                return Ok(text);
            }
            self.current = start;
        }
        let value = match self.peek().clone() {
            TokenKind::Identifier(id) => {
                self.advance();
                let mut label = id;
                while self.check(&TokenKind::DoubleColon) {
                    self.advance();
                    match self.peek().clone() {
                        TokenKind::Identifier(next) => {
                            self.advance();
                            label.push_str("::");
                            label.push_str(&next);
                        }
                        _ => {
                            return Err(ParseError::new(
                                ErrorKind::InvalidSyntax,
                                self.current_position(),
                                "Expected identifier after '::' in case label",
                            ));
                        }
                    }
                }
                label
            }
            TokenKind::DoubleColon => {
                // `::m::A`: an absolute scoped enumerator, kept absolute.
                self.advance();
                let value = self.parse_union_case_value()?;
                return Ok(format!("::{value}"));
            }
            TokenKind::IntegerLiteral(n) => {
                self.advance();
                n.to_string()
            }
            TokenKind::Minus => {
                self.advance();
                match self.peek().clone() {
                    TokenKind::IntegerLiteral(n) => {
                        self.advance();
                        (-n).to_string()
                    }
                    _ => {
                        return Err(ParseError::new(
                            ErrorKind::InvalidSyntax,
                            self.current_position(),
                            "Expected integer after '-' in case label",
                        ));
                    }
                }
            }
            TokenKind::BoolLiteral(b) => {
                self.advance();
                if b { "TRUE" } else { "FALSE" }.to_string()
            }
            TokenKind::CharLiteral(c) => {
                self.advance();
                format!("'{c}'")
            }
            _ => {
                return Err(ParseError::new(
                    ErrorKind::InvalidSyntax,
                    self.current_position(),
                    "Expected case label value",
                ));
            }
        };

        self.expect(&TokenKind::Colon, "Expected ':' after case label")?;
        Ok(value)
    }
}
