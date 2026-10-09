# framebuffer 测试固件（M5）
# 在 640x480 RGB565 帧缓冲上画 4 条水平彩色条：
#   红 0xF800 | 绿 0x07E0 | 蓝 0x001F | 白 0xFFFF，各 120 行高
# 每条 = 120 行 × 640 像素 × 2 字节 = 153600 字节 = 0x25800
# 退出约定：写 tohost = (0 << 1) | 1 → code 0（见 PLAN §5.8）

    .section .text.init
    .globl _start
_start:
    la      sp, _stack_top          # 栈顶 = RAM 末尾

    li      t0, 0x20000000          # t0 = fb 写入游标（从 fb 基址开始）

    # --- Bar 0: 红 0xF800, rows 0-119 ---
    li      t2, 0xF800F800          # 每字写 2 个红像素（低16位=像素0, 高16位=像素1）
    li      t1, 0x20025800          # 本条结束地址 = base + 0x25800
1:  sw      t2, 0(t0)
    addi    t0, t0, 4
    bltu    t0, t1, 1b

    # --- Bar 1: 绿 0x07E0, rows 120-239 ---
    li      t2, 0x07E007E0
    li      t1, 0x2004B000          # base + 0x4B000
2:  sw      t2, 0(t0)
    addi    t0, t0, 4
    bltu    t0, t1, 2b

    # --- Bar 2: 蓝 0x001F, rows 240-359 ---
    li      t2, 0x001F001F
    li      t1, 0x20070800          # base + 0x70800
3:  sw      t2, 0(t0)
    addi    t0, t0, 4
    bltu    t0, t1, 3b

    # --- Bar 3: 白 0xFFFF, rows 360-479 ---
    li      t2, 0xFFFFFFFF          # -1 = 两个 0xFFFF 白像素
    li      t1, 0x20096000          # base + 0x96000 = fb 末尾
4:  sw      t2, 0(t0)
    addi    t0, t0, 4
    bltu    t0, t1, 4b

    # --- 退出：tohost = 1 → Exited { code: 0 } ---
    li      t0, 1
    la      t1, tohost
    sw      t0, 0(t1)
5:  j       5b                      # 兜底自旋

    .section .tohost, "aw", @progbits
    .align 6
    .globl tohost
tohost:
    .word 0
    .globl fromhost
fromhost:
    .word 0
