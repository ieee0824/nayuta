use std::{fs, process::Command};

#[test]
fn deferred_bodies_have_independent_loop_depth() {
    let dir = std::env::temp_dir().join(format!("nayuta-deferred-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("deferred.rs");
    fs::write(
        &path,
        r#"fn main() {
    let n = std::env::args().nth(1).unwrap_or("100".into()).parse::<usize>().unwrap();
    for _ in 0..n {
        let _unused = || { for _ in 0..n {} };
        let _future = async { for _ in 0..n {} };
        let _nested = || { for _ in 0..n { for _ in 0..n {} } };
        let _nested_future = async { for _ in 0..n { for _ in 0..n {} } };
        for _ in 0..n {}
    }
}
"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_nayuta"))
        .arg(&path)
        .output()
        .unwrap();
    fs::remove_dir_all(dir).unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let findings: Vec<_> = stdout
        .lines()
        .filter_map(|line| line.split_once(": complexity: "))
        .map(|(location, message)| (location.rsplit(':').next().unwrap(), message))
        .collect();
    assert_eq!(
        findings,
        vec![
            ("3", "loop may grow as O(N)"),
            ("4", "loop may grow as O(N)"),
            ("5", "loop may grow as O(N)"),
            ("6", "loop may grow as O(N)"),
            ("6", "loop nesting depth is 2; work may grow as O(N^2)"),
            ("7", "loop may grow as O(N)"),
            ("7", "loop nesting depth is 2; work may grow as O(N^2)"),
            ("8", "loop nesting depth is 2; work may grow as O(N^2)"),
        ]
    );
}

#[test]
fn parse_failure_reports_error_and_continues() {
    let dir = std::env::temp_dir().join(format!("nayuta-parse-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("broken.rs"), "fn main( {").unwrap();
    fs::write(dir.join("valid.rs"), "fn f(xs: &[u8]) { for _ in xs {} }").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_nayuta"))
        .arg("--fail")
        .arg(&dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("broken.rs: parse error"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("O(N)"));
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn fifo_does_not_block_cli() {
    use std::time::{Duration, Instant};
    let dir = std::env::temp_dir().join(format!("nayuta-fifo-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(dir.join("blocked.rs"))
            .status()
            .unwrap()
            .success()
    );
    fs::write(dir.join("valid.rs"), "fn f(xs: &[u8]) { for _ in xs {} }").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_nayuta"))
        .arg(&dir)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            fs::remove_dir_all(&dir).unwrap();
            panic!("CLI blocked on FIFO");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    fs::remove_dir_all(dir).unwrap();
    assert!(status.success());
}
