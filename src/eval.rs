use crate::ast::*;
use crate::builtins;
use crate::error::{Error, ErrorKind};
use crate::inputs::Inputs;
use crate::span::Span;
use crate::value::Value;

pub trait Scope {
    fn inputs(&self) -> &Inputs;
    /// Config-path lookup (root is never ctx/env/secret). `Ok(None)` means the key does not exist.
    fn lookup(&self, path: &[String], span: Span) -> Result<Option<Value>, Error>;
}

pub fn eval(expr: &Expr, scope: &dyn Scope) -> Result<Value, Error> {
    match expr {
        Expr::Null(_) => Ok(Value::Null),
        Expr::Bool(b, _) => Ok(Value::Bool(*b)),
        Expr::Int(n, _) => Ok(Value::Int(*n)),
        Expr::Float(f, _) => Ok(Value::Float(*f)),
        Expr::Str(segs, _) => eval_string(segs, scope),
        Expr::List(items, _) => eval_all(items, scope).map(Value::List),
        Expr::Path(ids, span) => {
            eval_path(ids, *span, scope)?.ok_or_else(|| missing_error(ids, *span))
        }
        Expr::Call { name, args, span } if name.name == "default" => {
            if args.len() != 2 {
                return Err(Error::new(
                    ErrorKind::Resolve,
                    *span,
                    format!("default() expects 2 argument(s), got {}", args.len()),
                ));
            }
            match eval_optional(&args[0], scope)? {
                Some(v) => Ok(v),
                None => eval(&args[1], scope),
            }
        }
        Expr::Call { name, args, span } => {
            builtins::call(&name.name, eval_all(args, scope)?, *span)
        }
        Expr::Binary {
            op: BinOp::Eq,
            lhs,
            rhs,
            ..
        } => Ok(Value::Bool(eval(lhs, scope)? == eval(rhs, scope)?)),
        Expr::Binary {
            op: BinOp::Ne,
            lhs,
            rhs,
            ..
        } => Ok(Value::Bool(eval(lhs, scope)? != eval(rhs, scope)?)),
        Expr::Binary {
            op: BinOp::And,
            lhs,
            rhs,
            ..
        } => {
            if !eval_bool(lhs, "&&", scope)? {
                return Ok(Value::Bool(false));
            }
            Ok(Value::Bool(eval_bool(rhs, "&&", scope)?))
        }
        Expr::Binary {
            op: BinOp::Or,
            lhs,
            rhs,
            ..
        } => {
            if eval_bool(lhs, "||", scope)? {
                return Ok(Value::Bool(true));
            }
            Ok(Value::Bool(eval_bool(rhs, "||", scope)?))
        }
        Expr::Ternary {
            cond,
            then,
            otherwise,
            ..
        } => {
            if eval_bool(cond, "?:", scope)? {
                eval(then, scope)
            } else {
                eval(otherwise, scope)
            }
        }
        Expr::Fallback {
            primary, fallback, ..
        } => match eval_optional(primary, scope)? {
            Some(v) => Ok(v),
            None => eval(fallback, scope),
        },
        Expr::When(w) => eval_when(w, scope),
    }
}

pub fn eval_optional(expr: &Expr, scope: &dyn Scope) -> Result<Option<Value>, Error> {
    match expr {
        Expr::Path(ids, span) => eval_path(ids, *span, scope),
        _ => eval(expr, scope).map(Some),
    }
}

pub fn eval_all(exprs: &[Expr], scope: &dyn Scope) -> Result<Vec<Value>, Error> {
    exprs.iter().map(|e| eval(e, scope)).collect()
}

pub fn matches_pattern(
    pattern: &[Expr],
    subject: &[Value],
    scope: &dyn Scope,
) -> Result<bool, Error> {
    Ok(eval_all(pattern, scope)? == subject)
}

fn eval_path(ids: &[Ident], span: Span, scope: &dyn Scope) -> Result<Option<Value>, Error> {
    let root = ids[0].name.as_str();
    if RESERVED_ROOTS.contains(&root) {
        if ids.len() != 2 {
            return Err(Error::new(
                ErrorKind::Resolve,
                span,
                format!("`{root}` lookups take exactly one key, like `{root}.NAME`"),
            ));
        }
        let key = ids[1].name.as_str();
        let inputs = scope.inputs();
        let found = match root {
            "ctx" => inputs.ctx.get(key).cloned(),
            "env" => inputs.env.get(key),
            _ => inputs.secret.get(key),
        };
        return Ok(found.map(Value::Str));
    }
    let names: Vec<String> = ids.iter().map(|i| i.name.clone()).collect();
    scope.lookup(&names, span)
}

fn missing_error(ids: &[Ident], span: Span) -> Error {
    let key = ids.get(1).map(|i| i.name.as_str()).unwrap_or("");
    let message = match ids[0].name.as_str() {
        "ctx" => format!("ctx has no key `{key}`"),
        "env" => format!("environment variable `{key}` is not set and no fallback was given"),
        "secret" => format!("secret `{key}` is not available and no fallback was given"),
        _ => format!(
            "unknown key `{}`",
            ids.iter()
                .map(|i| i.name.as_str())
                .collect::<Vec<_>>()
                .join(".")
        ),
    };
    Error::new(ErrorKind::Resolve, span, message)
}

