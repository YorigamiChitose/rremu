#!/usr/bin/env bash
# 一键：编译 DTB -> 构建 OpenSBI FW_PAYLOAD(Linux Image) -> 在 rremu 上启动。
# 用法：LINUX_IMAGE=/path/to/Image ./run-linux.sh [--tui]
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RREMU_ROOT="$(cd "$HERE/.." && pwd)"
OPENSBI_DIR="${OPENSBI_DIR:-$RREMU_ROOT/../opensbi}"
OPENSBI_DIR="$(cd "$OPENSBI_DIR" && pwd)"

LINUX_IMAGE="${LINUX_IMAGE:-$RREMU_ROOT/../linux/arch/riscv/boot/Image}"
MEM_SIZE="${MEM_SIZE:-128M}"
FDT_ADDR="0x87f00000"
FW="$OPENSBI_DIR/build/platform/generic/firmware/fw_payload.elf"
DTS="$HERE/rremu-linux.dts"
DTB="$HERE/rremu-linux.dtb"
BIN="$RREMU_ROOT/target/release/rremu"

if [[ ! -f "$LINUX_IMAGE" ]]; then
  echo "Linux Image not found: $LINUX_IMAGE" >&2
  echo "请设置 LINUX_IMAGE 指向 RV32 Linux Image（建议内置 initramfs）" >&2
  exit 2
fi

echo "==> build rremu"
(cd "$RREMU_ROOT" && cargo build --release)

echo "==> compile DTB"
dtc -I dts -O dtb -o "$DTB" "$DTS"

echo "==> build OpenSBI FW_PAYLOAD"
# rremu 是 RV32；Linux RV32 Image 头要求 4MiB 负载偏移。
# 删除整个 build 目录，避免 make clean 留下的旧 linker script
# 继续使用上一次的 payload offset。
rm -rf "$OPENSBI_DIR/build"
(cd "$OPENSBI_DIR" && \
  make PLATFORM=generic CROSS_COMPILE=riscv64-linux-gnu- \
       PLATFORM_RISCV_XLEN=32 FW_PAYLOAD=y \
       FW_PAYLOAD_PATH="$LINUX_IMAGE" \
       FW_PAYLOAD_OFFSET=0x400000 \
       FW_PAYLOAD_ALIGN=0x400000 \
       FW_PAYLOAD_FDT_ADDR="$FDT_ADDR" >/dev/null)

echo "==> run Linux"
# 默认保持运行；CLI 批量模式按 Ctrl+C 暂停并刷新 UART。
if [[ "${1:-}" == "--tui" ]]; then
  exec "$BIN" "$FW" --load-base 0x80000000 --dtb "$DTB" -m "$MEM_SIZE" --tui
else
  exec "$BIN" "$FW" --load-base 0x80000000 --dtb "$DTB" -m "$MEM_SIZE" --no-fb -c c
fi
