#!/usr/bin/env bash
# 编译固件 → tests/data/*.elf（产物 gitignore）
#   hello.elf  — hello world（M1.7）
#   timer.elf  — 定时器中断（M3.6）
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FW="$ROOT/tests/firmware"
OUT="$ROOT/tests/data"
LD="$FW/hello.ld"  # 共用同一链接脚本（RAM 布局一致）

CFLAGS="-march=rv32imc_zicsr -mabi=ilp32 -nostdlib -static -T $LD"
CC="${RISCV_CC:-riscv64-unknown-elf-gcc}"

mkdir -p "$OUT"

build() {
    local name="$1"
    "$CC" $CFLAGS -o "$OUT/$name.elf" "$FW/$name.S"
    echo "OK: $OUT/$name.elf"
}

build hello
build timer
