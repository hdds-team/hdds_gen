// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Parser state machine and main parsing logic.
//!
//! The `Parser` struct consumes a token stream and produces an AST.

use crate::ast::{Definition, IdlFile};
use crate::error::{ErrorKind, ParseError, Position, Result};
use crate::lexer::Lexer;
use crate::token::{Token, TokenKind};
use std::collections::HashMap;
use std::convert::TryFrom;

use super::const_expr::ConstValue;
use super::definitions::merge_module_into;

/// Parser state machine that consumes a token stream and produces an AST.
pub struct Parser {
    pub(super) tokens: Vec<Token>,
    pub(super) current: usize,
    pub(super) const_env: HashMap<String, ConstValue>,
}

impl Parser {
    /// Create a new parser from IDL source code.
    ///
    /// Lexer errors are silently swallowed.  Prefer [`try_new`](Self::try_new)
    /// which propagates them.
    #[must_use]
    #[deprecated(since = "1.1.0", note = "use try_new() which propagates lexer errors")]
    pub fn new(input: &str) -> Self {
        let mut lexer = Lexer::new(input);
        let mut tokens = lexer.tokenize().unwrap_or_else(|_| vec![]);

        if tokens.is_empty() {
            tokens.push(Token::new(TokenKind::Eof, "", Position::new(1, 1)));
        }

        Self {
            tokens,
            current: 0,
            const_env: HashMap::default(),
        }
    }

    /// Create a new parser from IDL source code, propagating lexer errors.
    ///
    /// # Errors
    ///
    /// Returns an error when the input contains invalid lexical tokens.
    pub fn try_new(input: &str) -> Result<Self> {
        let mut lexer = Lexer::new(input);
        let mut tokens = lexer.tokenize()?;

        if tokens.is_empty() {
            tokens.push(Token::new(TokenKind::Eof, "", Position::new(1, 1)));
        }

        Ok(Self {
            tokens,
            current: 0,
            const_env: HashMap::default(),
        })
    }

    /// Create a parser from an existing token stream.
    #[must_use]
    pub fn from_tokens(mut tokens: Vec<Token>) -> Self {
        let needs_eof = tokens
            .last()
            .is_none_or(|token| !matches!(token.kind, TokenKind::Eof));
        if needs_eof {
            let pos = tokens
                .last()
                .map_or_else(|| Position::new(1, 1), |token| token.position);
            tokens.push(Token::new(TokenKind::Eof, "", pos));
        }
        Self {
            tokens,
            current: 0,
            const_env: HashMap::default(),
        }
    }

    #[must_use]
    pub(super) fn current_token(&self) -> &Token {
        let last_index = self.tokens.len().saturating_sub(1);
        &self.tokens[self.current.min(last_index)]
    }

    #[must_use]
    pub(super) fn current_position(&self) -> Position {
        self.current_token().position
    }

    #[must_use]
    pub(super) fn is_at_end(&self) -> bool {
        matches!(self.current_token().kind, TokenKind::Eof)
    }

    pub(super) fn advance(&mut self) -> &Token {
        if !self.is_at_end() {
            self.current += 1;
        }
        &self.tokens[self.current - 1]
    }

    #[must_use]
    pub(super) fn peek(&self) -> &TokenKind {
        &self.current_token().kind
    }

