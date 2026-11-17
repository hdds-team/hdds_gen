// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Token scanning and keyword recognition.
//!
//! Scans identifiers, keywords, strings, and punctuation.

use super::state::Lexer;
use crate::error::{ErrorKind, ParseError, Position, Result};
use crate::token::{Token, TokenKind};

impl Lexer {
    fn read_identifier(&mut self) -> Result<Token> {
        let start_pos = self.current_position();
        let mut lexeme = String::new();

        if let Some(ch) = self.peek() {
            if ch.is_ascii_alphabetic() || ch == '_' {
                lexeme.push(ch);
                self.advance();
            }
        }

        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                lexeme.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        // An escaped identifier (IDL 4.2 7.2.3.2): a leading underscore makes
        // the rest an identifier, never a keyword, and is not part of it
        // (`_map` is the identifier `map`). What follows it must itself be
        // an identifier, which begins with a letter (7.2.3).
        if let Some(name) = lexeme.strip_prefix('_') {
            if !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
                return Err(ParseError::new(
                    ErrorKind::InvalidSyntax,
                    start_pos,
                    format!("Invalid identifier '{lexeme}': an identifier begins with a letter, after an optional escaping '_'"),
                ));
            }
            let name = name.to_string();
            return Ok(Token::new(TokenKind::Identifier(name), lexeme, start_pos));
        }

        let kind = TokenKind::from_keyword(&lexeme).unwrap_or_else(|| match lexeme.as_str() {
            "true" | "TRUE" => TokenKind::BoolLiteral(true),
            "false" | "FALSE" => TokenKind::BoolLiteral(false),
            _ => TokenKind::Identifier(lexeme.clone()),
        });

