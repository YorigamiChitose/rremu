#!/usr/bin/env bash
# 使用冻结配置构建 S3.4a RV32 Linux Image。
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RREMU_ROOT="$(cd "$HERE/../.." && pwd)"
LINUX_DIR="${LINUX_DIR:-$RREMU_ROOT/../linux}"
CROSS_COMPILE="${CROSS_COMPILE:-riscv64-linux-gnu-}"
CONFIG="$HERE/rremu_rv32_test.config"
IMAGE="$LINUX_DIR/arch/riscv/boot/Image"

if [[ ! -d "$LINUX_DIR" ]]; then
  echo "Linux source not found: $LINUX_DIR" >&2
  exit 2
fi
if [[ ! -f "$CONFIG" ]]; then
  echo "frozen config not found: $CONFIG" >&2
  exit 2
fi

echo "==> build fixed initramfs"
"$HERE/build-initramfs.sh"

echo "==> install frozen kernel config"
cp "$CONFIG" "$LINUX_DIR/.config"
make -C "$LINUX_DIR" ARCH=riscv CROSS_COMPILE="$CROSS_COMPILE" olddefconfig >/tmp/rremu-linux-olddefconfig.log 2>&1
if ! cmp -s "$LINUX_DIR/.config" "$CONFIG"; then
  echo "ERROR: frozen config drifted after olddefconfig" >&2
  diff -u "$CONFIG" "$LINUX_DIR/.config" | head -80 >&2 || true
  exit 3
fi

echo "==> build Linux Image"
make -C "$LINUX_DIR" ARCH=riscv CROSS_COMPILE="$CROSS_COMPILE" -j"$(nproc)" Image

if [[ ! -f "$IMAGE" ]]; then
  echo "ERROR: Image not produced: $IMAGE" >&2
  exit 4
fi
ls -lh "$IMAGE"
