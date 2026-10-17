#!/usr/bin/env bash
# 构建 S3.4a 固定 initramfs：只包含一个静态 RV32 /init。
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${1:-$HERE/initramfs}"
CC="${CROSS_COMPILE:-riscv64-linux-gnu-}gcc"
LIST="$HERE/initramfs.list"

rm -rf "$OUT"
mkdir -p "$OUT"
"$CC" \
  -march=rv32imac -mabi=ilp32 \
  -nostdlib -static -fno-stack-protector -fno-pic \
  -o "$OUT/init" "$HERE/init.c" \
  -Wl,-e,_start
chmod +x "$OUT/init"

# 用 gen_init_cpio 列表创建 /dev/console，避免宿主无权限 mknod。
# CONFIG_INITRAMFS_SOURCE 指向该列表文件。
OUT_ABS="$(cd "$(dirname "$OUT")" && pwd)/$(basename "$OUT")"
{
  echo "dir /dev 0755 0 0"
  echo "nod /dev/console 0600 0 0 c 5 1"
  echo "file /init $OUT_ABS/init 0755 0 0"
} > "$LIST"

echo "initramfs ready: $OUT/init (list: $LIST)"