fn eval_string(segs: &[Segment], scope: &dyn Scope) -> Result<Value, Error> {
    if let [Segment::Interp(inner)] = segs {
        return eval(inner, scope);
    }
    let mut out = String::new();
    for seg in segs {
        match seg {
            Segment::Text(t) => out.push_str(t),
            Segment::Interp(e) => {
                let v = eval(e, scope)?;
                match v.interp_string() {
                    Some(s) => out.push_str(&s),
                    None => {
                        return Err(Error::new(
                            ErrorKind::Resolve,
                            e.span(),
                            format!("cannot interpolate a {} into a string", v.type_name()),
                        ))
                    }
                }
            }
        }
    }
    Ok(Value::Str(out))
}

fn eval_bool(expr: &Expr, op: &str, scope: &dyn Scope) -> Result<bool, Error> {
    match eval(expr, scope)? {
        Value::Bool(b) => Ok(b),
        other => Err(Error::new(
            ErrorKind::Resolve,
            expr.span(),
            format!("`{op}` requires a boolean, got {}", other.type_name()),
        )),
    }
}

fn eval_when(w: &WhenExpr, scope: &dyn Scope) -> Result<Value, Error> {
    let subject = eval_all(&w.subject, scope)?;
    for arm in &w.arms {
        if matches_pattern(&arm.pattern, &subject, scope)? {
            return eval(&arm.body, scope);
        }
    }
    match &w.else_arm {
        Some(e) => eval(e, scope),
        None => Err(Error::new(
            ErrorKind::Resolve,
            w.span,
            format!(
                "no `when` arm matched {} and there is no `else`",
                describe_values(&subject)
            ),
        )),
    }
}

pub fn describe_value(v: &Value) -> String {
    match v {
        Value::Str(s) => format!("{s:?}"),
        other => other
            .interp_string()
            .unwrap_or_else(|| other.type_name().to_string()),
    }
}

