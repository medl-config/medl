use crate::ast::*;
use crate::error::{Error, ErrorKind};
use crate::eval::{self, Scope};
use crate::inputs::Inputs;
use crate::loader::{load_file, normalize, SourceLoader};
use crate::span::{SourceMap, Span};
use crate::value::Value;
use indexmap::IndexMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub type MergedMap = IndexMap<String, Node>;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Leaf(Vec<OpEntry>),
    Map(MergedMap),
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpEntry {
    pub op: AssignOp,
    pub expr: Expr,
    pub span: Span,
    /// True when the assignment sits inside a block-position `when` arm.
    pub conditional: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequiredDecl {
    pub path: Vec<String>,
    pub message: Option<String>,
    pub span: Span,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Merged {
    pub root: MergedMap,
    pub required: Vec<RequiredDecl>,
}

pub fn merge(
    entry: &Path,
    loader: &dyn SourceLoader,
    sources: &mut SourceMap,
    inputs: &Inputs,
) -> Result<Merged, Error> {
    let mut merger = Merger {
        loader,
        sources,
        inputs,
        chain: Vec::new(),
        included: HashSet::new(),
    };
    let mut merged = Merged::default();
    merger.merge_file(&normalize(entry), None, &mut merged)?;
    Ok(merged)
}

pub fn op_symbol(op: AssignOp) -> &'static str {
    match op {
        AssignOp::Set => "=",
        AssignOp::Append => "+=",
        AssignOp::Remove => "-=",
    }
}

pub fn lookup_node<'a>(root: &'a MergedMap, path: &[String]) -> Option<&'a Node> {
    let (first, rest) = path.split_first()?;
    let mut node = root.get(first)?;
    for key in rest {
        match node {
            Node::Map(m) => node = m.get(key)?,
            Node::Leaf(_) => return None,
        }
    }
    Some(node)
}

pub fn lookup_leaf<'a>(root: &'a MergedMap, path: &[String]) -> Option<&'a [OpEntry]> {
    match lookup_node(root, path)? {
        Node::Leaf(ops) => Some(ops),
        Node::Map(_) => None,
    }
}

struct Merger<'a> {
    loader: &'a dyn SourceLoader,
    sources: &'a mut SourceMap,
    inputs: &'a Inputs,
    /// Files currently being merged, outermost first. Used for extends-cycle detection.
    chain: Vec<PathBuf>,
    /// Every file already merged in this resolution, including the entry file.
    included: HashSet<PathBuf>,
}

#[derive(Clone)]
struct Cursor {
    /// Directory of the current file; `extends` paths resolve against it.
    dir: PathBuf,
    /// Enclosing block names.
    prefix: Vec<String>,
    /// Inside a block-position `when` arm.
    conditional: bool,
}

impl<'a> Merger<'a> {
    fn merge_file(
        &mut self,
        path: &Path,
        from: Option<Span>,
        merged: &mut Merged,
    ) -> Result<(), Error> {
        if let Some(i) = self.chain.iter().position(|p| p == path) {
            let mut names: Vec<String> = self.chain[i..]
                .iter()
                .map(|p| p.display().to_string())
                .collect();
            names.push(path.display().to_string());
            let message = format!("extends cycle: {}", names.join(" -> "));
            return Err(match from {
                Some(span) => Error::new(ErrorKind::Load, span, message),
                None => Error::spanless(ErrorKind::Load, message),
            });
        }
        // A file is merged at most once: a diamond or a repeated `extends` is a no-op after
        // the first inclusion, whose position in visit order is where its statements apply.
        if !self.included.insert(path.to_path_buf()) {
            return Ok(());
        }
        let file = load_file(path, self.loader, self.sources).map_err(|mut e| {
            if e.span.is_none() {
                e.span = from;
            }
            e
        })?;
        self.chain.push(path.to_path_buf());
        let cursor = Cursor {
            dir: path.parent().map(Path::to_path_buf).unwrap_or_default(),
            prefix: Vec::new(),
            conditional: false,
        };
        let result = self.merge_stmts(&file.statements, &cursor, merged);
        self.chain.pop();
        result
    }

