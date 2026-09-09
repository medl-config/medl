use crate::error::{Error, ErrorKind};
use crate::span::Span;
use crate::value::Value;

pub fn call(name: &str, args: Vec<Value>, span: Span) -> Result<Value, Error> {
    let fail = |msg: String| Error::new(ErrorKind::Resolve, span, msg);
    let arity = |n: usize| -> Result<(), Error> {
        if args.len() == n {
            Ok(())
        } else {
            Err(fail(format!(
                "{name}() expects {n} argument(s), got {}",
                args.len()
            )))
        }
    };
    let str_arg = |i: usize| -> Result<&str, Error> {
        match &args[i] {
            Value::Str(s) => Ok(s.as_str()),
            other => Err(fail(format!(
                "{name}(): argument {} must be a string, got {}",
                i + 1,
                other.type_name()
            ))),
        }
    };
    let list_arg = |i: usize| -> Result<&Vec<Value>, Error> {
        match &args[i] {
            Value::List(l) => Ok(l),
            other => Err(fail(format!(
                "{name}(): argument {} must be a list, got {}",
                i + 1,
                other.type_name()
            ))),
        }
    };
    let int_arg = |i: usize| -> Result<i64, Error> {
        match &args[i] {
            Value::Int(n) => Ok(*n),
            other => Err(fail(format!(
                "{name}(): argument {} must be an integer, got {}",
                i + 1,
                other.type_name()
            ))),
        }
    };
    let str_result = |s: String| Ok(Value::Str(s));

    match name {
        "upper" => {
            arity(1)?;
            str_result(str_arg(0)?.to_uppercase())
        }
        "lower" => {
            arity(1)?;
            str_result(str_arg(0)?.to_lowercase())
        }
        "trim" => {
            arity(1)?;
            str_result(str_arg(0)?.trim().to_string())
        }
        "replace" => {
            arity(3)?;
            str_result(str_arg(0)?.replace(str_arg(1)?, str_arg(2)?))
        }
        "split" => {
            arity(2)?;
            let sep = str_arg(1)?;
            if sep.is_empty() {
                return Err(fail("split(): separator must not be empty".into()));
            }
            Ok(Value::List(
                str_arg(0)?
                    .split(sep)
                    .map(|p| Value::Str(p.to_string()))
                    .collect(),
            ))
        }
        "slice" => {
            arity(3)?;
            let chars: Vec<char> = str_arg(0)?.chars().collect();
            let (start, end) = (int_arg(1)?, int_arg(2)?);
            let valid = start >= 0 && end >= start && (end as usize) <= chars.len();
            if !valid {
                return Err(fail(format!(
                    "slice(): range {start}..{end} is out of bounds for a string of length {}",
                    chars.len()
                )));
            }
            str_result(chars[start as usize..end as usize].iter().collect())
        }
        "join" => {
            arity(2)?;
            let mut parts = Vec::new();
            for (i, item) in list_arg(0)?.iter().enumerate() {
                match item {
                    Value::Str(s) => parts.push(s.as_str()),
                    other => {
                        return Err(fail(format!(
                            "join(): list element {i} is a {}, not a string",
                            other.type_name()
                        )))
                    }
                }
            }
            str_result(parts.join(str_arg(1)?))
        }
        "len" => {
            arity(1)?;
            match &args[0] {
                Value::List(l) => Ok(Value::Int(l.len() as i64)),
                Value::Str(s) => Ok(Value::Int(s.chars().count() as i64)),
                other => Err(fail(format!(
                    "len(): argument 1 must be a list or string, got {}",
                    other.type_name()
                ))),
            }
        }
        "contains" => {
            arity(2)?;
            Ok(Value::Bool(list_arg(0)?.contains(&args[1])))
        }
        "first" => {
            arity(1)?;
            list_arg(0)?
                .first()
                .cloned()
                .ok_or_else(|| fail("first(): list is empty".into()))
        }
        "last" => {
            arity(1)?;
            list_arg(0)?
                .last()
                .cloned()
                .ok_or_else(|| fail("last(): list is empty".into()))
        }
        "snake_case" => {
            arity(1)?;
            str_result(
                split_words(str_arg(0)?)
                    .iter()
                    .map(|w| w.to_lowercase())
                    .collect::<Vec<_>>()
                    .join("_"),
            )
        }
        "kebab_case" => {
            arity(1)?;
            str_result(
                split_words(str_arg(0)?)
                    .iter()
                    .map(|w| w.to_lowercase())
                    .collect::<Vec<_>>()
                    .join("-"),
            )
        }
        "camel_case" => {
            arity(1)?;
            let words = split_words(str_arg(0)?);
            let mut out = String::new();
            for (i, w) in words.iter().enumerate() {
                if i == 0 {
                    out.push_str(&w.to_lowercase());
                } else {
                    out.push_str(&capitalize_word(w));
                }
            }
            str_result(out)
        }
        "pascal_case" => {
            arity(1)?;
            str_result(
                split_words(str_arg(0)?)
                    .iter()
                    .map(|w| capitalize_word(w))
                    .collect(),
            )
        }
        "capitalize" => {
            arity(1)?;
            let s = str_arg(0)?;
            let mut chars = s.chars();
            str_result(match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            })
        }
        "error" => {
            arity(1)?;
            Err(fail(str_arg(0)?.to_string()))
        }
        "default" => Err(fail(
            "default() must be evaluated lazily by eval, not builtins::call".into(),
        )),
        _ => Err(fail(format!("unknown function `{name}`"))),
    }
}

