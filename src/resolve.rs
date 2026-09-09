use crate::ast::AssignOp;
use crate::error::{Error, ErrorKind, Warning};
use crate::eval::{self, Scope};
use crate::graph::KeyPath;
use crate::inputs::Inputs;
use crate::merge::{lookup_leaf, Merged, MergedMap, Node};
use crate::span::Span;
use crate::value::Value;
use indexmap::IndexMap;
use std::collections::HashMap;

#[derive(Debug)]
pub struct Resolved {
    pub value: Value,
    pub warnings: Vec<Warning>,
}

pub fn resolve(merged: &Merged, order: &[KeyPath], inputs: &Inputs) -> Result<Resolved, Error> {
    let mut values: HashMap<KeyPath, Value> = HashMap::new();
    let mut warnings = Vec::new();
    for key in order {
        let ops = lookup_leaf(&merged.root, key).expect("order lists leaves only");
        let name = key.join(".");
        let mut acc: Option<Value> = None;
        for op in ops {
            let scope = TreeScope {
                inputs,
                root: &merged.root,
                values: &values,
            };
            let rhs = eval::eval(&op.expr, &scope)
                .map_err(|e| e.label(op.span, format!("while resolving `{name}`")))?;
            acc = fold_op(acc, op.op, rhs, &name, op.span, &mut warnings)?;
        }
        if let Some(v) = acc {
            values.insert(key.clone(), v);
        }
    }
    let value = Value::Map(materialize(&merged.root, &mut Vec::new(), &values));
    Ok(Resolved { value, warnings })
}

/// Spec section 3 operator table.
fn fold_op(
    acc: Option<Value>,
    op: AssignOp,
    rhs: Value,
    name: &str,
    span: Span,
    warnings: &mut Vec<Warning>,
) -> Result<Option<Value>, Error> {
    let fail = |msg: String| Error::new(ErrorKind::Resolve, span, msg);
    match op {
        AssignOp::Set => Ok(Some(rhs)),
        AssignOp::Append => match (acc, rhs) {
            (None, rhs) => Ok(Some(rhs)),
            (Some(Value::List(mut items)), Value::List(more)) => {
                items.extend(more);
                Ok(Some(Value::List(items)))
            }
            (Some(Value::List(_)), other) => Err(fail(format!(
                "cannot `+=` a {} to `{name}`: the right-hand side must be a list",
                other.type_name()
            ))),
            (Some(other), _) => Err(fail(format!(
                "cannot `+=` to `{name}`: it holds a {}, not a list",
                other.type_name()
            ))),
        },
        AssignOp::Remove => match (acc, rhs) {
            (None, _) => {
                warnings.push(Warning {
                    span,
                    message: format!("`-=` on `{name}` removed nothing: the key had no value yet"),
                });
                Ok(None)
            }
            (Some(Value::List(mut items)), Value::List(remove)) => {
                let before = items.len();
                items.retain(|item| !remove.contains(item));
                if items.len() == before {
                    warnings.push(Warning { span, message: format!("`-=` on `{name}` removed nothing: none of the listed values were present") });
                }
                Ok(Some(Value::List(items)))
            }
            (Some(Value::List(_)), other) => Err(fail(format!(
                "cannot `-=` a {} from `{name}`: the right-hand side must be a list",
                other.type_name()
            ))),
            (Some(other), _) => Err(fail(format!(
                "cannot `-=` from `{name}`: it holds a {}, not a list",
                other.type_name()
            ))),
        },
    }
}

struct TreeScope<'a> {
    inputs: &'a Inputs,
    root: &'a MergedMap,
    values: &'a HashMap<KeyPath, Value>,
}

