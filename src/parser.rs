use crate::ast::*;
use crate::error::{Error, ErrorKind};
use crate::lexer::{lex, StrPart, Token, TokenKind, MAX_NESTING};
use crate::span::{FileId, Span};

pub fn parse_source(src: &str, file: FileId) -> Result<File, Error> {
    parse(lex(src, file)?, file)
}

pub(crate) fn parse(tokens: Vec<Token>, file: FileId) -> Result<File, Error> {
    let mut p = Parser {
        tokens,
        pos: 0,
        file,
        depth: 0,
        nesting: 0,
    };
    let mut statements = Vec::new();
    while !p.at(&TokenKind::Eof) {
        statements.push(p.parse_statement()?);
    }
    Ok(File { file, statements })
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    file: FileId,
    /// Number of enclosing `block` statements. 0 means top level, where reserved roots are rejected.
    depth: usize,
    /// Current recursion depth, capped at `MAX_NESTING`.
    nesting: usize,
}

pub(crate) fn describe(kind: &TokenKind) -> String {
    use TokenKind::*;
    match kind {
        Ident(n) => format!("identifier `{n}`"),
        Number(n) => format!("number `{n}`"),
        Str(_) => "string literal".to_string(),
        KwExtends => "`extends`".into(),
        KwWhen => "`when`".into(),
        KwElse => "`else`".into(),
        KwRequired => "`required`".into(),
        True => "`true`".into(),
        False => "`false`".into(),
        Null => "`null`".into(),
        LBrace => "`{`".into(),
        RBrace => "`}`".into(),
        LParen => "`(`".into(),
        RParen => "`)`".into(),
        LBracket => "`[`".into(),
        RBracket => "`]`".into(),
        Comma => "`,`".into(),
        Dot => "`.`".into(),
        Eq => "`=`".into(),
        PlusEq => "`+=`".into(),
        MinusEq => "`-=`".into(),
        FatArrow => "`=>`".into(),
        EqEq => "`==`".into(),
        NotEq => "`!=`".into(),
        AndAnd => "`&&`".into(),
        OrOr => "`||`".into(),
        Question => "`?`".into(),
        Colon => "`:`".into(),
        Pipe => "`|`".into(),
        PipeGt => "`|>`".into(),
        Eof => "end of file".into(),
    }
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }
    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }
    fn peek_kind_at(&self, n: usize) -> Option<&TokenKind> {
        self.tokens.get(self.pos + n).map(|t| &t.kind)
    }
    fn prev_end(&self) -> u32 {
        self.tokens[self.pos.saturating_sub(1)].span.end
    }
    fn bump(&mut self) -> Token {
        let t = self.peek().clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn at(&self, kind: &TokenKind) -> bool {
        self.peek_kind() == kind
    }
    fn eat(&mut self, kind: &TokenKind) -> Option<Token> {
        if self.at(kind) {
            Some(self.bump())
        } else {
            None
        }
    }
    fn unexpected(&self, what: &str) -> Error {
        Error::new(
            ErrorKind::Parse,
            self.peek().span,
            format!("expected {what}, found {}", describe(self.peek_kind())),
        )
    }
    fn expect(&mut self, kind: &TokenKind, what: &str) -> Result<Token, Error> {
        if self.at(kind) {
            Ok(self.bump())
        } else {
            Err(self.unexpected(what))
        }
    }
    fn expect_ident(&mut self, what: &str) -> Result<Ident, Error> {
        match self.peek_kind().clone() {
            TokenKind::Ident(name) => {
                let t = self.bump();
                Ok(Ident { name, span: t.span })
            }
            _ => Err(self.unexpected(what)),
        }
    }
    fn check_reserved_root(&self, ident: &Ident, action: &str) -> Result<(), Error> {
        if self.depth == 0 && RESERVED_ROOTS.contains(&ident.name.as_str()) {
            return Err(Error::new(
                ErrorKind::Parse,
                ident.span,
                format!("`{}` is a reserved root and cannot be {action}", ident.name),
            ));
        }
        Ok(())
    }

    /// Run `f` one level deeper, refusing to descend past `MAX_NESTING`. The counter is
    /// decremented on every exit path, so only genuine nesting accumulates.
    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, Error>) -> Result<T, Error> {
        if self.nesting >= MAX_NESTING {
            return Err(Error::new(
                ErrorKind::Parse,
                self.peek().span,
                format!("nesting too deep (limit {MAX_NESTING})"),
            ));
        }
        self.nesting += 1;
        let out = f(self);
        self.nesting -= 1;
        out
    }

    fn parse_statements_until_rbrace(&mut self) -> Result<(Vec<Stmt>, Token), Error> {
        self.nested(Self::parse_statements_until_rbrace_inner)
    }

    fn parse_statements_until_rbrace_inner(&mut self) -> Result<(Vec<Stmt>, Token), Error> {
        let mut stmts = Vec::new();
        loop {
            if let Some(close) = self.eat(&TokenKind::RBrace) {
                return Ok((stmts, close));
            }
            if self.at(&TokenKind::Eof) {
                return Err(self.unexpected("`}`"));
            }
            stmts.push(self.parse_statement()?);
        }
    }

    fn parse_statement(&mut self) -> Result<Stmt, Error> {
        match self.peek_kind() {
            TokenKind::KwExtends => self.parse_extends(),
            TokenKind::KwRequired => self.parse_required(),
            TokenKind::KwWhen => self.parse_when_stmt().map(Stmt::When),
            TokenKind::Ident(_) => match self.peek_kind_at(1) {
                Some(TokenKind::LBrace) => self.parse_block(),
                Some(TokenKind::Eq) | Some(TokenKind::PlusEq) | Some(TokenKind::MinusEq) => {
                    self.parse_assign()
                }
                _ => {
                    self.bump();
                    Err(self.unexpected("`{`, `=`, `+=` or `-=` after identifier"))
                }
            },
            _ => Err(self.unexpected("a statement")),
        }
    }

    fn parse_block(&mut self) -> Result<Stmt, Error> {
        let name = self.expect_ident("identifier")?;
        self.check_reserved_root(&name, "used as a block")?;
        self.expect(&TokenKind::LBrace, "`{`")?;
        self.depth += 1;
        let result = self.parse_statements_until_rbrace();
        self.depth -= 1;
        let (body, close) = result?;
        Ok(Stmt::Block(BlockStmt {
            span: name.span.to(close.span),
            name,
            body,
        }))
    }

    fn parse_assign(&mut self) -> Result<Stmt, Error> {
        let name = self.expect_ident("identifier")?;
        self.check_reserved_root(&name, "assigned")?;
        let op = match self.bump().kind {
            TokenKind::Eq => AssignOp::Set,
            TokenKind::PlusEq => AssignOp::Append,
            TokenKind::MinusEq => AssignOp::Remove,
            _ => unreachable!("parse_statement checked the operator"),
        };
        let value = self.parse_expr()?;
        Ok(Stmt::Assign(AssignStmt {
            span: name.span.to(value.span()),
            name,
            op,
            value,
        }))
    }

    fn parse_extends(&mut self) -> Result<Stmt, Error> {
        let kw = self.bump();
        self.expect(&TokenKind::LParen, "`(` after `extends`")?;
        let mut paths = Vec::new();
        loop {
            let tok = self.peek().clone();
            let TokenKind::Str(parts) = tok.kind else {
                return Err(self.unexpected("a string literal"));
            };
            self.bump();
            paths.push((
                plain_string(
                    parts,
                    tok.span,
                    "extends() takes plain string literals, not interpolated strings",
                )?,
                tok.span,
            ));
            if self.eat(&TokenKind::Comma).is_none() {
                break;
            }
        }
        let close = self.expect(&TokenKind::RParen, "`)`")?;
        Ok(Stmt::Extends(ExtendsStmt {
            paths,
            span: kw.span.to(close.span),
        }))
    }

    fn parse_required(&mut self) -> Result<Stmt, Error> {
        let kw = self.bump();
        let mut path = vec![self.expect_ident("a key name after `required`")?];
        while self.eat(&TokenKind::Dot).is_some() {
            path.push(self.expect_ident("identifier after `.`")?);
        }
        if self.depth == 0 && RESERVED_ROOTS.contains(&path[0].name.as_str()) {
            return Err(Error::new(
                ErrorKind::Parse,
                path[0].span,
                format!(
                    "`{}` is a reserved root and cannot be declared required",
                    path[0].name
                ),
            ));
        }
        let mut end = path.last().unwrap().span;
        let mut message = None;
        if let TokenKind::Str(parts) = self.peek_kind().clone() {
            let tok = self.bump();
            message = Some(plain_string(
                parts,
                tok.span,
                "required message must be a plain string literal",
            )?);
            end = tok.span;
        }
        Ok(Stmt::Required(RequiredStmt {
            path,
            message,
            span: kw.span.to(end),
        }))
    }

    fn parse_when_stmt(&mut self) -> Result<WhenStmt, Error> {
        let kw = self.bump();
        let subject = self.parse_subject_or_pattern()?;
        self.expect(&TokenKind::LBrace, "`{` after `when` subject")?;
        let mut arms = Vec::new();
        let mut else_arm = None;
        let close = loop {
            if let Some(close) = self.eat(&TokenKind::RBrace) {
                break close;
            }
            if let Some(kw_else) = self.eat(&TokenKind::KwElse) {
                if else_arm.is_some() {
                    return Err(Error::new(
                        ErrorKind::Parse,
                        kw_else.span,
                        "duplicate `else` arm",
                    ));
                }
                self.expect(&TokenKind::FatArrow, "`=>`")?;
                else_arm = Some(self.parse_arm_block()?.0);
                continue;
            }
            let start = self.peek().span;
            let pattern = self.parse_subject_or_pattern()?;
            let pattern_span = start.to(pattern.last().unwrap().span());
            self.check_arity(&pattern, &subject, pattern_span)?;
            self.expect(&TokenKind::FatArrow, "`=>`")?;
            let (body, close) = self.parse_arm_block()?;
            arms.push(WhenArm {
                span: start.to(close.span),
                pattern,
                body,
            });
        };
        Ok(WhenStmt {
            subject,
            arms,
            else_arm,
            span: kw.span.to(close.span),
        })
    }

    fn parse_arm_block(&mut self) -> Result<(Vec<Stmt>, Token), Error> {
        self.expect(
            &TokenKind::LBrace,
            "`{` after `=>` in a block-position `when`",
        )?;
        self.parse_statements_until_rbrace()
    }

    pub(crate) fn parse_expr(&mut self) -> Result<Expr, Error> {
        let mut e = self.parse_ternary()?;
        while self.eat(&TokenKind::PipeGt).is_some() {
            e = self.parse_pipe_target(e)?;
        }
        Ok(e)
    }

    /// Inside `${...}`: `ternary [ "|" ternary ] { "|>" call }`.
    fn parse_interp_expr(&mut self) -> Result<Expr, Error> {
        let mut e = self.parse_ternary()?;
        if self.eat(&TokenKind::Pipe).is_some() {
            let fallback = self.parse_ternary()?;
            let span = e.span().to(fallback.span());
            e = Expr::Fallback {
                primary: Box::new(e),
                fallback: Box::new(fallback),
                span,
            };
        }
        while self.eat(&TokenKind::PipeGt).is_some() {
            e = self.parse_pipe_target(e)?;
        }
        Ok(e)
    }

    fn parse_pipe_target(&mut self, input: Expr) -> Result<Expr, Error> {
        let name = self.expect_ident("a function name after `|>`")?;
        self.check_builtin(&name)?;
        let mut args = vec![input];
        if self.eat(&TokenKind::LParen).is_some() {
            args.extend(self.parse_args()?);
        }
        let span = Span::new(self.file, args[0].span().start, self.prev_end());
        Ok(Expr::Call { name, args, span })
    }

    fn check_builtin(&self, name: &Ident) -> Result<(), Error> {
        if BUILTIN_NAMES.contains(&name.name.as_str()) {
            return Ok(());
        }
        Err(Error::new(
            ErrorKind::Parse,
            name.span,
            format!(
                "unknown function `{}`; only builtins may be called",
                name.name
            ),
        ))
    }

    /// Called after `(` was consumed; consumes through `)`.
    fn parse_args(&mut self) -> Result<Vec<Expr>, Error> {
        let mut args = Vec::new();
        while !self.at(&TokenKind::RParen) {
            args.push(self.parse_expr()?);
            if self.eat(&TokenKind::Comma).is_none() {
                break;
            }
        }
        self.expect(&TokenKind::RParen, "`)`")?;
        Ok(args)
    }

    /// The funnel every expression form passes through, so the nesting cap lives here.
    fn parse_ternary(&mut self) -> Result<Expr, Error> {
        self.nested(Self::parse_ternary_inner)
    }

    fn parse_ternary_inner(&mut self) -> Result<Expr, Error> {
        let cond = self.parse_or()?;
        if self.eat(&TokenKind::Question).is_none() {
            return Ok(cond);
        }
        let then = self.parse_expr()?;
        self.expect(&TokenKind::Colon, "`:` in ternary")?;
        let otherwise = self.parse_expr()?;
        let span = cond.span().to(otherwise.span());
        Ok(Expr::Ternary {
            cond: Box::new(cond),
            then: Box::new(then),
            otherwise: Box::new(otherwise),
            span,
        })
    }

    fn parse_or(&mut self) -> Result<Expr, Error> {
        let mut lhs = self.parse_and()?;
        while self.eat(&TokenKind::OrOr).is_some() {
            let rhs = self.parse_and()?;
            lhs = binary(BinOp::Or, lhs, rhs);
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Expr, Error> {
        let mut lhs = self.parse_equality()?;
        while self.eat(&TokenKind::AndAnd).is_some() {
            let rhs = self.parse_equality()?;
            lhs = binary(BinOp::And, lhs, rhs);
        }
        Ok(lhs)
    }

    fn parse_equality(&mut self) -> Result<Expr, Error> {
        let mut lhs = self.parse_primary()?;
        loop {
            let op = if self.eat(&TokenKind::EqEq).is_some() {
                BinOp::Eq
            } else if self.eat(&TokenKind::NotEq).is_some() {
                BinOp::Ne
            } else {
                return Ok(lhs);
            };
            let rhs = self.parse_primary()?;
            lhs = binary(op, lhs, rhs);
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, Error> {
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Null => {
                self.bump();
                Ok(Expr::Null(tok.span))
            }
            TokenKind::True => {
                self.bump();
                Ok(Expr::Bool(true, tok.span))
            }
            TokenKind::False => {
                self.bump();
                Ok(Expr::Bool(false, tok.span))
            }
            TokenKind::Number(text) => {
                self.bump();
                let out_of_range =
                    || Error::new(ErrorKind::Parse, tok.span, "number literal out of range");
                if text.contains('.') {
                    Ok(Expr::Float(
                        text.parse::<f64>().map_err(|_| out_of_range())?,
                        tok.span,
                    ))
                } else {
                    Ok(Expr::Int(
                        text.parse::<i64>().map_err(|_| out_of_range())?,
                        tok.span,
                    ))
                }
            }
            TokenKind::Str(parts) => {
                self.bump();
                self.parse_string(parts, tok.span)
            }
            TokenKind::LBracket => {
                self.bump();
                let mut items = Vec::new();
                while !self.at(&TokenKind::RBracket) {
                    items.push(self.parse_expr()?);
                    if self.eat(&TokenKind::Comma).is_none() {
                        break;
                    }
                }
                let close = self.expect(&TokenKind::RBracket, "`]`")?;
                Ok(Expr::List(items, tok.span.to(close.span)))
            }
            TokenKind::LParen => {
                self.bump();
                let e = self.parse_expr()?;
                self.expect(&TokenKind::RParen, "`)`")?;
                Ok(e)
            }
            TokenKind::KwWhen => self.parse_when_expr(),
            TokenKind::Ident(_) => {
                if self.peek_kind_at(1) == Some(&TokenKind::LParen) {
                    self.parse_call()
                } else {
                    self.parse_path()
                }
            }
            _ => Err(self.unexpected("an expression")),
        }
    }

    fn parse_call(&mut self) -> Result<Expr, Error> {
        let name = self.expect_ident("a function name")?;
        self.check_builtin(&name)?;
        self.expect(&TokenKind::LParen, "`(`")?;
        let args = self.parse_args()?;
        let span = Span::new(self.file, name.span.start, self.prev_end());
        Ok(Expr::Call { name, args, span })
    }

    fn parse_path(&mut self) -> Result<Expr, Error> {
        let mut ids = vec![self.expect_ident("identifier")?];
        while self.eat(&TokenKind::Dot).is_some() {
            ids.push(self.expect_ident("identifier after `.`")?);
        }
        let span = ids[0].span.to(ids.last().unwrap().span);
        Ok(Expr::Path(ids, span))
    }

    fn parse_string(&mut self, parts: Vec<StrPart>, span: Span) -> Result<Expr, Error> {
        let mut segs = Vec::new();
        for part in parts {
            match part {
                StrPart::Text(t) => segs.push(Segment::Text(t)),
                StrPart::Interp(tokens) => {
                    let mut sub = Parser {
                        tokens,
                        pos: 0,
                        file: self.file,
                        depth: self.depth,
                        nesting: self.nesting,
                    };
                    let e = sub.parse_interp_expr()?;
                    if !sub.at(&TokenKind::Eof) {
                        return Err(sub.unexpected("`}` to close interpolation"));
                    }
                    segs.push(Segment::Interp(e));
                }
            }
        }
        Ok(Expr::Str(segs, span))
    }

    /// `(a, b)` with a top-level comma is a tuple; `(a)` is a parenthesized expression.
    fn looks_like_tuple(&self) -> bool {
        if !self.at(&TokenKind::LParen) {
            return false;
        }
        let mut depth = 0usize;
        for tok in &self.tokens[self.pos..] {
            match tok.kind {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        return false;
                    }
                }
                TokenKind::Comma if depth == 1 => return true,
                TokenKind::Eof => return false,
                _ => {}
            }
        }
        false
    }

    /// Subject or pattern of a `when`: a tuple `(a, b)` or a single expression.
    fn parse_subject_or_pattern(&mut self) -> Result<Vec<Expr>, Error> {
        if !self.looks_like_tuple() {
            return Ok(vec![self.parse_expr()?]);
        }
        self.bump();
        let mut items = Vec::new();
        loop {
            items.push(self.parse_expr()?);
            if self.eat(&TokenKind::Comma).is_none() {
                break;
            }
        }
        self.expect(&TokenKind::RParen, "`)`")?;
        Ok(items)
    }

    fn check_arity(&self, pattern: &[Expr], subject: &[Expr], span: Span) -> Result<(), Error> {
        if pattern.len() == subject.len() {
            return Ok(());
        }
        Err(Error::new(
            ErrorKind::Parse,
            span,
            format!(
                "pattern has {} element(s) but the `when` subject has {}",
                pattern.len(),
                subject.len()
            ),
        ))
    }

    fn parse_when_expr(&mut self) -> Result<Expr, Error> {
        let kw = self.bump();
        let subject = self.parse_subject_or_pattern()?;
        self.expect(&TokenKind::LBrace, "`{` after `when` subject")?;
        let mut arms = Vec::new();
        let mut else_arm = None;
        let close = loop {
            if let Some(close) = self.eat(&TokenKind::RBrace) {
                break close;
            }
            if let Some(kw_else) = self.eat(&TokenKind::KwElse) {
                if else_arm.is_some() {
                    return Err(Error::new(
                        ErrorKind::Parse,
                        kw_else.span,
                        "duplicate `else` arm",
                    ));
                }
                self.expect(&TokenKind::FatArrow, "`=>`")?;
                else_arm = Some(Box::new(self.parse_expr()?));
                continue;
            }
            let start = self.peek().span;
            let pattern = self.parse_subject_or_pattern()?;
            let pattern_span = start.to(pattern.last().unwrap().span());
            self.check_arity(&pattern, &subject, pattern_span)?;
            self.expect(&TokenKind::FatArrow, "`=>`")?;
            let body = self.parse_expr()?;
            arms.push(WhenArm {
                span: start.to(body.span()),
                pattern,
                body,
            });
        };
        Ok(Expr::When(Box::new(WhenExpr {
            subject,
            arms,
            else_arm,
            span: kw.span.to(close.span),
        })))
    }
}