    fn merge_stmts(
        &mut self,
        stmts: &[Stmt],
        cursor: &Cursor,
        merged: &mut Merged,
    ) -> Result<(), Error> {
        for stmt in stmts {
            match stmt {
                Stmt::Extends(e) => {
                    for (rel, span) in &e.paths {
                        let target = normalize(&cursor.dir.join(rel));
                        self.merge_file(&target, Some(*span), merged)?;
                    }
                }
                Stmt::Block(b) => {
                    let mut child = cursor.clone();
                    child.prefix.push(b.name.name.clone());
                    ensure_map(&mut merged.root, &child.prefix, b.name.span)?;
                    self.merge_stmts(&b.body, &child, merged)?;
                }
                Stmt::Assign(a) => {
                    apply_assign(&mut merged.root, &cursor.prefix, a, cursor.conditional)?
                }
                Stmt::Required(r) => {
                    let mut path = cursor.prefix.clone();
                    path.extend(r.path.iter().map(|i| i.name.clone()));
                    merged.required.push(RequiredDecl {
                        path,
                        message: r.message.clone(),
                        span: r.span,
                    });
                }
                Stmt::When(w) => {
                    if let Some(body) = self.select_arm(w)? {
                        let mut child = cursor.clone();
                        child.conditional = true;
                        self.merge_stmts(body, &child, merged)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn select_arm<'s>(&self, w: &'s WhenStmt) -> Result<Option<&'s [Stmt]>, Error> {
        // Validate every subject and pattern up front, so an offending pattern in a later
        // arm is rejected even when an earlier arm matches.
        for e in &w.subject {
            check_inputs_only(e)?;
        }
        for arm in &w.arms {
            for e in &arm.pattern {
                check_inputs_only(e)?;
            }
        }
        let scope = InputScope {
            inputs: self.inputs,
        };
        let subject = eval::eval_all(&w.subject, &scope)?;
        for arm in &w.arms {
            if eval::matches_pattern(&arm.pattern, &subject, &scope)? {
                return Ok(Some(&arm.body));
            }
        }
        Ok(w.else_arm.as_deref())
    }
}

/// Scope for merge-time evaluation: only ctx/env/secret exist.
struct InputScope<'a> {
    inputs: &'a Inputs,
}

impl Scope for InputScope<'_> {
    fn inputs(&self) -> &Inputs {
        self.inputs
    }
    fn lookup(&self, path: &[String], span: Span) -> Result<Option<Value>, Error> {
        Err(config_path_error(&path.join("."), span))
    }
}

fn config_path_error(path: &str, span: Span) -> Error {
    Error::new(ErrorKind::Merge, span, format!(
        "block-position `when` may only reference ctx, env, or secret; `{path}` is a config value (use a value-position `when` instead)"))
}

fn check_inputs_only(expr: &Expr) -> Result<(), Error> {
    let mut offending: Option<(String, Span)> = None;
    expr.walk(&mut |e| {
        if let Expr::Path(ids, span) = e {
            if offending.is_none() && !RESERVED_ROOTS.contains(&ids[0].name.as_str()) {
                offending = Some((
                    ids.iter()
                        .map(|i| i.name.as_str())
                        .collect::<Vec<_>>()
                        .join("."),
                    *span,
                ));
            }
        }
    });
    match offending {
        Some((path, span)) => Err(config_path_error(&path, span)),
        None => Ok(()),
    }
}

/// Spec section 2: every `required` key must be filled by `=`, `+=`, or a block with a productive statement.
pub fn check_required(merged: &Merged) -> Result<(), Error> {
    let mut unfilled: IndexMap<String, Vec<&RequiredDecl>> = IndexMap::new();
    for decl in &merged.required {
        if !is_filled(&merged.root, &decl.path) {
            unfilled.entry(decl.path.join(".")).or_default().push(decl);
        }
    }
    if unfilled.is_empty() {
        return Ok(());
    }
    let keys: Vec<&str> = unfilled.keys().map(String::as_str).collect();
    let first_span = unfilled.values().next().unwrap()[0].span;
    let mut err = Error::new(
        ErrorKind::Required,
        first_span,
        format!(
            "{} required key(s) were never filled: {}",
            unfilled.len(),
            keys.join(", ")
        ),
    );
    for (key, decls) in &unfilled {
        for decl in decls {
            let label = match &decl.message {
                Some(msg) => format!("`{key}` declared required here: {msg}"),
                None => format!("`{key}` declared required here"),
            };
            err = err.label(decl.span, label);
        }
    }
    Err(err)
}

fn is_filled(root: &MergedMap, path: &[String]) -> bool {
    lookup_node(root, path).is_some_and(node_is_productive)
}

fn node_is_productive(node: &Node) -> bool {
    match node {
        Node::Leaf(ops) => ops
            .iter()
            .any(|o| matches!(o.op, AssignOp::Set | AssignOp::Append)),
        Node::Map(m) => m.values().any(node_is_productive),
    }
}

/// Walk/create the map at `path`. A leaf on the way is an error.
fn ensure_map<'m>(
    root: &'m mut MergedMap,
    path: &[String],
    span: Span,
) -> Result<&'m mut MergedMap, Error> {
    let mut map = root;
    for (i, key) in path.iter().enumerate() {
        let node = map
            .entry(key.clone())
            .or_insert_with(|| Node::Map(MergedMap::new()));
        match node {
            Node::Map(m) => map = m,
            Node::Leaf(ops) => {
                let assigned = ops.last().expect("leaf has at least one op").span;
                return Err(Error::new(
                    ErrorKind::Merge,
                    span,
                    format!(
                        "cannot open block `{}`: it already holds a value",
                        path[..=i].join(".")
                    ),
                )
                .label(assigned, "value assigned here"));
            }
        }
    }
    Ok(map)
}