impl Scope for TreeScope<'_> {
    fn inputs(&self) -> &Inputs {
        self.inputs
    }

    fn lookup(&self, path: &[String], span: Span) -> Result<Option<Value>, Error> {
        let mut map = self.root;
        for (i, key) in path.iter().enumerate() {
            match map.get(key) {
                None => return Ok(None),
                Some(Node::Leaf(_)) => {
                    let leaf = &path[..=i];
                    let value = self.values.get(leaf);
                    if i + 1 == path.len() {
                        return Ok(value.cloned());
                    }
                    // A leaf may hold a map (`t = "${s}"`); the rest of the path indexes into the value.
                    if let Some(Value::Map(_)) = value {
                        return index_value(value.expect("matched above"), path, i + 1, span);
                    }
                    let held = value
                        .map(|v| format!("a {}", v.type_name()))
                        .unwrap_or_else(|| "unset".to_string());
                    return Err(Error::new(
                        ErrorKind::Resolve,
                        span,
                        format!(
                            "cannot index into `{}` with `.{}`: `{}` is {held}, not a map",
                            leaf.join("."),
                            path[i + 1..].join("."),
                            leaf.join(".")
                        ),
                    ));
                }
                Some(Node::Map(m)) => map = m,
            }
        }
        Ok(Some(Value::Map(materialize(
            map,
            &mut path.to_vec(),
            self.values,
        ))))
    }
}

/// Walk `path[from..]` through an already-resolved value. A missing key is `Ok(None)`, so the
/// caller reports it the same way as any unknown key; a non-map on the way is an error.
fn index_value(
    value: &Value,
    path: &[String],
    from: usize,
    span: Span,
) -> Result<Option<Value>, Error> {
    let mut current = value;
    for j in from..path.len() {
        match current {
            Value::Map(m) => match m.get(&path[j]) {
                Some(v) => current = v,
                None => return Ok(None),
            },
            other => {
                return Err(Error::new(
                    ErrorKind::Resolve,
                    span,
                    format!(
                        "cannot index into `{}` with `.{}`: `{}` is a {}, not a map",
                        path[..j].join("."),
                        path[j..].join("."),
                        path[..j].join("."),
                        other.type_name()
                    ),
                ))
            }
        }
    }
    Ok(Some(current.clone()))
}

