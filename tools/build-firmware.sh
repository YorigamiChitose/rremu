#!/usr/bin/env bash
# 编译 hello world 固件 → tests/data/hello.elf（M1.7，产物 gitignore）
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FW="$ROOT/tests/firmware"
OUT="$ROOT/tests/data"

mkdir -p "$OUT"

riscv64-unknown-elf-gcc \
    -march=rv32imc_zicsr -mabi=ilp32 \
    -nostdlib -static \
    -T "$FW/hello.ld" \
    -o "$OUT/hello.elf" \
    "$FW/hello.S"

echo "OK: $OUT/hello.elf"