/// Lowercases the whole word, then uppercases its first character.
fn capitalize_word(w: &str) -> String {
    let lower = w.to_lowercase();
    let mut chars = lower.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Spec word-splitting rule: split on `_`, `-`, whitespace, lower→upper transitions,
/// and before the last capital of an uppercase run followed by lowercase.
pub fn split_words(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut words = Vec::new();
    let mut current = String::new();
    for i in 0..chars.len() {
        let c = chars[i];
        if c == '_' || c == '-' || c.is_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        if c.is_uppercase() && !current.is_empty() {
            let prev = chars[i - 1];
            let next_is_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            let boundary = prev.is_lowercase()
                || prev.is_ascii_digit()
                || (prev.is_uppercase() && next_is_lower);
            if boundary {
                words.push(std::mem::take(&mut current));
            }
        }
        current.push(c);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::{FileId, Span};

    fn sp() -> Span {
        Span::new(FileId(0), 0, 1)
    }
    fn s(x: &str) -> Value {
        Value::Str(x.into())
    }
    fn list(items: &[&str]) -> Value {
        Value::List(items.iter().map(|x| s(x)).collect())
    }
    fn ok(name: &str, args: Vec<Value>) -> Value {
        call(name, args, sp()).unwrap()
    }
    fn err(name: &str, args: Vec<Value>) -> String {
        call(name, args, sp()).unwrap_err().message
    }

    #[test]
    fn string_functions() {
        assert_eq!(ok("upper", vec![s("ab")]), s("AB"));
        assert_eq!(ok("lower", vec![s("AB")]), s("ab"));
        assert_eq!(ok("trim", vec![s("  a ")]), s("a"));
        assert_eq!(ok("replace", vec![s("1.4.0"), s("."), s("_")]), s("1_4_0"));
        assert_eq!(ok("split", vec![s("a,b"), s(",")]), list(&["a", "b"]));
        assert_eq!(
            ok("slice", vec![s("héllo"), Value::Int(1), Value::Int(3)]),
            s("él")
        );
        assert_eq!(ok("capitalize", vec![s("hello World")]), s("Hello World"));
    }

    #[test]
    fn split_rejects_an_empty_separator() {
        assert_eq!(
            err("split", vec![s("a,b"), s("")]),
            "split(): separator must not be empty"
        );
    }

    #[test]
    fn slice_rejects_out_of_range() {
        assert_eq!(
            err("slice", vec![s("abc"), Value::Int(2), Value::Int(9)]),
            "slice(): range 2..9 is out of bounds for a string of length 3"
        );
        assert_eq!(
            err("slice", vec![s("abc"), Value::Int(2), Value::Int(1)]),
            "slice(): range 2..1 is out of bounds for a string of length 3"
        );
    }

    #[test]
    fn list_functions() {
        assert_eq!(ok("join", vec![list(&["a", "b"]), s(", ")]), s("a, b"));
        assert_eq!(
            err("join", vec![Value::List(vec![Value::Int(1)]), s(",")]),
            "join(): list element 0 is a number, not a string"
        );
        assert_eq!(ok("len", vec![list(&["a", "b"])]), Value::Int(2));
        assert_eq!(ok("len", vec![s("héllo")]), Value::Int(5));
        assert_eq!(
            ok("contains", vec![list(&["a"]), s("a")]),
            Value::Bool(true)
        );
        assert_eq!(
            ok(
                "contains",
                vec![Value::List(vec![Value::Int(1)]), Value::Float(1.0)]
            ),
            Value::Bool(true)
        );
        assert_eq!(ok("first", vec![list(&["a", "b"])]), s("a"));
        assert_eq!(ok("last", vec![list(&["a", "b"])]), s("b"));
        assert_eq!(
            err("first", vec![Value::List(vec![])]),
            "first(): list is empty"
        );
        assert_eq!(
            err("last", vec![Value::List(vec![])]),
            "last(): list is empty"
        );
    }

    #[test]
    fn splits_words_per_spec() {
        let w = |x: &str| split_words(x);
        assert_eq!(w("HTTPServer"), vec!["HTTP", "Server"]);
        assert_eq!(w("service2Name"), vec!["service2", "Name"]);
        assert_eq!(w("Bright Sky Store"), vec!["Bright", "Sky", "Store"]);
        assert_eq!(
            w("snake_case-kebab camelCase"),
            vec!["snake", "case", "kebab", "camel", "Case"]
        );
        assert_eq!(w(""), Vec::<String>::new());
    }

    #[test]
    fn case_conversions() {
        assert_eq!(
            ok("snake_case", vec![s("Bright Sky Store")]),
            s("bright_sky_store")
        );
        assert_eq!(ok("kebab_case", vec![s("HTTPServer")]), s("http-server"));
        assert_eq!(ok("camel_case", vec![s("bright sky")]), s("brightSky"));
        assert_eq!(
            ok("pascal_case", vec![s("Bright Sky Expo")]),
            s("BrightSkyExpo")
        );
    }

    #[test]
    fn error_builtin_reports_message() {
        assert_eq!(
            err("error", vec![s("unsupported platform")]),
            "unsupported platform"
        );
    }

    #[test]
    fn arity_and_type_errors() {
        assert_eq!(err("upper", vec![]), "upper() expects 1 argument(s), got 0");
        assert_eq!(
            err("upper", vec![Value::Int(1)]),
            "upper(): argument 1 must be a string, got number"
        );
        assert_eq!(
            err("len", vec![Value::Int(1)]),
            "len(): argument 1 must be a list or string, got number"
        );
        assert_eq!(err("nope", vec![]), "unknown function `nope`");
    }
}