/// Output map in declaration order. Leaves without a value are skipped; declared maps always appear.
fn materialize(
    map: &MergedMap,
    prefix: &mut Vec<String>,
    values: &HashMap<KeyPath, Value>,
) -> IndexMap<String, Value> {
    let mut out = IndexMap::new();
    for (key, node) in map {
        prefix.push(key.clone());
        match node {
            Node::Leaf(_) => {
                if let Some(v) = values.get(prefix) {
                    out.insert(key.clone(), v.clone());
                }
            }
            Node::Map(m) => {
                out.insert(key.clone(), Value::Map(materialize(m, prefix, values)));
            }
        }
        prefix.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::test_util::{resolve_files, resolve_src};
    use serde_json::json;

    fn out(src: &str) -> serde_json::Value {
        resolve_src(src)
            .unwrap_or_else(|e| panic!("{}", e.message))
            .value
            .to_json()
    }
    fn err(src: &str) -> Error {
        resolve_src(src).expect_err("expected error")
    }

    #[test]
    fn scalars_and_nested_maps_keep_declaration_order() {
        let v = out("b = 2\na { y = 1  x = \"v${b}\" }");
        assert_eq!(
            serde_json::to_string(&v).unwrap(),
            r#"{"b":2,"a":{"y":1,"x":"v2"}}"#
        );
    }

    #[test]
    fn list_ops_fold_in_order() {
        assert_eq!(
            out("f = [\"a\", \"b\"]\nf -= [\"b\"]\nf += [\"c\", \"d\"]"),
            json!({"f": ["a", "c", "d"]})
        );
    }

    #[test]
    fn append_on_undefined_sets_and_remove_on_undefined_omits_the_key() {
        assert_eq!(out("f += [1]\ng -= [1]"), json!({"f": [1]}));
    }

    #[test]
    fn declared_blocks_always_appear() {
        assert_eq!(out("a { }\nb { c { } }"), json!({"a": {}, "b": {"c": {}}}));
    }

    #[test]
    fn list_op_type_mismatches_error() {
        assert_eq!(
            err("f = 1\nf += [2]").message,
            "cannot `+=` to `f`: it holds a number, not a list"
        );
        assert_eq!(
            err("f = [1]\nf += 2").message,
            "cannot `+=` a number to `f`: the right-hand side must be a list"
        );
        assert_eq!(
            err("f = \"s\"\nf -= [1]").message,
            "cannot `-=` from `f`: it holds a string, not a list"
        );
        assert_eq!(
            err("f = [1]\nf -= 1").message,
            "cannot `-=` a number from `f`: the right-hand side must be a list"
        );
    }

    #[test]
    fn remove_matching_nothing_warns() {
        let r = resolve_src("f = [1]\nf -= [2]").unwrap();
        assert_eq!(r.value.to_json(), json!({"f": [1]}));
        assert_eq!(r.warnings.len(), 1);
        assert_eq!(
            r.warnings[0].message,
            "`-=` on `f` removed nothing: none of the listed values were present"
        );
        assert_eq!(r.warnings[0].span.start, 8);
    }

    #[test]
    fn remove_on_an_undefined_key_warns() {
        let r = resolve_src("g -= [1]\nf = 1").unwrap();
        assert_eq!(r.value.to_json(), json!({"f": 1}));
        assert_eq!(r.warnings.len(), 1);
        assert_eq!(
            r.warnings[0].message,
            "`-=` on `g` removed nothing: the key had no value yet"
        );
        assert_eq!(r.warnings[0].span.start, 0);
    }

    #[test]
    fn remove_before_a_later_append_still_warns_without_claiming_the_key_is_undefined() {
        let r = resolve_src("g -= [1]\ng += [2]").unwrap();
        assert_eq!(r.value.to_json(), json!({"g": [2]}));
        assert_eq!(r.warnings.len(), 1);
        assert_eq!(
            r.warnings[0].message,
            "`-=` on `g` removed nothing: the key had no value yet"
        );
    }

    #[test]
    fn errors_are_labelled_with_the_key_being_resolved() {
        let e = err("app { a = error(\"boom\") }");
        assert_eq!(e.kind, ErrorKind::Resolve);
        assert_eq!(e.message, "boom");
        assert_eq!(e.labels[0].message, "while resolving `app.a`");
    }

    #[test]
    fn maps_are_values_and_indexing_into_scalars_errors() {
        assert_eq!(
            out("s { x = 1 }\nt = \"${s}\""),
            json!({"s": {"x": 1}, "t": {"x": 1}})
        );
        assert_eq!(
            err("a = 1\nb = \"${a.c}\"").message,
            "cannot index into `a` with `.c`: `a` is a number, not a map"
        );
        assert_eq!(
            err("a -= [1]\nb = \"${a.c}\"").message,
            "cannot index into `a` with `.c`: `a` is unset, not a map"
        );
        assert_eq!(err("a -= [1]\nb = \"${a}\"").message, "unknown key `a`");
    }

    #[test]
    fn map_valued_leaves_can_be_indexed() {
        assert_eq!(
            out("s { x = 1 }\nt = \"${s}\"\nu = \"${t.x}\""),
            json!({"s": {"x": 1}, "t": {"x": 1}, "u": 1})
        );
        assert_eq!(
            out("s { x { y = 1 } }\nt = \"${s}\"\nu = \"${t.x.y}\""),
            json!({"s": {"x": {"y": 1}}, "t": {"x": {"y": 1}}, "u": 1})
        );
        assert_eq!(
            err("s { x = 1 }\nt = \"${s}\"\nu = \"${t.nope}\"").message,
            "unknown key `t.nope`"
        );
        assert_eq!(
            err("s { x = 1 }\nt = \"${s}\"\nu = \"${t.x.y}\"").message,
            "cannot index into `t.x` with `.y`: `t.x` is a number, not a map"
        );
    }

    #[test]
    fn inputs_flow_through() {
        let inputs = Inputs::new().with_ctx("platform", "mobile");
        let (r, _) = resolve_files(
            &[(
                "main.medl",
                "p = ctx.platform\nh = \"${env.HOST | \"local\"}\"",
            )],
            &inputs,
        )
        .unwrap();
        assert_eq!(r.value.to_json(), json!({"p": "mobile", "h": "local"}));
    }

    #[test]
    fn value_position_when_without_match_errors_at_resolve_time() {
        let e = err("p = when 1 { 2 => \"x\" }");
        assert_eq!(e.message, "no `when` arm matched 1 and there is no `else`");
    }
}