        Ok(Token::new(kind, lexeme, start_pos))
    }

    fn read_string(&mut self) -> Result<Token> {
        let start_pos = self.current_position();
        let mut lexeme = String::new();
        let mut value = String::new();

        lexeme.push('"');
        self.advance();

        while let Some(ch) = self.peek() {
            if ch == '"' {
                lexeme.push(ch);
                self.advance();
                return Ok(Token::new(
                    TokenKind::StringLiteral(value),
                    lexeme,
                    start_pos,
                ));
            } else if ch == '\\' {
                lexeme.push(ch);
                self.advance();
                if let Some(escaped) = self.peek() {
                    lexeme.push(escaped);
                    value.push(match escaped {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '\\' => '\\',
                        '"' => '"',
                        _ => escaped,
                    });
                    self.advance();
                }
            } else if ch == '\n' {
                return Err(ParseError::new(
                    ErrorKind::InvalidSyntax,
                    start_pos,
                    "Unterminated string literal",
                ));
            } else {
                lexeme.push(ch);
                value.push(ch);
                self.advance();
            }
        }

        Err(ParseError::new(
            ErrorKind::UnexpectedEof,
            start_pos,
            "Unterminated string literal",
        ))
    }

    fn read_char(&mut self) -> Result<Token> {
        let start_pos = self.current_position();
        let mut lexeme = String::new();

        lexeme.push('\'');
        self.advance();

        let ch = self.peek().ok_or_else(|| {
            ParseError::new(
                ErrorKind::UnexpectedEof,
                start_pos,
                "Unterminated char literal",
            )
        })?;

        lexeme.push(ch);
        self.advance();
        let value = if ch == '\\' {
            let escaped = self.peek().ok_or_else(|| {
                ParseError::new(
                    ErrorKind::UnexpectedEof,
                    start_pos,
                    "Unterminated char literal",
                )
            })?;
            lexeme.push(escaped);
            self.advance();
            match escaped {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                '\\' => '\\',
                '\'' => '\'',
                _ => escaped,
            }
        } else {
            ch
        };

        if self.peek() != Some('\'') {
            return Err(ParseError::new(
                ErrorKind::InvalidSyntax,
                start_pos,
                "Expected closing quote for char literal",
            ));
        }
        lexeme.push('\'');
        self.advance();

        Ok(Token::new(TokenKind::CharLiteral(value), lexeme, start_pos))
    }

    pub(crate) fn next_token(&mut self) -> Result<Token> {
        self.consume_trivia()?;
        if self.is_at_end() {
            return Ok(Token::new(TokenKind::Eof, "", self.current_position()));
        }

        let pos = self.current_position();
        let ch = self
            .peek()
            .ok_or_else(|| ParseError::new(ErrorKind::UnexpectedEof, pos, "Unexpected EOF"))?;

        // A wide literal, `L'x'` / `L"text"` (IDL 4.2 7.2.6.2-3): the same
        // token as the narrow one; the type it initializes says it is wide.
        if ch == 'L' && matches!(self.peek_ahead(1), Some('\'' | '"')) {
            self.advance();
            return if self.peek() == Some('"') {
                self.read_string()
            } else {
                self.read_char()
            };
        }
        if ch.is_ascii_alphabetic() || ch == '_' {
            return self.read_identifier();
        }
        if ch.is_ascii_digit() {
            return self.read_number();
        }
        if ch == '"' {
            return self.read_string();
        }
        if ch == '\'' {
            return self.read_char();
        }
        if ch == '#' {
            return self.read_preprocessor_token(pos);
        }
        if ch == '@' {
            self.advance();
            return Ok(Token::new(TokenKind::Annotation, "@", pos));
        }
        if let Some(token) = self.read_double_operator(ch, pos) {
            return Ok(token);
        }
        if let Some(kind) = Self::single_char_token(ch) {
            self.advance();
            return Ok(Token::new(kind, ch.to_string(), pos));
        }

        Err(ParseError::new(
            ErrorKind::UnexpectedToken,
            pos,
            format!("Unexpected character: '{ch}'"),
        ))
    }

    fn consume_trivia(&mut self) -> Result<()> {
        loop {
            self.skip_whitespace();

            if self.is_at_end() {
                return Ok(());
            }

            if self.peek() == Some('/') {
                if self.peek_ahead(1) == Some('/') {
                    self.skip_line_comment();
                    continue;
                }
                if self.peek_ahead(1) == Some('*') {
                    self.skip_block_comment()?;
                    continue;
                }
            }

            if self.peek() == Some('\n') {
                self.advance();
                continue;
            }

            break;
        }

        Ok(())
    }

    fn read_preprocessor_token(&mut self, pos: Position) -> Result<Token> {
        self.advance();
        let directive = self.read_identifier()?;
        let name = directive.lexeme;
        let kind = match name.as_str() {
            "define" => TokenKind::PreprocessorDefine,
            "include" => TokenKind::PreprocessorInclude,
            "ifdef" => TokenKind::PreprocessorIfdef,
            "ifndef" => TokenKind::PreprocessorIfndef,
            "else" => TokenKind::PreprocessorElse,
            "elif" => TokenKind::PreprocessorElif,
            "endif" => TokenKind::PreprocessorEndif,
            "undef" => TokenKind::PreprocessorUndef,
            "pragma" => TokenKind::PreprocessorPragma,
            other => {
                return Err(ParseError::new(
                    ErrorKind::PreprocessorError,
                    pos,
                    format!("Unknown preprocessor directive: {other}"),
                ));
            }
        };

        Ok(Token::new(kind, format!("#{name}"), pos))
    }

    fn read_double_operator(&mut self, ch: char, pos: Position) -> Option<Token> {
        let token = match (ch, self.peek_ahead(1)) {
            (':', Some(':')) => Some(Token::new(TokenKind::DoubleColon, "::", pos)),
            ('<', Some('<')) => Some(Token::new(TokenKind::ShiftLeft, "<<", pos)),
            ('>', Some('>')) => Some(Token::new(TokenKind::ShiftRight, ">>", pos)),
            ('&', Some('&')) => Some(Token::new(TokenKind::DoubleAmpersand, "&&", pos)),
            ('|', Some('|')) => Some(Token::new(TokenKind::DoublePipe, "||", pos)),
            _ => None,
        }?;
        self.advance();
        self.advance();
        Some(token)
    }

    const fn single_char_token(ch: char) -> Option<TokenKind> {
        Some(match ch {
            '{' => TokenKind::LeftBrace,
            '}' => TokenKind::RightBrace,
            '(' => TokenKind::LeftParen,
            ')' => TokenKind::RightParen,
            '[' => TokenKind::LeftBracket,
            ']' => TokenKind::RightBracket,
            '<' => TokenKind::LeftAngle,
            '>' => TokenKind::RightAngle,
            ';' => TokenKind::Semicolon,
            ',' => TokenKind::Comma,
            ':' => TokenKind::Colon,
            '=' => TokenKind::Equal,
            '+' => TokenKind::Plus,
            '-' => TokenKind::Minus,
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,
            '%' => TokenKind::Percent,
            '&' => TokenKind::Ampersand,
            '|' => TokenKind::Pipe,
            '^' => TokenKind::Caret,
            '!' => TokenKind::Bang,
            _ => return None,
        })
    }
}
