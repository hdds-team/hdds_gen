// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Constant expression parsing and evaluation.
//!
//! Handles arithmetic expressions for const, enum, and array size declarations.

use crate::error::{ErrorKind, ParseError, Position, Result};
use crate::token::TokenKind;

use super::Parser;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Shl,
    Shr,
    BitAnd,
    BitOr,
    BitXor,
    LAnd,
    LOr,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum ConstValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
}

impl std::fmt::Display for ConstValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Int(i) => write!(f, "{i}"),
            Self::Float(v) => write!(f, "{v}"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Str(s) => write!(f, "{s}"),
        }
    }
}

impl ConstValue {
    #[must_use]
    pub(super) fn truthy(&self) -> bool {
        match self {
            Self::Bool(b) => *b,
            Self::Int(i) => *i != 0,
            Self::Float(f) => *f != 0.0,
            Self::Str(s) => !s.is_empty(),
        }
    }

    pub(super) fn as_int(&self, pos: Position) -> Result<i64> {
        match self {
            Self::Int(i) => Ok(*i),
            Self::Bool(b) => Ok(i64::from(*b)),
            _ => Err(ParseError::new(
                ErrorKind::InvalidSyntax,
                pos,
                "Expected integer",
            )),
        }
    }

    pub(super) fn unary_neg(self, pos: Position) -> Result<Self> {
        match self {
            Self::Int(i) => Ok(Self::Int(-i)),
            Self::Float(f) => Ok(Self::Float(-f)),
            _ => Err(ParseError::new(
                ErrorKind::InvalidSyntax,
                pos,
                "Invalid unary -",
            )),
        }
    }

    #[must_use]
    pub(super) fn unary_not(self) -> Self {
        Self::Bool(!self.truthy())
    }

    pub(super) fn apply_binary(self, op: &Op, rhs: Self, pos: Position) -> Result<Self> {
        let err = |msg: &str| Err(ParseError::new(ErrorKind::InvalidSyntax, pos, msg));

        match op {
            Op::Add => match (self, rhs) {
                (Self::Int(a), Self::Int(b)) => Ok(Self::Int(a.wrapping_add(b))),
                (Self::Float(a), Self::Float(b)) => Ok(Self::Float(a + b)),
                _ => err("+ expects numbers"),
            },
            Op::Sub => match (self, rhs) {
                (Self::Int(a), Self::Int(b)) => Ok(Self::Int(a.wrapping_sub(b))),
                (Self::Float(a), Self::Float(b)) => Ok(Self::Float(a - b)),
                _ => err("- expects numbers"),
            },
            Op::Mul => match (self, rhs) {
                (Self::Int(a), Self::Int(b)) => Ok(Self::Int(a.wrapping_mul(b))),
                (Self::Float(a), Self::Float(b)) => Ok(Self::Float(a * b)),
                _ => err("* expects numbers"),
            },
            Op::Div => match (self, rhs) {
                (Self::Int(_, ), Self::Int(0)) => err("Division by zero"),
                (Self::Int(a), Self::Int(b)) => Ok(Self::Int(a.wrapping_div(b))),
                (Self::Float(a), Self::Float(b)) => Ok(Self::Float(a / b)),
                _ => err("/ expects numbers"),
            },
            Op::Mod => match (self, rhs) {
                (Self::Int(_), Self::Int(0)) => err("Modulo by zero"),
                (Self::Int(a), Self::Int(b)) => Ok(Self::Int(a.wrapping_rem(b))),
                _ => err("% expects integers"),
            },
            Op::Shl => match (self, rhs) {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                (Self::Int(a), Self::Int(b)) if (0..64).contains(&b) => {
                    // Safety: b is in 0..64, fits in u32 without truncation or sign loss
                    Ok(Self::Int(a.wrapping_shl(b as u32)))
                }
                (Self::Int(_), Self::Int(_)) => err("Shift amount out of range (0..63)"),
                _ => err("<< expects integers"),
            },
            Op::Shr => match (self, rhs) {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                (Self::Int(a), Self::Int(b)) if (0..64).contains(&b) => {
                    // Safety: b is in 0..64, fits in u32 without truncation or sign loss
                    Ok(Self::Int(a.wrapping_shr(b as u32)))
                }
                (Self::Int(_), Self::Int(_)) => err("Shift amount out of range (0..63)"),
                _ => err(">> expects integers"),
            },
            Op::BitAnd => match (self, rhs) {
                (Self::Int(a), Self::Int(b)) => Ok(Self::Int(a & b)),
                _ => err("& expects integers"),
            },
            Op::BitOr => match (self, rhs) {
                (Self::Int(a), Self::Int(b)) => Ok(Self::Int(a | b)),
                _ => err("| expects integers"),
            },
            Op::BitXor => match (self, rhs) {
                (Self::Int(a), Self::Int(b)) => Ok(Self::Int(a ^ b)),
                _ => err("^ expects integers"),
            },
            Op::LAnd => Ok(Self::Bool(self.truthy() && rhs.truthy())),
            Op::LOr => Ok(Self::Bool(self.truthy() || rhs.truthy())),
        }
    }
}

