use crate::span::{FileId, Span};

pub const RESERVED_ROOTS: &[&str] = &["ctx", "env", "secret"];
pub const BUILTIN_NAMES: &[&str] = &[
    "upper",
    "lower",
    "trim",
    "replace",
    "split",
    "slice",
    "join",
    "len",
    "contains",
    "first",
    "last",
    "default",
    "snake_case",
    "kebab_case",
    "camel_case",
    "pascal_case",
    "capitalize",
    "error",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub file: FileId,
    pub statements: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Extends(ExtendsStmt),
    Block(BlockStmt),
    Assign(AssignStmt),
    Required(RequiredStmt),
    When(WhenStmt),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtendsStmt {
    pub paths: Vec<(String, Span)>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockStmt {
    pub name: Ident,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Set,
    Append,
    Remove,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssignStmt {
    pub name: Ident,
    pub op: AssignOp,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequiredStmt {
    pub path: Vec<Ident>,
    pub message: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhenArm<B> {
    pub pattern: Vec<Expr>,
    pub body: B,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhenStmt {
    pub subject: Vec<Expr>,
    pub arms: Vec<WhenArm<Vec<Stmt>>>,
    pub else_arm: Option<Vec<Stmt>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhenExpr {
    pub subject: Vec<Expr>,
    pub arms: Vec<WhenArm<Expr>>,
    pub else_arm: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Or,
    And,
    Eq,
    Ne,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    Text(String),
    Interp(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Null(Span),
    Bool(bool, Span),
    Int(i64, Span),
    Float(f64, Span),
    Str(Vec<Segment>, Span),
    List(Vec<Expr>, Span),
    Path(Vec<Ident>, Span),
    Call {
        name: Ident,
        args: Vec<Expr>,
        span: Span,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Ternary {
        cond: Box<Expr>,
        then: Box<Expr>,
        otherwise: Box<Expr>,
        span: Span,
    },
    Fallback {
        primary: Box<Expr>,
        fallback: Box<Expr>,
        span: Span,
    },
    When(Box<WhenExpr>),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Null(s)
            | Expr::Bool(_, s)
            | Expr::Int(_, s)
            | Expr::Float(_, s)
            | Expr::Str(_, s)
            | Expr::List(_, s)
            | Expr::Path(_, s) => *s,
            Expr::Call { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Ternary { span, .. }
            | Expr::Fallback { span, .. } => *span,
            Expr::When(w) => w.span,
        }
    }

    /// Pre-order visit of this expression and every sub-expression, including all `when` arms.
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a Expr)) {
        f(self);
        match self {
            Expr::Null(_) | Expr::Bool(..) | Expr::Int(..) | Expr::Float(..) | Expr::Path(..) => {}
            Expr::Str(segs, _) => {
                for s in segs {
                    if let Segment::Interp(e) = s {
                        e.walk(f);
                    }
                }
            }
            Expr::List(items, _) => {
                for e in items {
                    e.walk(f);
                }
            }
            Expr::Call { args, .. } => {
                for e in args {
                    e.walk(f);
                }
            }
            Expr::Binary { lhs, rhs, .. } => {
                lhs.walk(f);
                rhs.walk(f);
            }
            Expr::Ternary {
                cond,
                then,
                otherwise,
                ..
            } => {
                cond.walk(f);
                then.walk(f);
                otherwise.walk(f);
            }
            Expr::Fallback {
                primary, fallback, ..
            } => {
                primary.walk(f);
                fallback.walk(f);
            }
            Expr::When(w) => {
                for e in &w.subject {
                    e.walk(f);
                }
                for arm in &w.arms {
                    for e in &arm.pattern {
                        e.walk(f);
                    }
                    arm.body.walk(f);
                }
                if let Some(e) = &w.else_arm {
                    e.walk(f);
                }
            }
        }
    }
}