fn binary(op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
    let span = lhs.span().to(rhs.span());
    Expr::Binary {
        op,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
        span,
    }
}

fn plain_string(parts: Vec<StrPart>, span: Span, err: &str) -> Result<String, Error> {
    let mut out = String::new();
    for p in parts {
        match p {
            StrPart::Text(t) => out.push_str(&t),
            StrPart::Interp(_) => return Err(Error::new(ErrorKind::Parse, span, err)),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &str) -> File {
        parse_source(src, FileId(0)).unwrap_or_else(|e| panic!("{}: {src}", e.message))
    }
    fn parse_err(src: &str) -> String {
        parse_source(src, FileId(0))
            .expect_err("expected parse error")
            .message
    }

    #[test]
    fn parses_assignment_with_each_operator() {
        let f = parse_ok("a = 1\nb += [1]\nc -= [1, 2]");
        let ops: Vec<AssignOp> = f
            .statements
            .iter()
            .map(|s| match s {
                Stmt::Assign(a) => a.op,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(ops, vec![AssignOp::Set, AssignOp::Append, AssignOp::Remove]);
        let Stmt::Assign(a) = &f.statements[0] else {
            panic!()
        };
        assert_eq!(a.name.name, "a");
        assert_eq!(a.value, Expr::Int(1, Span::new(FileId(0), 4, 5)));
        assert_eq!(a.span, Span::new(FileId(0), 0, 5));
    }

    #[test]
    fn parses_nested_blocks() {
        let f = parse_ok("app { build { x = 1 } }");
        let Stmt::Block(app) = &f.statements[0] else {
            panic!()
        };
        assert_eq!(app.name.name, "app");
        let Stmt::Block(build) = &app.body[0] else {
            panic!()
        };
        assert_eq!(build.name.name, "build");
        assert!(matches!(build.body[0], Stmt::Assign(_)));
        assert_eq!(app.span, Span::new(FileId(0), 0, 23));
    }

    #[test]
    fn parses_extends_with_multiple_paths() {
        let f = parse_ok(r#"extends("a.medl", "b/c.medl")"#);
        let Stmt::Extends(e) = &f.statements[0] else {
            panic!()
        };
        let paths: Vec<&str> = e.paths.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, vec!["a.medl", "b/c.medl"]);
        assert_eq!(e.paths[1].1, Span::new(FileId(0), 18, 28));
    }

    #[test]
    fn rejects_interpolated_extends_path() {
        assert_eq!(
            parse_err(r#"extends("${x}.medl")"#),
            "extends() takes plain string literals, not interpolated strings"
        );
    }

    #[test]
    fn parses_required_with_dotted_path_and_message() {
        let f = parse_ok(r#"app { required session.app_id "needs id" }"#);
        let Stmt::Block(app) = &f.statements[0] else {
            panic!()
        };
        let Stmt::Required(r) = &app.body[0] else {
            panic!()
        };
        let names: Vec<&str> = r.path.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["session", "app_id"]);
        assert_eq!(r.message.as_deref(), Some("needs id"));
    }

    #[test]
    fn parses_required_without_message_followed_by_statement() {
        let f = parse_ok("required a\nb = 1");
        assert_eq!(f.statements.len(), 2);
        let Stmt::Required(r) = &f.statements[0] else {
            panic!()
        };
        assert_eq!(r.message, None);
    }

    #[test]
    fn rejects_required_on_reserved_root_at_top_level() {
        assert_eq!(
            parse_err("required ctx.platform"),
            "`ctx` is a reserved root and cannot be declared required"
        );
        assert_eq!(
            parse_err("required env"),
            "`env` is a reserved root and cannot be declared required"
        );
    }

    #[test]
    fn allows_reserved_names_inside_blocks() {
        parse_ok("app { required ctx }");
        parse_ok("app { env = 1 }");
    }

    #[test]
    fn rejects_assignment_or_block_on_reserved_root_at_top_level() {
        assert_eq!(
            parse_err("ctx = 1"),
            "`ctx` is a reserved root and cannot be assigned"
        );
        assert_eq!(
            parse_err("secret { }"),
            "`secret` is a reserved root and cannot be used as a block"
        );
    }

    #[test]
    fn rejects_reserved_word_as_statement_start() {
        assert_eq!(parse_err("true = 1"), "expected a statement, found `true`");
    }

    #[test]
    fn rejects_identifier_not_followed_by_block_or_assignment() {
        assert_eq!(
            parse_err("app 1"),
            "expected `{`, `=`, `+=` or `-=` after identifier, found number `1`"
        );
    }

    #[test]
    fn reports_missing_closing_brace() {
        assert_eq!(parse_err("app { x = 1"), "expected `}`, found end of file");
    }

    /// Parse `x = <src>` and return the right-hand side.
    fn expr_of(src: &str) -> Expr {
        let f = parse_ok(&format!("x = {src}"));
        let Stmt::Assign(a) = f.statements.into_iter().next().unwrap() else {
            panic!()
        };
        a.value
    }
    fn path(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }
    fn path_names(e: &Expr) -> Vec<String> {
        let Expr::Path(ids, _) = e else {
            panic!("not a path: {e:?}")
        };
        ids.iter().map(|i| i.name.clone()).collect()
    }

    #[test]
    fn parses_literals() {
        assert!(matches!(expr_of("null"), Expr::Null(_)));
        assert!(matches!(expr_of("true"), Expr::Bool(true, _)));
        assert!(matches!(expr_of("false"), Expr::Bool(false, _)));
        assert!(matches!(expr_of("42"), Expr::Int(42, _)));
        assert!(matches!(expr_of("1.5"), Expr::Float(f, _) if f == 1.5));
        assert!(
            matches!(expr_of(r#""s""#), Expr::Str(ref segs, _) if segs == &vec![Segment::Text("s".into())])
        );
    }

    #[test]
    fn parses_paths_and_calls() {
        assert_eq!(path_names(&expr_of("app.sku")), path(&["app", "sku"]));
        let Expr::Call { name, args, .. } = expr_of(r#"replace(a, "b", "c")"#) else {
            panic!()
        };
        assert_eq!(name.name, "replace");
        assert_eq!(args.len(), 3);
        assert_eq!(path_names(&args[0]), path(&["a"]));
    }

    #[test]
    fn rejects_unknown_function() {
        assert_eq!(
            parse_err("x = foo(1)"),
            "unknown function `foo`; only builtins may be called"
        );
        assert_eq!(
            parse_err("x = a |> foo"),
            "unknown function `foo`; only builtins may be called"
        );
    }

    #[test]
    fn binary_precedence_is_or_then_and_then_equality() {
        let Expr::Binary {
            op: BinOp::Or,
            lhs,
            rhs,
            ..
        } = expr_of("a || b && c == d")
        else {
            panic!()
        };
        assert_eq!(path_names(&lhs), path(&["a"]));
        let Expr::Binary {
            op: BinOp::And,
            lhs: b,
            rhs: eq,
            ..
        } = *rhs
        else {
            panic!()
        };
        assert_eq!(path_names(&b), path(&["b"]));
        assert!(matches!(*eq, Expr::Binary { op: BinOp::Eq, .. }));
        assert!(matches!(
            expr_of("a != b"),
            Expr::Binary { op: BinOp::Ne, .. }
        ));
    }

    #[test]
    fn parses_ternary_and_parentheses() {
        let Expr::Ternary {
            cond,
            then,
            otherwise,
            ..
        } = expr_of("a == b ? 1 : c ? 2 : 3")
        else {
            panic!()
        };
        assert!(matches!(*cond, Expr::Binary { op: BinOp::Eq, .. }));
        assert!(matches!(*then, Expr::Int(1, _)));
        assert!(matches!(*otherwise, Expr::Ternary { .. }));
        let Expr::Binary {
            op: BinOp::And,
            lhs,
            ..
        } = expr_of("(a || b) && c")
        else {
            panic!()
        };
        assert!(matches!(*lhs, Expr::Binary { op: BinOp::Or, .. }));
    }

    #[test]
    fn pipes_desugar_to_nested_calls() {
        let Expr::Call { name, args, .. } = expr_of(r#"a |> upper |> replace(".", "_")"#) else {
            panic!()
        };
        assert_eq!(name.name, "replace");
        assert_eq!(args.len(), 3);
        let Expr::Call {
            name: inner,
            args: inner_args,
            ..
        } = &args[0]
        else {
            panic!()
        };
        assert_eq!(inner.name, "upper");
        assert_eq!(path_names(&inner_args[0]), path(&["a"]));
    }

    #[test]
    fn parses_string_interpolation_segments() {
        let Expr::Str(segs, _) = expr_of(r#""x${a.b}y""#) else {
            panic!()
        };
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0], Segment::Text("x".into()));
        let Segment::Interp(e) = &segs[1] else {
            panic!()
        };
        assert_eq!(path_names(e), path(&["a", "b"]));
        assert_eq!(segs[2], Segment::Text("y".into()));
    }

    #[test]
    fn interpolation_fallback_binds_tighter_than_pipe() {
        let Expr::Str(segs, _) = expr_of(r#""${env.R | "eu" |> lower}""#) else {
            panic!()
        };
        let Segment::Interp(Expr::Call { name, args, .. }) = &segs[0] else {
            panic!("{segs:?}")
        };
        assert_eq!(name.name, "lower");
        let Expr::Fallback {
            primary, fallback, ..
        } = &args[0]
        else {
            panic!("{:?}", args[0])
        };
        assert_eq!(path_names(primary), path(&["env", "R"]));
        assert!(matches!(**fallback, Expr::Str(_, _)));
    }

    #[test]
    fn interpolation_span_is_absolute() {
        let Expr::Str(segs, _) = expr_of(r#""${ab}""#) else {
            panic!()
        };
        let Segment::Interp(e) = &segs[0] else {
            panic!()
        };
        assert_eq!(e.span(), Span::new(FileId(0), 7, 9));
    }

    #[test]
    fn fallback_outside_interpolation_is_rejected() {
        assert_eq!(
            parse_err(r#"x = env.R | "eu""#),
            "expected a statement, found `|`"
        );
    }

    #[test]
    fn parses_value_position_when() {
        let Expr::When(w) = expr_of(r#"when ctx.p { "mobile" => 1  "xr" => 2  else => 3 }"#) else {
            panic!()
        };
        assert_eq!(w.subject.len(), 1);
        assert_eq!(w.arms.len(), 2);
        assert!(matches!(w.arms[1].body, Expr::Int(2, _)));
        assert!(matches!(w.else_arm.as_deref(), Some(Expr::Int(3, _))));
    }

    #[test]
    fn parses_tuple_subject_and_patterns() {
        let Expr::When(w) = expr_of(r#"when (a, b) { (1, 2) => 3 }"#) else {
            panic!()
        };
        assert_eq!(w.subject.len(), 2);
        assert_eq!(w.arms[0].pattern.len(), 2);
        let Expr::When(w) = expr_of(r#"when (a) { (1) => 3 }"#) else {
            panic!()
        };
        assert_eq!(w.subject.len(), 1);
        assert_eq!(w.arms[0].pattern.len(), 1);
    }

    #[test]
    fn rejects_pattern_arity_mismatch_and_duplicate_else() {
        assert_eq!(
            parse_err("x = when (a, b) { 1 => 2 }"),
            "pattern has 1 element(s) but the `when` subject has 2"
        );
        assert_eq!(
            parse_err("x = when a { else => 1  else => 2 }"),
            "duplicate `else` arm"
        );
    }

    #[test]
    fn rejects_block_body_in_value_position_when() {
        assert_eq!(
            parse_err("x = when a { 1 => { y = 1 } }"),
            "expected an expression, found `{`"
        );
    }

    #[test]
    fn parses_block_position_when_with_else() {
        let f = parse_ok(r#"app { when ctx.platform { "mobile" => { x = 1 } else => {} } }"#);
        let Stmt::Block(app) = &f.statements[0] else {
            panic!()
        };
        let Stmt::When(w) = &app.body[0] else {
            panic!("{:?}", app.body[0])
        };
        assert_eq!(w.subject.len(), 1);
        assert_eq!(w.arms.len(), 1);
        assert!(matches!(w.arms[0].body[0], Stmt::Assign(_)));
        assert_eq!(w.else_arm.as_ref().map(|b| b.len()), Some(0));
    }

    #[test]
    fn when_arm_bodies_may_contain_extends_and_required() {
        let f = parse_ok(
            r#"when ctx.v { "c" => { extends("c.medl")  required session.app_id "msg" } }"#,
        );
        let Stmt::When(w) = &f.statements[0] else {
            panic!()
        };
        assert!(matches!(w.arms[0].body[0], Stmt::Extends(_)));
        assert!(matches!(w.arms[0].body[1], Stmt::Required(_)));
        assert_eq!(w.else_arm, None);
    }

    #[test]
    fn block_position_when_requires_block_bodies() {
        assert_eq!(
            parse_err("when a { 1 => 2 }"),
            "expected `{` after `=>` in a block-position `when`, found number `2`"
        );
    }

    #[test]
    fn top_level_when_arms_still_reject_reserved_roots() {
        assert_eq!(
            parse_err(r#"when ctx.x { "a" => { ctx = 1 } }"#),
            "`ctx` is a reserved root and cannot be assigned"
        );
    }

    #[test]
    fn block_when_rejects_arity_mismatch() {
        assert_eq!(
            parse_err("when (a, b) { 1 => { } }"),
            "pattern has 1 element(s) but the `when` subject has 2"
        );
    }

    /// A libtest thread gets a 2 MiB stack, which a debug build outgrows well before the
    /// nesting guard fires, so deep-nesting checks run with a stack a real program would have.
    fn deep<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap()
    }

    #[test]
    fn rejects_expressions_nested_past_the_depth_limit() {
        let parens = format!("x = {}1{}", "(".repeat(300), ")".repeat(300));
        assert_eq!(
            deep(move || parse_err(&parens)),
            "nesting too deep (limit 256)"
        );
        let ternaries = format!("x = {}2", "a ? 1 : ".repeat(300));
        assert_eq!(
            deep(move || parse_err(&ternaries)),
            "nesting too deep (limit 256)"
        );
        let interpolated = format!("x = \"${{{}1{}}}\"", "(".repeat(300), ")".repeat(300));
        assert_eq!(
            deep(move || parse_err(&interpolated)),
            "nesting too deep (limit 256)"
        );
    }

    #[test]
    fn rejects_statements_nested_past_the_depth_limit() {
        let blocks = format!("{}x = 1{}", "a { ".repeat(300), " }".repeat(300));
        assert_eq!(
            deep(move || parse_err(&blocks)),
            "nesting too deep (limit 256)"
        );
    }

    #[test]
    fn accepts_nesting_below_the_depth_limit() {
        let parens = format!("x = {}1{}", "(".repeat(200), ")".repeat(200));
        deep(move || parse_ok(&parens));
        let blocks = format!("{}x = 1{}", "a { ".repeat(200), " }".repeat(200));
        deep(move || parse_ok(&blocks));
    }

    #[test]
    fn parses_every_example_file() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        for rel in [
            "base.medl",
            "app.medl",
            "platforms/mobile.medl",
            "platforms/xr.medl",
            "variants/expo.medl",
            "variants/store.medl",
        ] {
            let src = std::fs::read_to_string(root.join(rel)).unwrap();
            parse_source(&src, FileId(0)).unwrap_or_else(|e| panic!("{rel}: {}", e.message));
        }
    }
}