fn describe_values(vals: &[Value]) -> String {
    if vals.len() == 1 {
        return describe_value(&vals[0]);
    }
    format!(
        "({})",
        vals.iter()
            .map(describe_value)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Stmt;
    use crate::parser::parse_source;
    use crate::span::FileId;
    use std::collections::HashMap;

    struct TestScope {
        inputs: Inputs,
        values: HashMap<String, Value>,
    }

    impl Scope for TestScope {
        fn inputs(&self) -> &Inputs {
            &self.inputs
        }
        fn lookup(&self, path: &[String], _span: Span) -> Result<Option<Value>, Error> {
            Ok(self.values.get(&path.join(".")).cloned())
        }
    }

    fn scope() -> TestScope {
        let mut values = HashMap::new();
        values.insert("app.name".to_string(), Value::Str("Acme".into()));
        values.insert(
            "app.features".to_string(),
            Value::List(vec![Value::Str("a".into()), Value::Str("b".into())]),
        );
        values.insert("app.n".to_string(), Value::Int(3));
        values.insert("app.flag".to_string(), Value::Bool(true));
        let inputs = Inputs::new()
            .with_ctx("platform", "mobile")
            .with_env(HashMap::from([("HOST".to_string(), "h".to_string())]))
            .with_secret(HashMap::from([("KEY".to_string(), "k".to_string())]));
        TestScope { inputs, values }
    }

    fn ev(src: &str) -> Result<Value, Error> {
        let f = parse_source(&format!("x = {src}"), FileId(0))
            .unwrap_or_else(|e| panic!("parse: {}", e.message));
        let Stmt::Assign(a) = &f.statements[0] else {
            panic!()
        };
        eval(&a.value, &scope())
    }
    fn ok(src: &str) -> Value {
        ev(src).unwrap_or_else(|e| panic!("{src}: {}", e.message))
    }
    fn err(src: &str) -> String {
        ev(src).expect_err(src).message
    }
    fn s(x: &str) -> Value {
        Value::Str(x.into())
    }

    #[test]
    fn literals_and_lists() {
        assert_eq!(ok("null"), Value::Null);
        assert_eq!(
            ok(r#"[1, "a", true, 1.5]"#),
            Value::List(vec![
                Value::Int(1),
                s("a"),
                Value::Bool(true),
                Value::Float(1.5)
            ])
        );
    }

    #[test]
    fn config_paths() {
        assert_eq!(ok("app.name"), s("Acme"));
        assert_eq!(err("app.nope"), "unknown key `app.nope`");
    }

    #[test]
    fn ctx_env_and_secret_paths() {
        assert_eq!(ok("ctx.platform"), s("mobile"));
        assert_eq!(ok("env.HOST"), s("h"));
        assert_eq!(ok("secret.KEY"), s("k"));
        assert_eq!(err("ctx.tier"), "ctx has no key `tier`");
        assert_eq!(
            err("env.NOPE"),
            "environment variable `NOPE` is not set and no fallback was given"
        );
        assert_eq!(
            err("secret.NOPE"),
            "secret `NOPE` is not available and no fallback was given"
        );
        assert_eq!(
            err("ctx"),
            "`ctx` lookups take exactly one key, like `ctx.NAME`"
        );
        assert_eq!(
            err("env.A.B"),
            "`env` lookups take exactly one key, like `env.NAME`"
        );
    }

    #[test]
    fn fallback_and_default_consume_missing_lookups() {
        assert_eq!(ok(r#""${env.NOPE | "d"}""#), s("d"));
        assert_eq!(ok(r#""${env.HOST | "d"}""#), s("h"));
        assert_eq!(ok(r#""${app.nope | "d"}""#), s("d"));
        assert_eq!(ok(r#"default(env.NOPE, "d")"#), s("d"));
        assert_eq!(ok("default(app.n, 1)"), Value::Int(3));
        assert_eq!(ok("default(app.nope, 1)"), Value::Int(1));
        assert_eq!(
            err("default(secret.A, secret.B)"),
            "secret `B` is not available and no fallback was given"
        );
        assert_eq!(err("default(1)"), "default() expects 2 argument(s), got 1");
    }

    #[test]
    fn interpolation_stringifies_scalars_and_rejects_compounds() {
        assert_eq!(
            ok(r#""n=${app.n} f=${app.flag} p=${ctx.platform}""#),
            s("n=3 f=true p=mobile")
        );
        assert_eq!(
            err(r#""x${app.features}""#),
            "cannot interpolate a list into a string"
        );
        assert_eq!(
            err(r#""a${null}""#),
            "cannot interpolate a null into a string"
        );
    }

    #[test]
    fn lone_interpolation_yields_the_value_unchanged() {
        assert_eq!(
            ok(r#""${app.features}""#),
            Value::List(vec![s("a"), s("b")])
        );
        assert_eq!(ok(r#""${app.n}""#), Value::Int(3));
        assert_eq!(ok(r#""${null}""#), Value::Null);
    }

    #[test]
    fn equality_and_logic() {
        assert_eq!(ok("app.n == 3.0"), Value::Bool(true));
        assert_eq!(ok(r#"app.n == "3""#), Value::Bool(false));
        assert_eq!(ok("app.n != 3"), Value::Bool(false));
        assert_eq!(ok("true && false"), Value::Bool(false));
        assert_eq!(ok("false || true"), Value::Bool(true));
        assert_eq!(err("1 && true"), "`&&` requires a boolean, got number");
        assert_eq!(err("false || 1"), "`||` requires a boolean, got number");
    }

    #[test]
    fn logic_and_ternary_are_lazy() {
        assert_eq!(ok(r#"false && error("boom")"#), Value::Bool(false));
        assert_eq!(ok(r#"true || error("boom")"#), Value::Bool(true));
        assert_eq!(ok(r#"true ? 1 : error("boom")"#), Value::Int(1));
        assert_eq!(ok(r#"false ? error("boom") : 2"#), Value::Int(2));
        assert_eq!(err("1 ? 1 : 2"), "`?:` requires a boolean, got number");
    }

    #[test]
    fn when_expressions() {
        assert_eq!(
            ok(r#"when ctx.platform { "mobile" => 1  else => 2 }"#),
            Value::Int(1)
        );
        assert_eq!(
            ok(r#"when ctx.platform { "xr" => 1  else => 2 }"#),
            Value::Int(2)
        );
        assert_eq!(
            ok(r#"when ctx.platform { "mobile" => 1  "xr" => error("boom") }"#),
            Value::Int(1)
        );
        assert_eq!(
            ok(r#"when (ctx.platform, app.flag) { ("mobile", true) => "yes"  else => "no" }"#),
            s("yes")
        );
        assert_eq!(
            err(r#"when ctx.platform { "xr" => 1 }"#),
            "no `when` arm matched \"mobile\" and there is no `else`"
        );
        assert_eq!(
            err(r#"when (ctx.platform, app.n) { ("xr", 1) => 1 }"#),
            "no `when` arm matched (\"mobile\", 3) and there is no `else`"
        );
    }

    #[test]
    fn calls_and_pipes() {
        assert_eq!(ok("app.name |> upper"), s("ACME"));
        assert_eq!(ok(r#"app.features |> join(", ")"#), s("a, b"));
        assert_eq!(err(r#"error("boom ${ctx.platform}")"#), "boom mobile");
    }

    #[test]
    fn helpers_for_merge() {
        let sc = scope();
        let f = parse_source(
            r#"x = when (ctx.platform, 1) { ("mobile", 1) => 0 }"#,
            FileId(0),
        )
        .unwrap();
        let Stmt::Assign(a) = &f.statements[0] else {
            panic!()
        };
        let Expr::When(w) = &a.value else { panic!() };
        let subject = eval_all(&w.subject, &sc).unwrap();
        assert_eq!(subject, vec![s("mobile"), Value::Int(1)]);
        assert!(matches_pattern(&w.arms[0].pattern, &subject, &sc).unwrap());
        assert!(!matches_pattern(&w.arms[0].pattern, &[s("xr"), Value::Int(1)], &sc).unwrap());
    }
}
