use crate::ast::{Expr, RESERVED_ROOTS};
use crate::error::{Error, ErrorKind};
use crate::merge::{lookup_leaf, Merged, MergedMap, Node};
use crate::span::Span;
use std::collections::HashMap;

pub type KeyPath = Vec<String>;

/// Every config-path reference in `expr`, including inside all `when` arms and fallbacks.
pub fn collect_refs(expr: &Expr, out: &mut Vec<(KeyPath, Span)>) {
    expr.walk(&mut |e| {
        if let Expr::Path(ids, span) = e {
            if !RESERVED_ROOTS.contains(&ids[0].name.as_str()) {
                out.push((ids.iter().map(|i| i.name.clone()).collect(), *span));
            }
        }
    });
}

pub fn topo_order(merged: &Merged) -> Result<Vec<KeyPath>, Error> {
    let mut leaves: Vec<KeyPath> = Vec::new();
    collect_leaves(&merged.root, &mut Vec::new(), &mut leaves);
    let index: HashMap<&KeyPath, usize> = leaves.iter().enumerate().map(|(i, k)| (k, i)).collect();

    let mut deps: Vec<Vec<(usize, Span)>> = vec![Vec::new(); leaves.len()];
    for (i, key) in leaves.iter().enumerate() {
        let mut refs = Vec::new();
        for op in lookup_leaf(&merged.root, key).expect("collected from tree") {
            collect_refs(&op.expr, &mut refs);
        }
        for (path, span) in refs {
            for target in ref_targets(&merged.root, &path) {
                deps[i].push((index[&target], span));
            }
        }
    }

    let mut walk = Walk {
        leaves: &leaves,
        deps: &deps,
        state: vec![State::New; leaves.len()],
        stack: Vec::new(),
        order: Vec::new(),
    };
    for i in 0..leaves.len() {
        walk.visit(i)?;
    }
    Ok(walk.order)
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    New,
    Visiting,
    Done,
}

struct Walk<'a> {
    leaves: &'a [KeyPath],
    deps: &'a [Vec<(usize, Span)>],
    state: Vec<State>,
    stack: Vec<usize>,
    order: Vec<KeyPath>,
}

impl Walk<'_> {
    /// Depth-first, post-order, with an explicit frame stack so a long dependency chain
    /// cannot overflow the real one. Each frame is a node plus a cursor into its deps.
    fn visit(&mut self, root: usize) -> Result<(), Error> {
        if self.state[root] == State::Done {
            return Ok(());
        }
        self.state[root] = State::Visiting;
        self.stack.push(root);
        let mut frames: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some(frame) = frames.last_mut() {
            let (i, cursor) = (frame.0, frame.1);
            if cursor == self.deps[i].len() {
                frames.pop();
                self.stack.pop();
                self.state[i] = State::Done;
                self.order.push(self.leaves[i].clone());
                continue;
            }
            frame.1 += 1;
            let (j, span) = self.deps[i][cursor];
            match self.state[j] {
                State::Visiting => return Err(self.cycle_error(j, span)),
                State::New => {
                    self.state[j] = State::Visiting;
                    self.stack.push(j);
                    frames.push((j, 0));
                }
                State::Done => {}
            }
        }
        Ok(())
    }

    /// The cycle path is the current stack from the offender's first occurrence, plus the
    /// offender again to close the loop.
    fn cycle_error(&self, j: usize, span: Span) -> Error {
        let start = self
            .stack
            .iter()
            .position(|&k| k == j)
            .expect("visiting nodes are on the stack");
        let mut names: Vec<String> = self.stack[start..]
            .iter()
            .map(|&k| self.leaves[k].join("."))
            .collect();
        names.push(self.leaves[j].join("."));
        Error::new(
            ErrorKind::Cycle,
            span,
            format!("dependency cycle: {}", names.join(" -> ")),
        )
    }
}

fn collect_leaves(map: &MergedMap, prefix: &mut Vec<String>, out: &mut Vec<KeyPath>) {
    for (key, node) in map {
        prefix.push(key.clone());
        match node {
            Node::Leaf(_) => out.push(prefix.clone()),
            Node::Map(m) => collect_leaves(m, prefix, out),
        }
        prefix.pop();
    }
}