impl Parser {
    pub(super) fn parse_const_expression(&mut self, min_prec: u8) -> Result<ConstValue> {
        let mut lhs = self.parse_const_unary()?;

        loop {
            let (op, prec, right_assoc) = match self.peek() {
                TokenKind::DoublePipe => (Op::LOr, 1, false),
                TokenKind::DoubleAmpersand => (Op::LAnd, 2, false),
                TokenKind::Pipe => (Op::BitOr, 3, false),
                TokenKind::Caret => (Op::BitXor, 4, false),
                TokenKind::Ampersand => (Op::BitAnd, 5, false),
                TokenKind::ShiftLeft => (Op::Shl, 6, false),
                TokenKind::ShiftRight => (Op::Shr, 6, false),
                TokenKind::Plus => (Op::Add, 7, false),
                TokenKind::Minus => (Op::Sub, 7, false),
                TokenKind::Star => (Op::Mul, 8, false),
                TokenKind::Slash => (Op::Div, 8, false),
                TokenKind::Percent => (Op::Mod, 8, false),
                _ => break,
            };

            if prec < min_prec {
                break;
            }
            let op_pos = self.current_position();
            self.advance();
            let next_min = if right_assoc { prec } else { prec + 1 };
            let rhs = self.parse_const_expression(next_min)?;
            lhs = lhs.apply_binary(&op, rhs, op_pos)?;
        }
        Ok(lhs)
    }

    fn parse_const_unary(&mut self) -> Result<ConstValue> {
        match self.peek() {
            TokenKind::Minus => {
                self.advance();
                let pos = self.current_position();
                let v = self.parse_const_unary()?;
                v.unary_neg(pos)
            }
            TokenKind::Plus => {
                self.advance();
                let v = self.parse_const_unary()?;
                Ok(v)
            }
            TokenKind::Bang => {
                self.advance();
                let v = self.parse_const_unary()?;
                Ok(v.unary_not())
            }
            TokenKind::LeftParen => {
                self.advance();
                let v = self.parse_const_expression(0)?;
                self.expect(&TokenKind::RightParen, "Expected ')' in expression")?;
                Ok(v)
            }
            TokenKind::IntegerLiteral(n) => {
                let v = ConstValue::Int(*n);
                self.advance();
                Ok(v)
            }
            TokenKind::FloatLiteral(f) => {
                let v = ConstValue::Float(*f);
                self.advance();
                Ok(v)
            }
            TokenKind::StringLiteral(s) => {
                let v = ConstValue::Str(s.clone());
                self.advance();
                Ok(v)
            }
            TokenKind::BoolLiteral(b) => {
                let v = ConstValue::Bool(*b);
                self.advance();
                Ok(v)
            }
            TokenKind::Identifier(name) => {
                let id = name.clone();
                self.advance();
                self.const_env.get(&id).cloned().ok_or_else(|| {
                    ParseError::new(
                        ErrorKind::InvalidSyntax,
                        self.current_position(),
                        format!("Unknown identifier in const expression: {id}"),
                    )
                })
            }
            _ => Err(ParseError::new(
                ErrorKind::InvalidSyntax,
                self.current_position(),
                "Expected expression",
            )),
        }
    }
}
