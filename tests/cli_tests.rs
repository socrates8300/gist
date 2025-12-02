use std::process::Command;
use std::path::PathBuf;

fn get_binary_path() -> PathBuf {
    let mut path = std::env::current_exe().unwrap();
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("gist")
}

#[test]
fn test_help() {
    let binary = get_binary_path();
    let output = Command::new(binary)
        .arg("--help")
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage:"));
}

#[test]
fn test_version() {
    let binary = get_binary_path();
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());
}