    #[must_use]
    pub(super) fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(self.peek()) == std::mem::discriminant(kind)
    }

    /// Parse a bound value: either an integer literal or a const identifier.
    pub(super) fn parse_bound_u32(&mut self, context: &str) -> Result<u32> {
        let pos = self.current_position();
        match self.peek() {
            TokenKind::IntegerLiteral(n) => {
                let raw = *n;
                self.advance();
                Self::literal_u32(raw, pos, context)
            }
            TokenKind::Identifier(name) => {
                let name = name.clone();
                self.advance();
                let val = self.const_env.get(&name).ok_or_else(|| {
                    ParseError::new(
                        ErrorKind::InvalidSyntax,
                        pos,
                        format!("Unknown constant '{name}' used as {context}"),
                    )
                })?;
                let int_val = val.as_int().map_err(|_| {
                    ParseError::new(
                        ErrorKind::InvalidSyntax,
                        pos,
                        format!("Constant '{name}' is not an integer"),
                    )
                })?;
                Self::literal_u32(int_val, pos, context)
            }
            _ => Err(ParseError::new(
                ErrorKind::InvalidSyntax,
                pos,
                format!("Expected integer or constant for {context}"),
            )),
        }
    }

    pub(super) fn literal_u32(value: i64, position: Position, context: &str) -> Result<u32> {
        if value < 0 {
            return Err(ParseError::new(
                ErrorKind::InvalidSyntax,
                position,
                format!("{context} must be non-negative (got {value})"),
            ));
        }
        u32::try_from(value).map_err(|_| {
            ParseError::new(
                ErrorKind::InvalidSyntax,
                position,
                format!("{context} exceeds u32::MAX (got {value})"),
            )
        })
    }

    pub(super) fn expect(&mut self, kind: &TokenKind, message: &str) -> Result<Token> {
        if self.check(kind) {
            Ok(self.advance().clone())
        } else {
            Err(ParseError::new(
                ErrorKind::UnexpectedToken,
                self.current_position(),
                format!("{message}: expected {kind:?}, found {:?}", self.peek()),
            ))
        }
    }

    /// Special version of expect for closing angle brackets in templates/sequences/maps.
    /// Handles the case where `>>` is lexed as a single `ShiftRight` token instead of two `RightAngle` tokens.
    /// This is a classic issue in C++ template parsing (pre-C++11).
    ///
    /// When we encounter `>>`, we:
    /// 1. Consume it as the first `>`
    /// 2. Insert a synthetic `>` token back into the token stream for the next parsing operation
    pub(super) fn expect_angle_close(&mut self, message: &str) -> Result<Token> {
        match self.peek() {
            TokenKind::RightAngle => Ok(self.advance().clone()),
            TokenKind::ShiftRight => {
                let token = self.advance().clone();

                // Insert a synthetic RightAngle token at the current position
                // so the next parse operation will see it
                let right_angle_token = Token::new(TokenKind::RightAngle, ">", token.position);
                self.tokens.insert(self.current, right_angle_token);

                // Decrement current because we're about to return to a loop that will
                // advance the position. By inserting at self.current, the next peek()
                // will return our synthetic token.
                // Actually, we just need to return success - the synthetic token is at current position

                // Return a synthetic RightAngle token
                Ok(Token::new(TokenKind::RightAngle, ">", token.position))
            }
            _ => Err(ParseError::new(
                ErrorKind::UnexpectedToken,
                self.current_position(),
                format!("{message}: expected '>', found {:?}", self.peek()),
            )),
        }
    }

    /// Parse the entire IDL file into an [`IdlFile`].
    ///
    /// # Errors
    ///
    /// Returns an error when the token stream describes an invalid IDL construct.
    pub fn parse(&mut self) -> Result<IdlFile> {
        let mut file = IdlFile::new();

        while !self.is_at_end() {
            if matches!(
                self.peek(),
                TokenKind::PreprocessorDefine
                    | TokenKind::PreprocessorInclude
                    | TokenKind::PreprocessorIfdef
                    | TokenKind::PreprocessorIfndef
                    | TokenKind::PreprocessorElse
                    | TokenKind::PreprocessorElif
                    | TokenKind::PreprocessorEndif
                    | TokenKind::PreprocessorUndef
                    | TokenKind::PreprocessorPragma
            ) {
                self.advance();
                while !self.is_at_end() {
                    match self.peek() {
                        TokenKind::Module
                        | TokenKind::Struct
                        | TokenKind::Enum
                        | TokenKind::Union
                        | TokenKind::Typedef
                        | TokenKind::Const
                        | TokenKind::Annotation => break,
                        _ => {
                            self.advance();
                        }
                    }
                }
                continue;
            }

            let def = self.parse_definition()?;
            match def {
                Definition::Module(m) => merge_module_into(&mut file.definitions, m),
                other => file.add_definition(other),
            }
        }

        Ok(file)
    }
}