fn apply_assign(
    root: &mut MergedMap,
    prefix: &[String],
    a: &AssignStmt,
    conditional: bool,
) -> Result<(), Error> {
    let map = ensure_map(root, prefix, a.name.span)?;
    let entry = OpEntry {
        op: a.op,
        expr: a.value.clone(),
        span: a.span,
        conditional,
    };
    if !map.contains_key(&a.name.name) {
        map.insert(a.name.name.clone(), Node::Leaf(vec![entry]));
        return Ok(());
    }
    let node = map.get_mut(&a.name.name).expect("checked above");
    match node {
        Node::Leaf(ops) => {
            if a.op == AssignOp::Set {
                ops.clear();
            }
            ops.push(entry);
        }
        Node::Map(_) => {
            if a.op == AssignOp::Set {
                *node = Node::Leaf(vec![entry]);
            } else {
                let full: Vec<&str> = prefix
                    .iter()
                    .map(String::as_str)
                    .chain([a.name.name.as_str()])
                    .collect();
                return Err(Error::new(
                    ErrorKind::Merge,
                    a.span,
                    format!(
                        "cannot apply `{}` to `{}`: it is a block, not a list",
                        op_symbol(a.op),
                        full.join(".")
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::test_util::{keypath, merge_src};

    fn leaf<'a>(m: &'a Merged, path: &str) -> &'a [OpEntry] {
        lookup_leaf(&m.root, &keypath(path)).unwrap_or_else(|| panic!("no leaf `{path}`"))
    }
    fn ops(m: &Merged, path: &str) -> Vec<AssignOp> {
        leaf(m, path).iter().map(|o| o.op).collect()
    }
    fn ok(files: &[(&str, &str)]) -> Merged {
        merge_src(files).unwrap_or_else(|e| panic!("{}", e.message))
    }
    fn err(files: &[(&str, &str)]) -> Error {
        merge_src(files).expect_err("expected merge error")
    }

    #[test]
    fn single_assignment_becomes_leaf_with_one_set_op() {
        let m = ok(&[("main.medl", r#"app { name = "x" }"#)]);
        assert_eq!(ops(&m, "app.name"), vec![AssignOp::Set]);
        assert!(!leaf(&m, "app.name")[0].conditional);
        assert!(matches!(leaf(&m, "app.name")[0].expr, Expr::Str(..)));
    }

    #[test]
    fn blocks_deep_merge_across_extends_in_declaration_order() {
        let m = ok(&[
            (
                "main.medl",
                "extends(\"base.medl\")\napp { build { b = 2 } }",
            ),
            ("base.medl", "app { build { a = 1 } }\nother = 0"),
        ]);
        let root_keys: Vec<&String> = m.root.keys().collect();
        assert_eq!(root_keys, vec!["app", "other"]);
        let Some(Node::Map(app)) = m.root.get("app") else {
            panic!()
        };
        let Some(Node::Map(build)) = app.get("build") else {
            panic!()
        };
        assert_eq!(build.keys().collect::<Vec<_>>(), vec!["a", "b"]);
    }

    #[test]
    fn op_chain_accumulates_in_visit_order() {
        let m = ok(&[
            ("main.medl", "extends(\"base.medl\")\nf += [2]\nf -= [1]"),
            ("base.medl", "f = [1]"),
        ]);
        assert_eq!(
            ops(&m, "f"),
            vec![AssignOp::Set, AssignOp::Append, AssignOp::Remove]
        );
    }

    #[test]
    fn set_resets_the_op_chain() {
        let m = ok(&[("main.medl", "f = [1]\nf += [2]\nf = [9]")]);
        assert_eq!(ops(&m, "f"), vec![AssignOp::Set]);
        assert!(
            matches!(&leaf(&m, "f")[0].expr, Expr::List(items, _) if matches!(items[0], Expr::Int(9, _)))
        );
    }

    #[test]
    fn append_or_remove_on_undefined_key_starts_a_chain() {
        let m = ok(&[("main.medl", "f -= [1]")]);
        assert_eq!(ops(&m, "f"), vec![AssignOp::Remove]);
    }

    #[test]
    fn set_replaces_block_but_append_on_block_errors() {
        let m = ok(&[("main.medl", "a { x = 1 }\na = 2")]);
        assert_eq!(ops(&m, "a"), vec![AssignOp::Set]);
        let e = err(&[("main.medl", "a { x = 1 }\na += [1]")]);
        assert_eq!(e.kind, ErrorKind::Merge);
        assert_eq!(
            e.message,
            "cannot apply `+=` to `a`: it is a block, not a list"
        );
    }

    #[test]
    fn block_over_scalar_errors_with_label() {
        let e = err(&[("main.medl", "a = 1\na { x = 2 }")]);
        assert_eq!(e.message, "cannot open block `a`: it already holds a value");
        assert_eq!(e.labels.len(), 1);
        assert_eq!(e.labels[0].message, "value assigned here");
        assert_eq!(e.labels[0].span.start, 0);
    }

    #[test]
    fn extends_resolves_relative_to_including_file_and_merges_at_root() {
        let m = ok(&[
            (
                "sub/main.medl",
                "outer { extends(\"../base.medl\", \"inner/x.medl\") }",
            ),
            ("base.medl", "a = 1"),
            ("sub/inner/x.medl", "b = 2"),
        ]);
        assert_eq!(ops(&m, "a"), vec![AssignOp::Set]);
        assert_eq!(ops(&m, "b"), vec![AssignOp::Set]);
        assert!(matches!(m.root.get("outer"), Some(Node::Map(_))));
    }

    #[test]
    fn missing_extends_file_is_reported_at_the_extends_span() {
        let e = err(&[("main.medl", "extends(\"nope.medl\")")]);
        assert_eq!(e.kind, ErrorKind::Load);
        assert_eq!(e.message, "cannot read `nope.medl`: no such file");
        assert_eq!(e.span.unwrap().start, 8);
    }

    #[test]
    fn a_file_is_merged_at_most_once_per_resolution() {
        // Diamond: main -> b -> base and main -> c -> base.
        let m = ok(&[
            ("main.medl", "extends(\"b.medl\", \"c.medl\")"),
            ("b.medl", "extends(\"base.medl\")"),
            ("c.medl", "extends(\"base.medl\")"),
            ("base.medl", "f += [\"x\"]"),
        ]);
        assert_eq!(ops(&m, "f"), vec![AssignOp::Append]);
        // The same file named twice by sequential `extends` statements.
        let m = ok(&[
            (
                "main.medl",
                "extends(\"base.medl\")\nextends(\"./base.medl\")",
            ),
            ("base.medl", "f += [\"x\"]"),
        ]);
        assert_eq!(ops(&m, "f"), vec![AssignOp::Append]);
    }

    #[test]
    fn extends_cycle_is_reported_with_chain() {
        let e = err(&[
            ("a.medl", "extends(\"b.medl\")"),
            ("b.medl", "extends(\"a.medl\")"),
        ]);
        assert_eq!(e.message, "extends cycle: a.medl -> b.medl -> a.medl");
    }

    #[test]
    fn required_is_collected_with_absolute_path() {
        let m = ok(&[(
            "main.medl",
            "app { required session.app_id \"msg\"\nrequired sku }",
        )]);
        assert_eq!(m.required.len(), 2);
        assert_eq!(m.required[0].path, keypath("app.session.app_id"));
        assert_eq!(m.required[0].message.as_deref(), Some("msg"));
        assert_eq!(m.required[1].path, keypath("app.sku"));
    }

    #[test]
    fn lookup_helpers_distinguish_maps_and_leaves() {
        let m = ok(&[("main.medl", "a { b = 1 }")]);
        assert!(matches!(
            lookup_node(&m.root, &keypath("a")),
            Some(Node::Map(_))
        ));
        assert!(lookup_leaf(&m.root, &keypath("a")).is_none());
        assert!(lookup_leaf(&m.root, &keypath("a.b")).is_some());
        assert!(lookup_node(&m.root, &keypath("a.b.c")).is_none());
        assert!(lookup_node(&m.root, &keypath("zz")).is_none());
    }

    use crate::inputs::Inputs;
    use crate::test_util::merge_files;

    fn ok_ctx(files: &[(&str, &str)], ctx: &[(&str, &str)]) -> Merged {
        let mut inputs = Inputs::new();
        for (k, v) in ctx {
            inputs = inputs.with_ctx(k, v);
        }
        merge_files(files, &inputs)
            .map(|(m, _)| m)
            .unwrap_or_else(|e| panic!("{}", e.message))
    }
    fn err_ctx(files: &[(&str, &str)], ctx: &[(&str, &str)]) -> Error {
        let mut inputs = Inputs::new();
        for (k, v) in ctx {
            inputs = inputs.with_ctx(k, v);
        }
        merge_files(files, &inputs)
            .map(|_| ())
            .expect_err("expected merge error")
    }

    #[test]
    fn block_when_merges_matching_arm_and_marks_ops_conditional() {
        let src = r#"app { when ctx.platform { "mobile" => { x = 1 } else => { x = 2 } } }"#;
        let m = ok_ctx(&[("main.medl", src)], &[("platform", "mobile")]);
        assert!(matches!(leaf(&m, "app.x")[0].expr, Expr::Int(1, _)));
        assert!(leaf(&m, "app.x")[0].conditional);
        let m = ok_ctx(&[("main.medl", src)], &[("platform", "xr")]);
        assert!(matches!(leaf(&m, "app.x")[0].expr, Expr::Int(2, _)));
    }

    #[test]
    fn block_when_without_else_contributes_nothing_on_no_match() {
        let src = r#"app { when ctx.platform { "mobile" => { x = 1 } } }"#;
        let m = ok_ctx(&[("main.medl", src)], &[("platform", "xr")]);
        assert!(lookup_leaf(&m.root, &keypath("app.x")).is_none());
        assert!(matches!(m.root.get("app"), Some(Node::Map(_))));
    }

    #[test]
    fn block_when_supports_tuples_builtins_and_top_level() {
        let src = r#"when (ctx.a, contains(["x", "y"], ctx.b)) { ("1", true) => { z = 1 } }"#;
        let m = ok_ctx(&[("main.medl", src)], &[("a", "1"), ("b", "y")]);
        assert_eq!(ops(&m, "z"), vec![AssignOp::Set]);
    }

    #[test]
    fn extends_and_required_inside_arms_fire_only_on_match() {
        let files = [
            (
                "main.medl",
                r#"app { when ctx.variant { "c" => { extends("c.medl")  required session.app_id "msg" } } }"#,
            ),
            ("c.medl", "y = 1"),
        ];
        let m = ok_ctx(&files, &[("variant", "c")]);
        assert_eq!(ops(&m, "y"), vec![AssignOp::Set]);
        assert_eq!(m.required.len(), 1);
        assert_eq!(m.required[0].path, keypath("app.session.app_id"));
        let m = ok_ctx(&files, &[("variant", "s")]);
        assert!(lookup_leaf(&m.root, &keypath("y")).is_none());
        assert!(m.required.is_empty());
    }

    #[test]
    fn block_when_rejects_config_paths_in_subject_and_pattern() {
        let e = err_ctx(
            &[("main.medl", "app { f = true\nwhen app.f { true => {} } }")],
            &[],
        );
        assert_eq!(e.kind, ErrorKind::Merge);
        assert_eq!(e.message, "block-position `when` may only reference ctx, env, or secret; `app.f` is a config value (use a value-position `when` instead)");
        let e = err_ctx(
            &[("main.medl", "when ctx.x { app.y => {} }")],
            &[("x", "1")],
        );
        assert_eq!(e.message, "block-position `when` may only reference ctx, env, or secret; `app.y` is a config value (use a value-position `when` instead)");
        // An offending pattern in a later arm is rejected even though the first arm matches.
        let e = err_ctx(
            &[("main.medl", "when ctx.x { \"1\" => {}  app.y => {} }")],
            &[("x", "1")],
        );
        assert!(e.message.contains("`app.y` is a config value"));
    }

    #[test]
    fn block_when_reports_missing_ctx_key() {
        let e = err_ctx(
            &[("main.medl", r#"when ctx.platform { "mobile" => {} }"#)],
            &[],
        );
        assert_eq!(e.message, "ctx has no key `platform`");
    }

    #[test]
    fn unfilled_required_is_reported_with_message() {
        let m = ok(&[(
            "main.medl",
            "app { required sku \"every variant must set a SKU\" }",
        )]);
        let e = check_required(&m).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Required);
        assert_eq!(e.message, "1 required key(s) were never filled: app.sku");
        assert_eq!(e.labels.len(), 1);
        assert_eq!(
            e.labels[0].message,
            "`app.sku` declared required here: every variant must set a SKU"
        );
        assert_eq!(e.span, Some(e.labels[0].span));
    }

    #[test]
    fn unfilled_required_dedupes_by_key_and_lists_every_declaration() {
        let m = ok(&[
            (
                "main.medl",
                "extends(\"b.medl\")\na { required x  required y }",
            ),
            ("b.medl", "a { required x \"from b\" }"),
        ]);
        let e = check_required(&m).unwrap_err();
        assert_eq!(e.message, "2 required key(s) were never filled: a.x, a.y");
        assert_eq!(e.labels.len(), 3);
        assert_eq!(e.labels[0].message, "`a.x` declared required here: from b");
        assert_eq!(e.labels[1].message, "`a.x` declared required here");
    }

    #[test]
    fn required_is_filled_by_set_append_or_productive_block_in_any_order() {
        for src in [
            "a { required x }\na { x = 1 }",
            "a { x = 1 }\na { required x }",
            "a { required x }\na { x += [1] }",
            "a { required net }\na { net { t = 1 } }",
            "a { required net }\na { net { deeper { t = 1 } } }",
        ] {
            let m = ok(&[("main.medl", src)]);
            check_required(&m).unwrap_or_else(|e| panic!("{src}: {}", e.message));
        }
    }

    #[test]
    fn required_is_not_filled_by_remove_or_by_required_only_blocks() {
        let m = ok(&[("main.medl", "a { required x }\na { x -= [1] }")]);
        assert_eq!(
            check_required(&m).unwrap_err().message,
            "1 required key(s) were never filled: a.x"
        );
        let m = ok(&[("main.medl", "a { required net }\na { net { required t } }")]);
        assert_eq!(
            check_required(&m).unwrap_err().message,
            "2 required key(s) were never filled: a.net, a.net.t"
        );
    }

    #[test]
    fn required_never_removes_an_existing_value() {
        let m = ok(&[("main.medl", "a { x = 1 }\na { required x }")]);
        check_required(&m).unwrap();
        assert_eq!(ops(&m, "a.x"), vec![AssignOp::Set]);
    }
}
