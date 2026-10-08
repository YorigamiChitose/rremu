#!/usr/bin/env bash
# tools/build-tests.sh — 拉取官方 riscv-tests 编译 3 个代表测试到 tests/data/
#
# 用法：./tools/build-tests.sh [test...]
#   默认编译 rv32ui-p-add（M1.12 验收用）
#   M2 补 rv32um-p-mul / rv32uc-p-rvc
#
# 需要：riscv64-unknown-elf-gcc（已确认具备）
# march=rv32im_zicsr（含 M + Zicsr，不含 C，避免 M1 无 C 解码）

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
DATA_DIR="$ROOT_DIR/tests/data"
TMP_DIR="${TMPDIR:-/tmp}/riscv-tests"
TESTS="${*:-rv32ui-p-add}"

# 支持外部 riscv-tests 路径：RISCV_TESTS=/path/to/riscv-tests ./build-tests.sh
# 若未设置，则使用 $TMPDIR/riscv-tests（首次自动克隆 + 初始化 env 子模块）
if [ -n "${RISCV_TESTS:-}" ]; then
    RT_DIR="$RISCV_TESTS"
    if [ ! -d "$RT_DIR/env/p" ]; then
        echo "ERROR: RISCV_TESTS=$RT_DIR 但 env/p 不存在；请先初始化子模块："
        echo "  cd $RT_DIR && git submodule update --init env"
        exit 1
    fi
else
    if [ ! -d "$TMP_DIR/isa" ]; then
        echo "==> Cloning riscv-tests to $TMP_DIR"
        git clone --depth 1 https://github.com/riscv/riscv-tests.git "$TMP_DIR"
        git -C "$TMP_DIR" submodule update --init env
    fi
    RT_DIR="$TMP_DIR"
fi

mkdir -p "$DATA_DIR"

# ---- 2. 编译每个测试 ----
ISA_DIR="$RT_DIR/isa"
ENV_P="$RT_DIR/env/p"
MACROS="$ISA_DIR/macros/scalar"

for test in $TESTS; do
    # 解析：rv32ui-p-add → isa/rv32ui/add.S
    ext="${test%%-p-*}"     # rv32ui
    name="${test##*-p-}"     # add
    src="$ISA_DIR/$ext/$name.S"

    if [ ! -f "$src" ]; then
        # 某些测试名用 _ 而非 -（如 ld_st）
        src="$ISA_DIR/$ext/${name//-/_}.S"
    fi

    if [ ! -f "$src" ]; then
        echo "ERROR: cannot find source for $test ($ext/$name)"
        exit 1
    fi

    out="$DATA_DIR/$test.elf"
    echo "==> Building $test ($src → $out)"

    riscv64-unknown-elf-gcc \
        -march=rv32im_zicsr -mabi=ilp32 \
        -nostdlib -nostartfiles -static -mcmodel=medany \
        -I "$ENV_P" -I "$MACROS" \
        -T "$ENV_P/link.ld" \
        "$src" -o "$out"

    echo "    done: $out"
done

echo "==> All tests built successfully"
