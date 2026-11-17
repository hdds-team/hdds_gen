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

/// Parser state machine that consumes a token stream and produces an AST.
pub struct Parser {
    pub(super) tokens: Vec<Token>,
    pub(super) current: usize,
    /// Constants by global name (`m::MAX`).
    pub(super) const_env: HashMap<String, ConstValue>,
    /// Every global name declared so far, with the sequence number of its
    /// first declaration (modules, types, constants, enumerators).
    pub(super) decl_seq: HashMap<String, usize>,
    /// The sequence number of the definition (struct, union, typedef) of
    /// each global name: what it may reference was declared before it.
    pub(super) def_seq: HashMap<String, usize>,
    /// Next declaration sequence number.
    pub(super) seq: usize,
    /// Module scope stack for FQN resolution (e.g., `["my_mod", "inner"]`)
    pub(super) scope_stack: Vec<String>,
    /// The lexer error [`Parser::new`] met, returned by [`Parser::parse`]
    /// instead of an empty AST.
    lex_error: Option<ParseError>,
}

impl Parser {
    /// Create a new parser from IDL source code.
    ///
    /// A lexer error is kept and returned by [`parse`](Self::parse), so an
    /// invalid input never parses to an empty AST. Prefer
    /// [`try_new`](Self::try_new), which returns it at once.
    #[must_use]
    #[deprecated(since = "1.1.0", note = "use try_new() which propagates lexer errors")]
    pub fn new(input: &str) -> Self {
        match Self::try_new(input) {
            Ok(parser) => parser,
            Err(e) => {
                let mut parser = Self::from_tokens(Vec::new());
                parser.lex_error = Some(e);
                parser
            }
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
            scope_stack: Vec::new(),
            lex_error: None,
            decl_seq: HashMap::default(),
            def_seq: HashMap::default(),
            seq: 0,
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
            scope_stack: Vec::new(),
            lex_error: None,
            decl_seq: HashMap::default(),
            def_seq: HashMap::default(),
            seq: 0,
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

    /// Parse a bound value: either an integer literal or a (possibly
    /// scoped or absolute) constant name.
    pub(super) fn parse_bound_u32(&mut self, context: &str) -> Result<u32> {
        let pos = self.current_position();
        match self.peek() {
            TokenKind::IntegerLiteral(n) => {
                let raw = *n;
                self.advance();
                Self::literal_u32(raw, pos, context)
            }
            TokenKind::Identifier(_) | TokenKind::DoubleColon => {
                let name = self.parse_scoped_name(context)?;
                let val = self.lookup_const(&name, pos)?;
                let int_val = val.as_int(pos).map_err(|_| {
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

    /// A scoped name, `a`, `a::b` or `::a::b`, as written.
    pub(super) fn parse_scoped_name(&mut self, context: &str) -> Result<String> {
        let mut name = String::new();
        if self.check(&TokenKind::DoubleColon) {
            self.advance();
            name.push_str("::");
        }
        loop {
            let TokenKind::Identifier(part) = self.peek().clone() else {
                return Err(ParseError::new(
                    ErrorKind::InvalidSyntax,
                    self.current_position(),
                    format!("Expected identifier in {context}"),
                ));
            };
            self.advance();
            name.push_str(&part);
            if !self.check(&TokenKind::DoubleColon) {
                return Ok(name);
            }
            self.advance();
            name.push_str("::");
        }
    }

    /// The value of the constant `name` names from the current scope, by
    /// the IDL 4.2 scoping rules ([`crate::scope`]) over the names declared
    /// so far.
    pub(super) fn lookup_const(&self, name: &str, pos: Position) -> Result<ConstValue> {
        let global =
            crate::scope::resolve(&self.scope_stack, name, |q| self.decl_seq.contains_key(q));
        global
            .and_then(|g| self.const_env.get(&g))
            .cloned()
            .ok_or_else(|| {
                ParseError::new(
                    ErrorKind::InvalidSyntax,
                    pos,
                    format!("Unknown constant '{name}'"),
                )
            })
    }

    /// The next declaration sequence number.
    pub(super) const fn next_seq(&mut self) -> usize {
        self.seq += 1;
        self.seq
    }

    /// Record that `global` is declared (first declaration wins).
    pub(super) fn declare(&mut self, global: String, seq: usize) {
        self.decl_seq.entry(global).or_insert(seq);
    }

    /// A reference to a name declared only AFTER the definition holding it
    /// (in a nearer scope than the declaration visible at that point) is
    /// rewritten as the absolute name of the declaration that was visible
    /// (IDL 4.2 7.5: a name is resolved where it is used; Cyclone DDS
    /// reads it the same way), so that every later lookup, which sees the
    /// whole file, still finds it.
    fn pin_declaration_order(&self, file: &mut IdlFile) {
        fn walk(p: &Parser, defs: &mut [Definition], scope: &mut Vec<String>) {
            for def in defs {
                if let Definition::Module(m) = def {
                    scope.push(m.name.clone());
                    walk(p, &mut m.definitions, scope);
                    scope.pop();
                    continue;
                }
                let Some((name, _)) = crate::scope::declared_name(def) else {
                    continue;
                };
                let Some(&at) = p.def_seq.get(&crate::scope::join(scope, name)) else {
                    continue;
                };
                // A structure is in scope inside its own body (a member
                // `sequence<Node>`); a typedef's name is not in scope in its
                // own type: `typedef T T;` inside `A` names the outer `T`.
                let own_visible = !matches!(def, Definition::Typedef(_));
                crate::scope::for_each_reference_mut(def, &mut |n: &mut String| {
                    if n.starts_with("::") {
                        return;
                    }
                    let then = crate::scope::resolve(scope, n, |q| {
                        p.decl_seq
                            .get(q)
                            .is_some_and(|&d| d < at || (own_visible && d == at))
                    });
                    let now = crate::scope::resolve(scope, n, |q| p.decl_seq.contains_key(q));
                    if let Some(then) = then {
                        if now.as_ref() != Some(&then) {
                            *n = format!("::{then}");
                        }
                    }
                });
            }
        }
        walk(self, &mut file.definitions, &mut Vec::new());
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
    /// Returns an error when the token stream describes an invalid IDL
    /// construct, or the lexer error of a parser built by [`Parser::new`].
    pub fn parse(&mut self) -> Result<IdlFile> {
        if let Some(e) = &self.lex_error {
            return Err(e.clone());
        }
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

            for def in self.parse_definition()? {
                match def {
                    // A reopened module stays where it is reopened (IDL 4.2
                    // 7.5.2): its definitions keep their declaration order.
                    Definition::Module(m) => file.add_definition(Definition::Module(m)),
                    other => file.add_definition(other),
                }
            }
        }

        self.pin_declaration_order(&mut file);
        Ok(file)
    }
}
