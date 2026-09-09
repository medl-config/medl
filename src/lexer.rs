use crate::error::{Error, ErrorKind};
use crate::span::{FileId, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Ident(String),
    Number(String),
    Str(Vec<StrPart>),
    KwExtends,
    KwWhen,
    KwElse,
    KwRequired,
    True,
    False,
    Null,
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Dot,
    Eq,
    PlusEq,
    MinusEq,
    FatArrow,
    EqEq,
    NotEq,
    AndAnd,
    OrOr,
    Question,
    Colon,
    Pipe,
    PipeGt,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StrPart {
    Text(String),
    Interp(Vec<Token>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// Maximum syntactic nesting depth, shared by the lexer (nested interpolations) and the
/// parser (nested expressions and blocks). Deeper input is an error rather than a stack overflow.
pub(crate) const MAX_NESTING: usize = 256;

pub fn lex(src: &str, file: FileId) -> Result<Vec<Token>, Error> {
    Lexer {
        src,
        pos: 0,
        file,
        depth: 0,
    }
    .run(false)
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    file: FileId,
    /// Current `${` nesting depth, capped at `MAX_NESTING`.
    depth: usize,
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }
    fn peek_at(&self, n: usize) -> Option<char> {
        self.src[self.pos..].chars().nth(n)
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }
    fn span_from(&self, start: usize) -> Span {
        Span::new(self.file, start as u32, self.pos as u32)
    }
    fn err(&self, start: usize, msg: impl Into<String>) -> Error {
        Error::new(ErrorKind::Lex, self.span_from(start), msg)
    }

    /// When `in_interp` is true, lexing stops at the `}` that closes the enclosing `${`
    /// (consuming it) and the returned vector ends with an `Eof` token.
    fn run(&mut self, in_interp: bool) -> Result<Vec<Token>, Error> {
        let mut out = Vec::new();
        let mut depth = 0usize;
        loop {
            self.skip_ws_and_comments();
            let start = self.pos;
            let Some(c) = self.peek() else {
                if in_interp {
                    return Err(self.err(start, "unterminated interpolation: expected '}'"));
                }
                out.push(Token {
                    kind: TokenKind::Eof,
                    span: self.span_from(start),
                });
                return Ok(out);
            };
            let kind = match c {
                '{' => {
                    self.bump();
                    depth += 1;
                    TokenKind::LBrace
                }
                '}' => {
                    self.bump();
                    if in_interp && depth == 0 {
                        out.push(Token {
                            kind: TokenKind::Eof,
                            span: Span::new(self.file, start as u32, start as u32),
                        });
                        return Ok(out);
                    }
                    depth = depth.saturating_sub(1);
                    TokenKind::RBrace
                }
                '(' => {
                    self.bump();
                    TokenKind::LParen
                }
                ')' => {
                    self.bump();
                    TokenKind::RParen
                }
                '[' => {
                    self.bump();
                    TokenKind::LBracket
                }
                ']' => {
                    self.bump();
                    TokenKind::RBracket
                }
                ',' => {
                    self.bump();
                    TokenKind::Comma
                }
                '.' => {
                    self.bump();
                    TokenKind::Dot
                }
                '?' => {
                    self.bump();
                    TokenKind::Question
                }
                ':' => {
                    self.bump();
                    TokenKind::Colon
                }
                '=' => {
                    self.bump();
                    match self.peek() {
                        Some('=') => {
                            self.bump();
                            TokenKind::EqEq
                        }
                        Some('>') => {
                            self.bump();
                            TokenKind::FatArrow
                        }
                        _ => TokenKind::Eq,
                    }
                }
                '+' => self.two_char('=', TokenKind::PlusEq, start)?,
                '-' => self.two_char('=', TokenKind::MinusEq, start)?,
                '!' => self.two_char('=', TokenKind::NotEq, start)?,
                '&' => self.two_char('&', TokenKind::AndAnd, start)?,
                '|' => {
                    self.bump();
                    match self.peek() {
                        Some('|') => {
                            self.bump();
                            TokenKind::OrOr
                        }
                        Some('>') => {
                            self.bump();
                            TokenKind::PipeGt
                        }
                        _ => TokenKind::Pipe,
                    }
                }
                '"' => {
                    self.bump();
                    self.lex_string(start)?
                }
                c if c.is_ascii_digit() => self.lex_number(),
                c if c.is_alphabetic() || c == '_' => self.lex_ident(),
                other => return Err(self.err(start, format!("unexpected character '{other}'"))),
            };
            out.push(Token {
                kind,
                span: self.span_from(start),
            });
        }
    }

    fn two_char(
        &mut self,
        second: char,
        kind: TokenKind,
        start: usize,
    ) -> Result<TokenKind, Error> {
        let first = self.bump().unwrap();
        if self.peek() == Some(second) {
            self.bump();
            Ok(kind)
        } else {
            Err(self.err(start, format!("unexpected character '{first}'")))
        }
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('#') => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                _ => break,
            }
        }
    }

    fn lex_number(&mut self) -> TokenKind {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.bump();
        }
        if self.peek() == Some('.') && matches!(self.peek_at(1), Some(c) if c.is_ascii_digit()) {
            self.bump();
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.bump();
            }
        }
        TokenKind::Number(self.src[start..self.pos].to_string())
    }

    fn lex_ident(&mut self) -> TokenKind {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_alphanumeric() || c == '_') {
            self.bump();
        }
        match &self.src[start..self.pos] {
            "extends" => TokenKind::KwExtends,
            "when" => TokenKind::KwWhen,
            "else" => TokenKind::KwElse,
            "required" => TokenKind::KwRequired,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "null" => TokenKind::Null,
            word => TokenKind::Ident(word.to_string()),
        }
    }

    fn lex_string(&mut self, start: usize) -> Result<TokenKind, Error> {
        let mut parts = Vec::new();
        let mut text = String::new();
        loop {
            match self.bump() {
                None => return Err(self.err(start, "unterminated string literal")),
                Some('"') => break,
                Some('\\') => {
                    let esc_start = self.pos - 1;
                    match self.bump() {
                        Some('n') => text.push('\n'),
                        Some('t') => text.push('\t'),
                        Some('"') => text.push('"'),
                        Some('\\') => text.push('\\'),
                        Some('$') => text.push('$'),
                        Some(other) => {
                            return Err(self.err(esc_start, format!("unknown escape '\\{other}'")))
                        }
                        None => return Err(self.err(start, "unterminated string literal")),
                    }
                }
                Some('$') if self.peek() == Some('{') => {
                    let interp_start = self.pos - 1;
                    self.bump();
                    if self.depth >= MAX_NESTING {
                        return Err(self.err(
                            interp_start,
                            format!("nesting too deep (limit {MAX_NESTING})"),
                        ));
                    }
                    if !text.is_empty() {
                        parts.push(StrPart::Text(std::mem::take(&mut text)));
                    }
                    self.depth += 1;
                    let inner = self.run(true);
                    self.depth -= 1;
                    parts.push(StrPart::Interp(inner?));
                }
                Some(c) => text.push(c),
            }
        }
        if !text.is_empty() {
            parts.push(StrPart::Text(text));
        }
        Ok(TokenKind::Str(parts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TokenKind::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        lex(src, FileId(0))
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn lexes_punctuation_and_operators() {
        assert_eq!(
            kinds("{ } ( ) [ ] , . = += -= => == != && || ? : | |>"),
            vec![
                LBrace, RBrace, LParen, RParen, LBracket, RBracket, Comma, Dot, Eq, PlusEq,
                MinusEq, FatArrow, EqEq, NotEq, AndAnd, OrOr, Question, Colon, Pipe, PipeGt, Eof
            ]
        );
    }

    #[test]
    fn lexes_keywords_identifiers_and_numbers() {
        assert_eq!(
            kinds("extends when else required true false null app_1 30 1.5"),
            vec![
                KwExtends,
                KwWhen,
                KwElse,
                KwRequired,
                True,
                False,
                Null,
                Ident("app_1".into()),
                Number("30".into()),
                Number("1.5".into()),
                Eof
            ]
        );
    }

    #[test]
    fn skips_comments_and_records_spans() {
        let toks = lex("# hi\nx = 1", FileId(0)).unwrap();
        assert_eq!(toks[0].kind, Ident("x".into()));
        assert_eq!((toks[0].span.start, toks[0].span.end), (5, 6));
        assert_eq!((toks[2].span.start, toks[2].span.end), (9, 10));
        assert_eq!(toks[3].kind, Eof);
    }

    #[test]
    fn rejects_unknown_character() {
        let err = lex("x = @", FileId(0)).unwrap_err();
        assert_eq!(err.message, "unexpected character '@'");
        assert_eq!(err.span.unwrap().start, 4);
    }

    #[test]
    fn rejects_lone_plus_and_bang() {
        assert_eq!(
            lex("+", FileId(0)).unwrap_err().message,
            "unexpected character '+'"
        );
        assert_eq!(
            lex("!", FileId(0)).unwrap_err().message,
            "unexpected character '!'"
        );
    }

    #[test]
    fn lexes_plain_string() {
        assert_eq!(
            kinds(r#""hi""#),
            vec![Str(vec![StrPart::Text("hi".into())]), Eof]
        );
        assert_eq!(kinds(r#""""#), vec![Str(vec![]), Eof]);
    }

    #[test]
    fn lexes_escapes() {
        assert_eq!(
            kinds(r#""a\"b\\c\n\t\$""#),
            vec![Str(vec![StrPart::Text("a\"b\\c\n\t$".into())]), Eof]
        );
        assert_eq!(
            lex(r#""\q""#, FileId(0)).unwrap_err().message,
            "unknown escape '\\q'"
        );
    }

    #[test]
    fn lexes_interpolation_with_nested_string_and_pipes() {
        let k = kinds(r#""${env.R | "eu" |> lower}!""#);
        let Str(parts) = &k[0] else {
            panic!("expected string, got {:?}", k[0])
        };
        assert_eq!(parts.len(), 2);
        let StrPart::Interp(inner) = &parts[0] else {
            panic!("expected interpolation")
        };
        let inner: Vec<TokenKind> = inner.iter().map(|t| t.kind.clone()).collect();
        assert_eq!(
            inner,
            vec![
                Ident("env".into()),
                Dot,
                Ident("R".into()),
                Pipe,
                Str(vec![StrPart::Text("eu".into())]),
                PipeGt,
                Ident("lower".into()),
                Eof
            ]
        );
        assert_eq!(parts[1], StrPart::Text("!".into()));
    }

    #[test]
    fn interpolation_may_contain_braces() {
        let k = kinds(r#""${when a { 1 => 2 }}""#);
        let Str(parts) = &k[0] else { panic!() };
        let StrPart::Interp(inner) = &parts[0] else {
            panic!()
        };
        assert_eq!(inner.last().unwrap().kind, Eof);
        assert_eq!(inner.iter().filter(|t| t.kind == RBrace).count(), 1);
    }

    #[test]
    fn interpolation_tokens_carry_absolute_spans() {
        let toks = lex(r#"x = "${ab}""#, FileId(0)).unwrap();
        let Str(parts) = &toks[2].kind else { panic!() };
        let StrPart::Interp(inner) = &parts[0] else {
            panic!()
        };
        assert_eq!((inner[0].span.start, inner[0].span.end), (7, 9));
    }

    #[test]
    fn unterminated_string_is_error() {
        assert_eq!(
            lex(r#""abc"#, FileId(0)).unwrap_err().message,
            "unterminated string literal"
        );
    }

    #[test]
    fn unterminated_interpolation_is_error() {
        assert_eq!(
            lex(r#""${a"#, FileId(0)).unwrap_err().message,
            "unterminated interpolation: expected '}'"
        );
    }

    /// Nested interpolations recurse through `run` and `lex_string`; the guard must fire
    /// before the stack does, exactly like the parser's limit.
    #[test]
    fn rejects_interpolations_nested_past_the_depth_limit() {
        let mut inner = "1".to_string();
        for _ in 0..3000 {
            inner = format!("\"${{{inner}}}\"");
        }
        let err = lex(&format!("x = {inner}"), FileId(0)).unwrap_err();
        assert_eq!(err.message, "nesting too deep (limit 256)");
    }

    #[test]
    fn accepts_interpolations_nested_below_the_depth_limit() {
        let mut inner = "1".to_string();
        for _ in 0..200 {
            inner = format!("\"${{{inner}}}\"");
        }
        lex(&format!("x = {inner}"), FileId(0)).unwrap();
    }
}
