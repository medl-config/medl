use medl::inputs::Inputs;
use medl::loader::FsLoader;
use medl::Medl;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// `inputs.txt` lines: `ctx.platform=mobile`, `env.HOST=x`, `secret.KEY=v`. Blank lines and `#` comments ignored.
fn read_inputs(dir: &Path) -> Inputs {
    let mut ctx = Vec::new();
    let mut env = HashMap::new();
    let mut secret = HashMap::new();
    if let Ok(text) = fs::read_to_string(dir.join("inputs.txt")) {
        for line in text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let (key, value) = line
                .split_once('=')
                .unwrap_or_else(|| panic!("bad inputs line: {line}"));
            let (root, name) = key
                .split_once('.')
                .unwrap_or_else(|| panic!("bad inputs key: {key}"));
            match root {
                "ctx" => ctx.push((name.to_string(), value.to_string())),
                "env" => {
                    env.insert(name.to_string(), value.to_string());
                }
                "secret" => {
                    secret.insert(name.to_string(), value.to_string());
                }
                other => panic!("unknown inputs root `{other}`"),
            }
        }
    }
    let mut inputs = Inputs::new().with_env(env).with_secret(secret);
    for (k, v) in ctx {
        inputs = inputs.with_ctx(&k, &v);
    }
    inputs
}

fn run_case(dir: &Path) -> Result<(), String> {
    let inputs = read_inputs(dir);
    let mut medl = Medl::new(Box::new(FsLoader));
    let result = medl.resolve(&dir.join("main.medl"), &inputs);
    let expected_json = dir.join("expected.json");
    let expected_err = dir.join("expected.err");
    match (result, expected_json.exists(), expected_err.exists()) {
        (Ok(out), true, _) => {
            let expected: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&expected_json).unwrap())
                    .map_err(|e| format!("bad expected.json: {e}"))?;
            let actual = out.value.to_json();
            if actual != expected {
                return Err(format!(
                    "output mismatch\n expected: {expected}\n   actual: {actual}"
                ));
            }
            let rendered: String = out
                .warnings
                .iter()
                .map(|w| w.render(&medl.sources))
                .collect();
            check_lines(dir.join("expected.warnings"), &rendered, "warnings")
        }
        (Ok(out), false, true) => Err(format!(
            "expected an error but resolved to {}",
            out.value.to_json()
        )),
        (Err(e), _, true) => check_lines(expected_err, &e.render(&medl.sources), "error output"),
        (Err(e), _, false) => Err(format!("unexpected error:\n{}", e.render(&medl.sources))),
        (Ok(_), false, false) => Err("case has neither expected.json nor expected.err".to_string()),
    }
}

/// Every non-blank line of `file` must appear as a substring of `rendered`.
/// A missing file means `rendered` must be empty.
fn check_lines(file: PathBuf, rendered: &str, what: &str) -> Result<(), String> {
    let Ok(text) = fs::read_to_string(&file) else {
        return if rendered.is_empty() {
            Ok(())
        } else {
            Err(format!("unexpected {what}:\n{rendered}"))
        };
    };
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        if !rendered.contains(line) {
            return Err(format!("{what} missing {line:?}\n got:\n{rendered}"));
        }
    }
    Ok(())
}

/// A libtest thread gets a 2 MiB stack, which a debug build outgrows while parsing the
/// deeply nested `nesting_too_deep` case, so the cases run with a stack a real program would have.
#[test]
fn fixture_cases() {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run_all_cases)
        .unwrap()
        .join()
        .unwrap();
}

fn run_all_cases() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cases");
    let mut dirs: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "no fixture cases found");
    let failures: Vec<String> = dirs
        .iter()
        .filter_map(|dir| {
            run_case(dir)
                .err()
                .map(|e| format!("[{}] {e}", dir.file_name().unwrap().to_string_lossy()))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} fixture case(s) failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
