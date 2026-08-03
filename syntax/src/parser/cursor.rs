use super::Parser;
use crate::{
    ast::BinaryOp,
    error::{CompileError, Result},
    lexer::{Token, TokenKind},
};

/// Binary operator precedence table. All operators are left-associative;
/// `parse_binary` tightens the right operand with `prec + 1`.
const BINARY_OPS: &[(&str, BinaryOp, u8)] = &[
    ("||", BinaryOp::Or, 1),
    ("&&", BinaryOp::And, 2),
    ("==", BinaryOp::Eq, 3),
    ("!=", BinaryOp::Ne, 3),
    ("<", BinaryOp::Lt, 4),
    ("<=", BinaryOp::Le, 4),
    (">", BinaryOp::Gt, 4),
    (">=", BinaryOp::Ge, 4),
    ("+", BinaryOp::Add, 5),
    ("-", BinaryOp::Sub, 5),
    ("*", BinaryOp::Mul, 6),
    ("/", BinaryOp::Div, 6),
];

impl Parser {
    pub(super) fn snapshot(&self) -> usize {
        self.current
    }

    pub(super) fn restore(&mut self, snapshot: usize) {
        self.current = snapshot;
    }

    pub(super) fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }

    pub(super) fn previous(&self) -> &Token {
        &self.tokens[self.current - 1]
    }

    pub(super) fn advance(&mut self) -> &Token {
        if !self.is_at_end() {
            self.current += 1;
        }
        self.previous()
    }

    pub(super) fn check_ident(&self, expected: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Ident(value) if value == expected)
    }

    pub(super) fn check_symbol(&self, expected: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Symbol(value) if value == expected)
    }

    pub(super) fn check_duration_literal(&self) -> bool {
        matches!(&self.peek().kind, TokenKind::Duration(_))
    }

    pub(super) fn check_function_start(&self) -> bool {
        self.check_ident("function")
            || self.check_ident("native")
            || self.check_ident("provider")
            || self.check_ident("static")
    }

    pub(super) fn check_provider_capability_start(&self) -> bool {
        if !self.check_ident("provider") {
            return false;
        }
        let Some(token) = self.tokens.get(self.current + 1) else {
            return false;
        };
        matches!(
            &token.kind,
            TokenKind::Ident(value)
                if value != "function"
                    && value != "native"
                    && value != "provider"
                    && value != "static"
        )
    }

    pub(super) fn match_ident(&mut self, expected: &str) -> bool {
        if self.check_ident(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub(super) fn match_symbol(&mut self, expected: &str) -> bool {
        if self.check_symbol(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub(super) fn match_statement_terminator(&mut self) -> bool {
        self.match_symbol(";") || self.match_symbol(",")
    }

    pub(super) fn expect_ident(&mut self, message: &str) -> Result<String> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::Ident(value) => Ok(value),
            _ => Err(CompileError::syntax(message, token.span.start)),
        }
    }

    pub(super) fn expect_ident_value(&mut self, expected: &str) -> Result<Token> {
        let token = self.advance().clone();
        match &token.kind {
            TokenKind::Ident(value) if value == expected => Ok(token),
            _ => Err(CompileError::syntax(
                format!("expected {expected}"),
                token.span.start,
            )),
        }
    }

    pub(super) fn expect_string(&mut self, message: &str) -> Result<String> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::String(value) => Ok(value),
            _ => Err(CompileError::syntax(message, token.span.start)),
        }
    }

    pub(super) fn expect_positive_integer(&mut self, message: &str) -> Result<u64> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::Number(value)
                if value.is_finite()
                    && value.fract() == 0.0
                    && value > 0.0
                    && value <= u64::MAX as f64 =>
            {
                Ok(value as u64)
            }
            _ => Err(CompileError::syntax(message, token.span.start)),
        }
    }

    pub(super) fn expect_symbol(&mut self, expected: &str) -> Result<()> {
        let token = self.advance().clone();
        match &token.kind {
            TokenKind::Symbol(value) if value == expected => Ok(()),
            _ => Err(CompileError::syntax(
                format!("expected symbol {expected}"),
                token.span.start,
            )),
        }
    }

    pub(super) fn is_at_end(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    pub(super) fn peek_binary_op(&self) -> Option<(BinaryOp, u8)> {
        let TokenKind::Symbol(value) = &self.peek().kind else {
            return None;
        };
        BINARY_OPS
            .iter()
            .find(|(symbol, _, _)| symbol == value)
            .map(|(_, op, prec)| (*op, *prec))
    }

    pub(super) fn skip_balanced_block(&mut self, unterminated_message: &str) -> Result<()> {
        self.expect_symbol("{")?;
        let mut depth = 1usize;
        while depth > 0 {
            let token = self.advance().clone();
            match token.kind {
                TokenKind::Symbol(value) if value == "{" => depth += 1,
                TokenKind::Symbol(value) if value == "}" => depth -= 1,
                TokenKind::Eof => {
                    return Err(CompileError::syntax(unterminated_message, token.span.start))
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn import_tail_is_terminated(&self) -> bool {
        self.is_at_end()
            || self.check_symbol(";")
            || self.check_ident("export")
            || self.check_ident("test")
            || self.check_ident("import")
            || self.check_ident("provider")
            || self.check_ident("const")
            || self.check_ident("type")
            || self.check_ident("actor")
            || self.check_ident("alias")
            || self.check_ident("interface")
            || self.check_ident("impl")
            || self.check_ident("db")
            || self.check_function_start()
    }
}
