#!/usr/bin/env bash
# 一键：编译 DTB -> 构建 OpenSBI(RV32 FW_JUMP) -> 在 rremu 上运行。
# 用法：./run-opensbi.sh [--tui]
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RREMU_ROOT="$(cd "$HERE/.." && pwd)"
OPENSBI_DIR="${OPENSBI_DIR:-/home/yori/yori-riscv-util/opensbi}"
FW="$OPENSBI_DIR/build/platform/generic/firmware/fw_jump.elf"
DTS="$HERE/rremu.dts"
DTB="$HERE/rremu.dtb"
BIN="$RREMU_ROOT/target/release/rremu"

echo "==> build rremu"
(cd "$RREMU_ROOT" && cargo build --release)

echo "==> compile DTB"
dtc -I dts -O dtb -o "$DTB" "$DTS"

echo "==> build OpenSBI (RV32 generic FW_JUMP)"
(cd "$OPENSBI_DIR" && make clean >/dev/null && \
  make PLATFORM=generic CROSS_COMPILE=riscv64-linux-gnu- \
       PLATFORM_RISCV_XLEN=32 FW_JUMP=y \
       FW_JUMP_ADDR=0x80200000 FW_JUMP_FDT_ADDR=0x80F00000 >/dev/null)

echo "==> run"
if [[ "${1:-}" == "--tui" ]]; then
  exec "$BIN" "$FW" --load-base 0x80000000 --dtb "$DTB" --tui
else
  exec timeout -s INT 6 "$BIN" "$FW" --load-base 0x80000000 --dtb "$DTB" --no-fb -c c
fi
