//! M1.11 集成测试：`rremu hello.elf -c c` 输出 "hello world" 且退出码 0。
//!
//! 验收（PLAN §11 步 1.11）：`rremu hello.elf -c c` 输出 hello world 退出码 0。

#![allow(clippy::expect_used)] // 集成测试内放行

use std::process::Command;

#[test]
fn hello_world_continue_batch() {
    let bin = env!("CARGO_BIN_EXE_rremu");
    let elf = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/data/hello.elf");
    let output = Command::new(bin)
        .arg(elf)
        .args(["-c", "c"])
        .output()
        .expect("failed to spawn rremu");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "rremu exited non-zero\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("hello world"),
        "expected 'hello world' in stdout, got: {stdout}"
    );
}
