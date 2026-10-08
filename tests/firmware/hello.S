# hello world 固件（M1.7）
# - march 含 c 但 .option norvc：M1 内核尚无 C 解码，强制生成 32 位编码
# - 退出约定：写 tohost = (0 << 1) | 1 → code 0（见 PLAN §5.8）

    .option norvc
    .section .text.init
    .globl _start
_start:
    la      sp, _stack_top          # 栈顶 = RAM 末尾（链接脚本提供）
    la      t0, msg                 # t0 = 消息游标
    la      t1, msg_end             # t1 = 消息结尾
1:  bgeu    t0, t1, 2f              # 游标越过结尾 → 完成
    lb      t2, 0(t0)               # 取一个字符
    li      t3, 0x10000000          # UART THR 基址（PLAN §5.5）
    sb      t2, 0(t3)               # 发送字符
    addi    t0, t0, 1
    j       1b
2:  li      t0, 1                   # (0 << 1) | 1 → Exited { code: 0 }
    la      t1, tohost
    sw      t0, 0(t1)
3:  j       3b                      # 兜底自旋（正常不应到达）

    .section .rodata
msg:
    .ascii "hello world\n"
msg_end:

    .section .tohost, "aw", @progbits
    .align 6
    .globl tohost
tohost:
    .word 0
    .globl fromhost
fromhost:
    .word 0
