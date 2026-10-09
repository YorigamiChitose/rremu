# 定时器中断测试固件（M3.6）
#
# 验证 CLINT mtime/mtimecmp 驱动的机器定时器中断完整路径：
#   1. 设置 mtvec → handler
#   2. 使能 mie.MTI + mstatus.MIE
#   3. 设 mtimecmp = mtime + REARM，随后进入自旋
#   4. mtime ≥ mtimecmp 时 CPU 在取指前陷入 → handler
#   5. handler 中重设 mtimecmp（推到 mtime 之后），count++
#   6. count 达 THRESHOLD 后写 tohost 退出
#
# REARM 必须大于 handler 指令数（~18 条），否则 mret 后立即重陷，
# 主循环无机会检查 count —— 这里取 30 留余量。

    .option norvc
    .section .text.init
    .globl _start
_start:
    la      sp, _stack_top          # 栈顶 = RAM 末尾
    la      t0, handler             # mtvec ← handler
    csrw    mtvec, t0
    li      t0, 0x80               # mie.MTI (bit 7)
    csrw    mie, t0
    # 设初始 mtimecmp = mtime + REARM
    li      t0, 0x0200BFF8         # CLINT mtime 地址
    lw      t1, 0(t0)
    addi    t1, t1, 30             # REARM = 30
    li      t0, 0x02004000         # CLINT mtimecmp 地址
    sw      t1, 0(t0)
    li      t0, 8                 # mstatus.MIE (bit 3)
    csrs    mstatus, t0
spin:
    j       spin                  # 等待中断（无 wfi 支持，忙等）

# ---- 中断 handler ----
handler:
    addi    sp, sp, -8
    sw      t0, 0(sp)
    sw      t1, 4(sp)
    # 重设 mtimecmp = mtime + REARM（先推过 mtime 再 mret，防死循环）
    li      t0, 0x0200BFF8
    lw      t1, 0(t0)
    addi    t1, t1, 30
    li      t0, 0x02004000
    sw      t1, 0(t0)
    # count++
    la      t0, count
    lw      t1, 0(t0)
    addi    t1, t1, 1
    sw      t1, 0(t0)
    # count >= THRESHOLD(3) → 退出
    li      t0, 3
    blt     t1, t0, resume
    # 退出：写 tohost = (0 << 1) | 1 → exit code 0
    li      t0, 1
    la      t1, tohost
    sw      t0, 0(t1)
exit_spin:
    j       exit_spin             # 兜底自旋（正常应已 halt）
resume:
    lw      t0, 0(sp)
    lw      t1, 4(sp)
    addi    sp, sp, 8
    mret

    .section .bss
    .align 2
count:
    .word 0

    .section .tohost, "aw", @progbits
    .align 6
    .globl tohost
tohost:
    .word 0
    .globl fromhost
fromhost:
    .word 0
