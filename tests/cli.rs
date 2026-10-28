use std::{fs, process::Command};

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
