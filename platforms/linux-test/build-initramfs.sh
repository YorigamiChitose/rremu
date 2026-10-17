#!/usr/bin/env bash
# 构建 S3.4a 固定 initramfs：只包含一个静态 RV32 /init。
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${1:-$HERE/initramfs}"
CC="${CROSS_COMPILE:-riscv64-linux-gnu-}gcc"

rm -rf "$OUT"
mkdir -p "$OUT"
"$CC" \
  -march=rv32imac -mabi=ilp32 \
  -nostdlib -static -fno-stack-protector -fno-pic \
  -o "$OUT/init" "$HERE/init.c" \
  -Wl,-e,_start
chmod +x "$OUT/init"

echo "initramfs ready: $OUT/init"
