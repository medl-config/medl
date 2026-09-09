use crate::ast::{AssignOp, Expr, RESERVED_ROOTS};
use crate::error::Warning;
use crate::merge::{lookup_leaf, Merged, MergedMap, Node};

/// Spec section 3 `--strict` warnings that can be decided from the merged tree alone.
pub fn lint(merged: &Merged) -> Vec<Warning> {
    let mut warnings = Vec::new();
    dead_required_markers(merged, &mut warnings);
    dead_fallbacks(&merged.root, &mut warnings);
    warnings
}

fn dead_required_markers(merged: &Merged, out: &mut Vec<Warning>) {
    for decl in &merged.required {
        let Some(ops) = lookup_leaf(&merged.root, &decl.path) else {
            continue;
        };
        let filled_here = ops.iter().any(|o| {
            matches!(o.op, AssignOp::Set | AssignOp::Append)
                && !o.conditional
                && o.span.file == decl.span.file
        });
        if filled_here {
            out.push(Warning {
                span: decl.span,
                message: format!("`{}` is declared required here but the same file always assigns it; the marker is dead", decl.path.join(".")),
            });
        }
    }
}

fn dead_fallbacks(map: &MergedMap, out: &mut Vec<Warning>) {
    for node in map.values() {
        match node {
            Node::Map(m) => dead_fallbacks(m, out),
            Node::Leaf(ops) => {
                for op in ops {
                    op.expr.walk(&mut |e| {
                        if let Expr::Fallback { primary, span, .. } = e {
                            let is_input_lookup = matches!(&**primary, Expr::Path(ids, _) if RESERVED_ROOTS.contains(&ids[0].name.as_str()));
                            if !is_input_lookup {
                                out.push(Warning {
                                    span: *span,
                                    message: "`|` fallback is dead code: the left side is not a ctx, env, or secret lookup".to_string(),
                                });
                            }
                        }
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inputs::Inputs;
    use crate::test_util::{merge_files, merge_src};

    fn messages(files: &[(&str, &str)]) -> Vec<String> {
        lint(&merge_src(files).unwrap())
            .into_iter()
            .map(|w| w.message)
            .collect()
    }

    #[test]
    fn required_with_unconditional_fill_in_same_file_is_dead() {
        let m = messages(&[("main.medl", "app {\n  required sku\n  sku = \"X\"\n}")]);
        assert_eq!(m, vec!["`app.sku` is declared required here but the same file always assigns it; the marker is dead"]);
        let m = merge_src(&[("main.medl", "app {\n  required sku\n  sku = \"X\"\n}")]).unwrap();
        assert_eq!(lint(&m)[0].span.start, 8);
    }

    #[test]
    fn required_filled_elsewhere_or_conditionally_is_not_dead() {
        assert!(messages(&[
            ("main.medl", "extends(\"base.medl\")\napp { sku = \"X\" }"),
            ("base.medl", "app { required sku }"),
        ])
        .is_empty());
        let inputs = Inputs::new().with_ctx("v", "a");
        let (m, _) = merge_files(
            &[(
                "main.medl",
                "app { required sku\nwhen ctx.v { \"a\" => { sku = \"X\" } } }",
            )],
            &inputs,
        )
        .unwrap();
        assert!(lint(&m).is_empty());
    }

    #[test]
    fn fallback_on_config_path_is_dead_but_input_lookups_are_fine() {
        let m = messages(&[("main.medl", "a = 1\nb = \"${a | 2}\"\nc = \"${env.X | 2}\"\nd = \"${ctx.y | 2}\"\ne = \"${secret.z | 2}\"")]);
        assert_eq!(
            m,
            vec!["`|` fallback is dead code: the left side is not a ctx, env, or secret lookup"]
        );
    }

    #[test]
    fn fallbacks_inside_when_arms_are_linted() {
        let m = messages(&[("main.medl", "a = 1\nb = when 1 { 1 => \"${a | 2}\" }")]);
        assert_eq!(m.len(), 1);
    }
}
