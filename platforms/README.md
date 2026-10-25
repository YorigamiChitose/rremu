# platforms — 平台定义与外部固件运行

本目录提供 rremu 的**平台描述**，用于运行 **OpenSBI**（M-mode 固件）等需要设备树的外部镜像。

## 文件

| 文件 | 说明 |
|---|---|
| `rremu.dts` | rremu 平台设备树源（RAM/CLINT/PLIC/UART/RTC/syscon/CPU，RV32） |
| `rremu-linux.dts` | Linux 启动用设备树源（128MiB RAM、`bootargs`） |
| `rremu.dtb` / `rremu-linux.dtb` | 由对应 DTS 编译出的二进制设备树 |
| `run-opensbi.sh` | 一键：编译 DTB → 构建 OpenSBI（RV32 FW_JUMP）→ 在 rremu 上运行 |
| `run-linux.sh` | 一键：构建 OpenSBI FW_PAYLOAD（嵌入 Linux Image）→ 启动 Linux |

## 依赖

- `dtc`（device tree compiler）
- `scons`（RT-Thread 用；本目录不需要）
- OpenSBI 源码（默认 rremu 同级的 `../opensbi`，可用 `OPENSBI_DIR` 覆盖）
- 交叉工具链：`riscv64-linux-gnu-`（**必须**——OpenSBI 强制 PIE，`riscv64-unknown-elf` 的 ld 不支持 `-pie`）

## 手动运行

```sh
# 1) 编译 DTB
dtc -I dts -O dtb -o rremu.dtb rremu.dts

# 2) 构建 OpenSBI（RV32 generic FW_JUMP）
cd "$OPENSBI_DIR" && make clean
make PLATFORM=generic CROSS_COMPILE=riscv64-linux-gnu- \
     PLATFORM_RISCV_XLEN=32 FW_JUMP=y \
     FW_JUMP_ADDR=0x80200000 FW_JUMP_FDT_ADDR=0x80F00000

# 3) 在 rremu 上运行
rremu build/platform/generic/firmware/fw_jump.elf \
     --load-base 0x80000000 \
     --dtb rremu.dtb \
     --no-fb -c c
```

或直接：`./run-opensbi.sh`（TUI 加 `--tui`）。

## 关键约定（踩坑记录）

- **OpenSBI 是 PIE**（ELF 链接在 `0x0`），须用 `--load-base 0x80000000` 按 RAM 基址加载；
  它运行时会自行处理 `.rela.dyn` 重定位。
- `FW_JUMP_FDT_ADDR` 默认 `0x82200000`，**超出 rremu 默认 16MiB RAM** → 必须设为 RAM 内地址
  （此处 `0x80F00000`，与 `--dtb` 放置位置一致）。
- `FW_TEXT_START` **保持默认 `0`**（PIE 链接基址）；设成 `0x80000000` 会导致重定位写错地址。
- OpenSBI 会执行 `sfence.vma`、`mret`/`sret`、读写 `medeleg/mideleg`、`pmpcfg/pmpaddr`
  ——这些由 rremu 的 S2 能力支持。

## 预期输出（节选）

```
OpenSBI v1.9
Platform Name               : rremu-rv32
Platform HART Protection    : pmp
Platform Timer Device       : aclint-mtimer @ 10000000Hz
Platform Console Device     : uart8250
Domain0 Next Address        : 0x80200000
Domain0 Next Arg1           : 0x80f00000
Domain0 Next Mode           : S-mode
Boot HART Base ISA          : rv32imafdc
```

之后跳转到空的 `0x80200000`（尚无 payload/Linux），符合预期。


## 运行 Linux

需要一份 RV32 Linux Image，建议内核内置 initramfs：

```sh
cd platforms
LINUX_IMAGE=/path/to/Image ./run-linux.sh        # 或 --tui
```

脚本使用：

- `rremu-linux.dts`：128MiB RAM，`bootargs = "console=ttyS0 earlycon keep_bootcon"`
- OpenSBI `FW_PAYLOAD`，payload offset `0x400000`（RV32 Linux Image 头要求 4MiB）
- DTB/FDT 地址 `0x87f00000`
- rremu `-m 128M`

如果 Linux Image 未内置 initramfs，需要自行构建带 initramfs 的内核或改用其他
initrd 注入方案。

Linux 成功启动后应看到：

```text
Run /init as init process
rremu: linux userspace init reached
reboot: Power down
Program exited with code 0.
```
