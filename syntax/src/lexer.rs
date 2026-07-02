use crate::error::{CompileError, Result, SourceLocation, SourceSpan};

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Ident(String),
    Number(f64),
    String(String),
    Symbol(String),
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: SourceSpan,
}

pub fn lex(source: &str) -> Result<Vec<Token>> {
    let mut lexer = Lexer {
        source,
        chars: source.char_indices().peekable(),
        line: 1,
        column: 1,
    };
    lexer.lex_all()
}

struct Lexer<'a> {
    source: &'a str,
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    line: usize,
    column: usize,
}

impl Lexer<'_> {
    fn lex_all(&mut self) -> Result<Vec<Token>> {
        let mut tokens = Vec::new();
        loop {
            self.skip_ws_and_comments()?;
            let start = self.location();
            let Some((offset, ch)) = self.peek() else {
                tokens.push(Token {
                    kind: TokenKind::Eof,
                    span: SourceSpan { start, end: start },
                });
                return Ok(tokens);
            };
            let token = if is_ident_start(ch) {
                self.ident()
            } else if ch.is_ascii_digit() {
                self.number()?
            } else if ch == '"' {
                self.string()?
            } else {
                self.symbol(offset)?
            };
            tokens.push(token);
        }
    }

    fn skip_ws_and_comments(&mut self) -> Result<()> {
        loop {
            while matches!(self.peek().map(|(_, ch)| ch), Some(ch) if ch.is_whitespace()) {
                self.bump();
            }
            if self.starts_with("//") {
                while let Some((_, ch)) = self.peek() {
                    self.bump();
                    if ch == '\n' {
                        break;
                    }
                }
                continue;
            }
            if self.starts_with("/*") {
                self.bump();
                self.bump();
                loop {
                    if self.peek().is_none() {
                        return Err(CompileError::syntax(
                            "unterminated block comment",
                            self.location(),
                        ));
                    }
                    if self.starts_with("*/") {
                        self.bump();
                        self.bump();
                        break;
                    }
                    self.bump();
                }
                continue;
            }
            return Ok(());
        }
    }

    fn ident(&mut self) -> Token {
        let start = self.location();
        let mut text = String::new();
        while matches!(self.peek().map(|(_, ch)| ch), Some(ch) if is_ident_continue(ch)) {
            text.push(self.bump().unwrap().1);
        }
        let end = self.location();
        Token {
            kind: TokenKind::Ident(text),
            span: SourceSpan { start, end },
        }
    }

    fn number(&mut self) -> Result<Token> {
        let start = self.location();
        let mut text = String::new();
        while matches!(self.peek().map(|(_, ch)| ch), Some(ch) if ch.is_ascii_digit()) {
            text.push(self.bump().unwrap().1);
        }
        if self.peek().map(|(_, ch)| ch) == Some('.') {
            text.push(self.bump().unwrap().1);
            while matches!(self.peek().map(|(_, ch)| ch), Some(ch) if ch.is_ascii_digit()) {
                text.push(self.bump().unwrap().1);
            }
        }
        let value = text
            .parse::<f64>()
            .map_err(|_| CompileError::syntax("invalid number literal", start))?;
        let end = self.location();
        Ok(Token {
            kind: TokenKind::Number(value),
            span: SourceSpan { start, end },
        })
    }

    fn string(&mut self) -> Result<Token> {
        let start = self.location();
        self.bump();
        let mut text = String::new();
        loop {
            let Some((_, ch)) = self.bump() else {
                return Err(CompileError::syntax("unterminated string literal", start));
            };
            match ch {
                '"' => break,
                '\\' => {
                    let Some((_, escaped)) = self.bump() else {
                        return Err(CompileError::syntax("unterminated escape sequence", start));
                    };
                    text.push(match escaped {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '"' => '"',
                        '\\' => '\\',
                        other => other,
                    });
                }
                other => text.push(other),
            }
        }
        let end = self.location();
        Ok(Token {
            kind: TokenKind::String(text),
            span: SourceSpan { start, end },
        })
    }

    fn symbol(&mut self, _offset: usize) -> Result<Token> {
        let start = self.location();
        let two = ["->", "=>", "==", "!=", "<=", ">=", "&&", "||"];
        for item in two {
            if self.starts_with(item) {
                self.bump();
                self.bump();
                let end = self.location();
                return Ok(Token {
                    kind: TokenKind::Symbol(item.to_string()),
                    span: SourceSpan { start, end },
                });
            }
        }
        let ch = self.bump().unwrap().1;
        let allowed = "{}()[],.:;+-*/<>=?|!%".contains(ch);
        if !allowed {
            return Err(CompileError::syntax(
                format!("unexpected character {ch:?}"),
                start,
            ));
        }
        let end = self.location();
        Ok(Token {
            kind: TokenKind::Symbol(ch.to_string()),
            span: SourceSpan { start, end },
        })
    }

    fn starts_with(&mut self, needle: &str) -> bool {
        let Some((offset, _)) = self.peek() else {
            return false;
        };
        self.source[offset..].starts_with(needle)
    }

    fn peek(&mut self) -> Option<(usize, char)> {
        self.chars.peek().copied()
    }

    fn bump(&mut self) -> Option<(usize, char)> {
        let item = self.chars.next()?;
        if item.1 == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(item)
    }

    fn location(&mut self) -> SourceLocation {
        let offset = self
            .peek()
            .map(|(offset, _)| offset)
            .unwrap_or(self.source.len());
        SourceLocation {
            line: self.line,
            column: self.column,
            offset,
        }
    }
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    is_ident_start(ch) || ch.is_ascii_digit()
}
