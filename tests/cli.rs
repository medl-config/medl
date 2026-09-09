use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn case_dir(name: &str, main: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("medl-cli-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("main.medl"), main).unwrap();
    dir
}

fn medl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_medl"))
}

#[test]
fn resolve_prints_pretty_json_in_declaration_order() {
    let dir = case_dir("ok", "app {\n  z = 1\n  a = \"${ctx.platform}\"\n}\n");
    let out = medl()
        .args(["resolve", "--ctx", "platform=mobile"])
        .arg(dir.join("main.medl"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "{\n  \"app\": {\n    \"z\": 1,\n    \"a\": \"mobile\"\n  }\n}\n"
    );
    assert!(out.stderr.is_empty());
}

#[test]
fn errors_go_to_stderr_with_exit_code_1() {
    let dir = case_dir("err", "a = \"${b}\"\nb = \"${a}\"\n");
    let out = medl()
        .arg("resolve")
        .arg(dir.join("main.medl"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("error: dependency cycle: a -> b -> a"),
        "{stderr}"
    );
    assert!(stderr.contains("main.medl:2:"), "{stderr}");
    assert!(out.stdout.is_empty());
}

#[test]
fn secrets_come_from_flags_and_env_from_the_process() {
    let dir = case_dir(
        "inputs",
        "k = \"${secret.K}\"\ne = \"${env.MEDL_CLI_TEST_ENV}\"\n",
    );
    let out = medl()
        .args(["resolve", "--secret", "K=s"])
        .arg(dir.join("main.medl"))
        .env("MEDL_CLI_TEST_ENV", "v")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"k\": \"s\""), "{stdout}");
    assert!(stdout.contains("\"e\": \"v\""), "{stdout}");
}

#[test]
fn strict_prints_warnings_only_when_asked() {
    let dir = case_dir("strict", "f = [1]\nf -= [2]\n");
    let quiet = medl()
        .arg("resolve")
        .arg(dir.join("main.medl"))
        .output()
        .unwrap();
    assert!(quiet.status.success());
    assert!(
        quiet.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&quiet.stderr)
    );
    let strict = medl()
        .args(["resolve", "--strict"])
        .arg(dir.join("main.medl"))
        .output()
        .unwrap();
    assert!(strict.status.success());
    let stderr = String::from_utf8_lossy(&strict.stderr);
    assert!(
        stderr.contains("warning: `-=` on `f` removed nothing"),
        "{stderr}"
    );
}

#[test]
fn malformed_pair_is_a_usage_error() {
    let dir = case_dir("usage", "a = 1\n");
    let out = medl()
        .args(["resolve", "--ctx", "platform"])
        .arg(dir.join("main.medl"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr)
        .contains("error: --ctx expected KEY=VALUE, got `platform`"));
}
