/*
 * rremu S3.4a 最小 init：
 * - 通过 write(1, ...) 输出到达用户态的标志
 * - 通过 reboot(LINUX_REBOOT_CMD_POWER_OFF) 请求停机
 * - 若 reboot 返回则死循环
 */

typedef unsigned long u32;

static long syscall4(long n, long a, long b, long c, long d) {
    register long a0 __asm__("a0") = a;
    register long a1 __asm__("a1") = b;
    register long a2 __asm__("a2") = c;
    register long a3 __asm__("a3") = d;
    register long a7 __asm__("a7") = n;
    __asm__ volatile("ecall"
                     : "+r"(a0)
                     : "r"(a1), "r"(a2), "r"(a3), "r"(a7)
                     : "memory");
    return a0;
}

__attribute__((noreturn))
void _start(void) {
    static const char msg[] = "rremu: linux userspace init reached\n";
    syscall4(64, 1, (long)msg, sizeof(msg) - 1, 0); /* write */
    /* reboot(LINUX_REBOOT_MAGIC1, LINUX_REBOOT_MAGIC2, POWER_OFF, NULL) */
    syscall4(142, 0xfee1dead, 672274793, 0x4321fedc, 0);
    for (;;) {
        syscall4(93, 0, 0, 0, 0); /* exit */
    }
}
