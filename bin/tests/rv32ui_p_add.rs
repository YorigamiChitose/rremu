//! M1.12 集成测试：`rremu rv32ui-p-add.elf -c c` PASS（tohost=1 → exit 0）。
//!
//! 验收（PLAN §11 步 1.12）：rv32ui-p-add PASS（tohost=1）。
//! riscv-tests 约定：TESTNUM=1 写入 tohost 表示 PASS；rremu 退出码 = tohost>>1 = 0。

#![allow(clippy::expect_used)] // 集成测试内放行

use std::process::Command;

#[test]
fn rv32ui_p_add_passes() {
    let bin = env!("CARGO_BIN_EXE_rremu");
    let elf = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/data/rv32ui-p-add.elf");
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
        stdout.contains("Program exited with code 0"),
        "expected PASS (exit code 0), got stdout: {stdout}"
    );
}