/// Leaves that a reference to `path` depends on.
fn ref_targets(root: &MergedMap, path: &[String]) -> Vec<KeyPath> {
    let mut map = root;
    for (i, key) in path.iter().enumerate() {
        match map.get(key) {
            None => return Vec::new(),
            Some(Node::Leaf(_)) => return vec![path[..=i].to_vec()],
            Some(Node::Map(m)) => map = m,
        }
    }
    let mut out = Vec::new();
    collect_leaves(map, &mut path.to_vec(), &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::test_util::{keypath, merge_src};

    fn order(src: &str) -> Vec<String> {
        let m = merge_src(&[("main.medl", src)]).unwrap();
        topo_order(&m)
            .unwrap_or_else(|e| panic!("{}", e.message))
            .iter()
            .map(|k| k.join("."))
            .collect()
    }
    fn cycle(src: &str) -> Error {
        let m = merge_src(&[("main.medl", src)]).unwrap();
        topo_order(&m).expect_err("expected cycle")
    }

    #[test]
    fn dependencies_come_first_then_declaration_order() {
        assert_eq!(order("b = \"${a}\"\na = 1\nc = 2"), vec!["a", "b", "c"]);
    }

    #[test]
    fn map_reference_depends_on_every_leaf_under_it() {
        assert_eq!(
            order("z = \"${s}\"\ns { x = 1  y = \"${s.x}\" }"),
            vec!["s.x", "s.y", "z"]
        );
    }

    #[test]
    fn reference_through_a_leaf_depends_on_that_leaf() {
        assert_eq!(order("b = \"${a.c}\"\na = [1]"), vec!["a", "b"]);
    }

    #[test]
    fn unknown_keys_and_reserved_roots_add_no_edges() {
        assert_eq!(
            order("b = \"${nope} ${ctx.x} ${env.Y}\"\na = 1"),
            vec!["b", "a"]
        );
    }

    #[test]
    fn direct_and_self_cycles_are_reported_with_path() {
        let e = cycle("a = \"${b}\"\nb = \"${a}\"");
        assert_eq!(e.kind, ErrorKind::Cycle);
        assert_eq!(e.message, "dependency cycle: a -> b -> a");
        assert_eq!(cycle("a = \"${a}\"").message, "dependency cycle: a -> a");
    }

    #[test]
    fn cycle_through_untaken_when_arm_is_still_an_error() {
        let e = cycle("a = when ctx.x { \"1\" => 1  else => b }\nb = \"${a}\"");
        assert_eq!(e.message, "dependency cycle: a -> b -> a");
    }

    /// The chain is declared backwards, so the very first node visited is 20_000 deps deep.
    #[test]
    fn a_very_long_dependency_chain_resolves_without_overflowing() {
        const N: usize = 20_000;
        let mut src = String::new();
        for i in (1..N).rev() {
            src.push_str(&format!("k{i} = \"${{k{}}}\"\n", i - 1));
        }
        src.push_str("k0 = 1\n");
        let m = merge_src(&[("main.medl", src.as_str())]).unwrap();
        let order = topo_order(&m).unwrap_or_else(|e| panic!("{}", e.message));
        assert_eq!(order.len(), N);
        assert_eq!(order.first().map(|k| k.join(".")).as_deref(), Some("k0"));
        assert_eq!(order.last().map(|k| k.join(".")).as_deref(), Some("k19999"));
    }

    #[test]
    fn collect_refs_skips_reserved_roots_and_visits_all_arms() {
        let m = merge_src(&[(
            "main.medl",
            "a = when ctx.x { p => q  else => \"${r.s | t}\" }",
        )])
        .unwrap();
        let ops = crate::merge::lookup_leaf(&m.root, &keypath("a")).unwrap();
        let mut refs = Vec::new();
        collect_refs(&ops[0].expr, &mut refs);
        let names: Vec<String> = refs.iter().map(|(k, _)| k.join(".")).collect();
        assert_eq!(names, vec!["p", "q", "r.s", "t"]);
    }
}
