# S3.4 子规划：Linux 启动与设备

> 状态：**S3.4a 进行中，未合并**
> 目标分支：`test/s3.4-linux-devices`（三个仓库）
> 前置：S3.0–S3.3 已合并；CLI UART 直出已合并到 `main`

## 1. 目标与边界

S3.4 分两个子阶段：

### S3.4a：最小 Linux 启动

验收目标：

1. OpenSBI 加载 RV32 Linux Image
2. Linux 启动并进入内置 initramfs 的 `/init`
3. `/init` 向 UART 输出：
   ```text
   rremu: linux userspace init reached
   ```
4. `/init` 通过 reboot/poweroff 让 rremu 在测试时限内退出
5. 不回归现有 `rremu-test` 50 用例和 OpenSBI smoke test

本阶段不追求：

- 磁盘 rootfs
- 网络
- 图形
- SMP
- 完整 Linux defconfig
- virtio

### S3.4b：virtio 与 rootfs

只有 S3.4a 达到验收后，才开始：

- virtio-mmio
- virtio-blk
- rootfs
- 用户态 shell

## 2. 当前基线

### 2.1 仓库与分支

| 仓库 | 分支 | 状态 |
|---|---|---|
| `rremu` | `test/s3.4-linux-devices` | S3.4a 设备/脚本，未合并 |
| `rremu-am` | `test/s3.4-linux-devices` | RTC/syscon 地址常量，未合并 |
| `rremu-test` | `test/s3.4-linux-devices` | RTC/syscon 用例，未合并 |
| `rremu` | `main` | 已包含 CLI UART 直出 |

所有本地测试分支已 rebase 到包含 UART 直出的新 `main`。

### 2.2 已实现的 rremu 能力

| 能力/设备 | 实际状态 |
|---|---|
| RV32IMAC + Zicsr | 支持 |
| M/S/U 三级特权 | 支持 |
| Sv32 MMU | 支持 |
| PLIC | 支持 |
| CLINT | 支持，但 Linux 使用 SBI timer |
| NS16550A 子集 UART | 支持 |
| Goldfish RTC | 支持时间/alarm 寄存器；**没有 alarm IRQ** |
| syscon poweroff/reboot | 支持 |
| Framebuffer | 仅供 rremu 自身 TUI/窗口 |
| virtio | 未实现 |
| PCI/USB/网络/块设备 | 未实现 |
| Sstc/stimecmp | 未实现 |
| Zicbom/Zicboz/Svpbmt/Svnapot | 未实现 |
| F/D/V 扩展 | 未实现 |
| IOMMU/APLIC/IMSIC | 未实现 |
| ACPI/EFI | 未实现 |

### 2.3 已有 Linux 构建资产

当前临时使用：

```text
Linux 源码：/home/yori/yori-riscv-util/linux
Linux 版本：6.12.0
initramfs：/home/yori/yori-riscv-util/rremu-linux-initramfs
OpenSBI：/home/yori/yori-riscv-util/opensbi
```

当前已能构建：

- RV32 Linux Image
- 内置 initramfs
- OpenSBI FW_PAYLOAD
- rremu 可启动 Linux 到内核早期阶段

当前现象：

- 内核能输出 SBI、earlycon、内存布局、ASID、devtmpfs、clocksource 等信息
- 尚未稳定确认 `/init` 执行
- 尚未确认 syscon poweroff 成功
- 曾出现约 1 秒附近停住的情况，需要按 5 秒测试协议定位

### 2.4 CLI UART 直出

已合并到 `main` 的能力：

- CLI 模式 UART 输出实时写 stdout
- 可直接重定向：
  ```sh
  rremu fw_payload.elf ... -c c > /tmp/linux-test.log 2>&1
  ```
- TUI 行为不变

这是 S3.4 调试的基础：不再依赖暂停时 flush。

## 3. 固定测试配置

### 3.1 固定版本与路径

```text
Linux 源码：/home/yori/yori-riscv-util/linux
Linux 版本：6.12.0
交叉工具链：riscv64-linux-gnu-
OpenSBI：/home/yori/yori-riscv-util/opensbi
```

最终固定文件规划：

```text
rremu/platforms/linux-test/
├── init.c
├── build-initramfs.sh
├── rremu_rv32_test.config
└── build-linux.sh
```

其中 `rremu_rv32_test.config` 保存生成后的完整 `.config`，作为实际构建依据；本文件中的 fragment 是允许的配置变更集。

### 3.2 固定 Linux 配置：使用项

```text
CONFIG_32BIT=y
CONFIG_ARCH_RV32I=y
CONFIG_NONPORTABLE=y
# CONFIG_64BIT is not set
# CONFIG_PORTABLE is not set

CONFIG_MMU=y
CONFIG_RISCV_SBI=y
# CONFIG_RISCV_SBI_V01 is not set
# CONFIG_RISCV_M_MODE is not set

CONFIG_RISCV_INTC=y
CONFIG_SIFIVE_PLIC=y
CONFIG_RISCV_TIMER=y

# CONFIG_SMP is not set
CONFIG_NR_CPUS=1
CONFIG_HZ_100=y
CONFIG_HZ=100
CONFIG_HZ_PERIODIC=y
# CONFIG_NO_HZ_IDLE is not set

# CONFIG_RISCV_PROBE_UNALIGNED_ACCESS is not set
CONFIG_RISCV_SLOW_UNALIGNED_ACCESS=y
# CONFIG_RISCV_EFFICIENT_UNALIGNED_ACCESS is not set

CONFIG_RISCV_ISA_C=y

CONFIG_BLK_DEV_INITRD=y
CONFIG_INITRAMFS_COMPRESSION_NONE=y

CONFIG_DEVTMPFS=y
CONFIG_DEVTMPFS_MOUNT=y
CONFIG_PROC_FS=y
CONFIG_SYSFS=y
CONFIG_TMPFS=y
CONFIG_BINFMT_ELF=y
CONFIG_BINFMT_SCRIPT=y

CONFIG_TTY=y
CONFIG_PRINTK=y
CONFIG_SERIAL_8250=y
CONFIG_SERIAL_8250_CONSOLE=y
CONFIG_SERIAL_OF_PLATFORM=y
CONFIG_SERIAL_EARLYCON=y
CONFIG_SERIAL_8250_NR_UARTS=1
CONFIG_SERIAL_8250_RUNTIME_UARTS=1

CONFIG_RTC_CLASS=y
CONFIG_RTC_DRV_GOLDFISH=y

CONFIG_POWER_RESET=y
CONFIG_POWER_RESET_SYSCON=y
CONFIG_POWER_RESET_SYSCON_POWEROFF=y
CONFIG_SYSCON_REBOOT_MODE=y

CONFIG_CMDLINE_FALLBACK=y
CONFIG_CMDLINE="console=ttyS0 earlycon keep_bootcon"
# CONFIG_CMDLINE_EXTEND is not set
# CONFIG_CMDLINE_FORCE is not set
```

### 3.3 固定 Linux 配置：明确不使用项

以下设备/特性在 rremu 中不存在，固定关闭：

```text
# CONFIG_PCI is not set
# CONFIG_USB_SUPPORT is not set
# CONFIG_NET is not set
# CONFIG_WLAN is not set
# CONFIG_WIRELESS is not set
# CONFIG_BT is not set
# CONFIG_MEDIA_SUPPORT is not set
# CONFIG_DRM is not set
# CONFIG_FB is not set
# CONFIG_FRAMEBUFFER_CONSOLE is not set
# CONFIG_SOUND is not set
# CONFIG_HID is not set
# CONFIG_INPUT is not set
# CONFIG_I2C is not set
# CONFIG_SPI is not set
# CONFIG_PINCTRL is not set
# CONFIG_GPIOLIB is not set
# CONFIG_THERMAL is not set
# CONFIG_WATCHDOG is not set
# CONFIG_CPU_FREQ is not set
# CONFIG_CPU_IDLE is not set
# CONFIG_PM is not set
# CONFIG_SUSPEND is not set
# CONFIG_HIBERNATION is not set
# CONFIG_IOMMU_SUPPORT is not set
# CONFIG_KVM is not set
# CONFIG_XEN is not set
# CONFIG_HYPERV is not set
# CONFIG_VIRTIO is not set
# CONFIG_VIRTIO_MMIO is not set
# CONFIG_VIRTIO_BLK is not set
# CONFIG_VIRTIO_CONSOLE is not set
# CONFIG_MODULES is not set
# CONFIG_CGROUPS is not set
# CONFIG_NAMESPACES is not set
# CONFIG_SECURITY is not set
# CONFIG_AUDIT is not set
# CONFIG_PERF_EVENTS is not set
# CONFIG_BPF_SYSCALL is not set
# CONFIG_FTRACE is not set
# CONFIG_TRACING is not set
# CONFIG_HUGETLBFS is not set
# CONFIG_ACPI is not set
# CONFIG_EFI is not set
```

无块设备：

```text
# CONFIG_BLOCK is not set
# CONFIG_BLK_DEV is not set
# CONFIG_BLK_DEV_RAM is not set
# CONFIG_SCSI is not set
# CONFIG_ATA is not set
# CONFIG_MMC is not set
# CONFIG_MTD is not set
# CONFIG_NVME_CORE is not set
# CONFIG_SWAP is not set
```

无 RTC alarm 相关功能：

```text
# CONFIG_RTC_HCTOSYS is not set
# CONFIG_RTC_SYSTOHC is not set
# CONFIG_RTC_INTF_DEV is not set
# CONFIG_RTC_INTF_PROC is not set
```

无 8250 变体：

```text
# CONFIG_SERIAL_8250_DW is not set
# CONFIG_SERIAL_8250_PCI is not set
# CONFIG_SERIAL_8250_EXAR is not set
# CONFIG_SERIAL_8250_FINTEK is not set
# CONFIG_SERIAL_8250_DMA is not set
# CONFIG_SERIAL_EARLYCON_RISCV_SBI is not set
# CONFIG_HVC_RISCV_SBI is not set
```

无 APLIC/IMSIC/CLINT 驱动：

```text
# CONFIG_RISCV_APLIC is not set
# CONFIG_RISCV_IMSIC is not set
# CONFIG_RISCV_APLIC_MSI is not set
# CONFIG_CLINT_TIMER is not set
# CONFIG_RISCV_SBI_CPUIDLE is not set
```

无扩展：

```text
# CONFIG_FPU is not set
# CONFIG_RISCV_ISA_V is not set
# CONFIG_RISCV_ISA_V_DEFAULT_ENABLE is not set
# CONFIG_RISCV_ISA_SVNAPOT is not set
# CONFIG_RISCV_ISA_SVPBMT is not set
# CONFIG_RISCV_ISA_ZICBOM is not set
# CONFIG_RISCV_ISA_ZICBOZ is not set
# CONFIG_RISCV_ISA_ZAWRS is not set
# CONFIG_RISCV_ISA_ZBA is not set
# CONFIG_RISCV_ISA_ZBB is not set
# CONFIG_RISCV_ISA_ZBC is not set
```

无调试项：

```text
# CONFIG_DEBUG_KERNEL is not set
# CONFIG_DEBUG_ATOMIC_SLEEP is not set
# CONFIG_DEBUG_LIST is not set
# CONFIG_DEBUG_MUTEXES is not set
# CONFIG_DEBUG_SPINLOCK is not set
# CONFIG_DEBUG_OBJECTS is not set
# CONFIG_DEBUG_PREEMPT is not set
# CONFIG_SCHED_DEBUG is not set
# CONFIG_LATENCYTOP is not set
# CONFIG_KALLSYMS is not set
```

说明：

- Sstc 没有独立配置开关，属于运行时扩展；rremu 不实现 `stimecmp`，明确不使用。
- 若某个选项被核心依赖强制打开，记录实际值，不视为配置漂移。
- 不启用 `initcall_debug`。

#### 3.3.1 已接受的 forced selects（A+）

决策：**A+，接受当前完整冻结 `.config`，不再继续与 Kconfig 依赖裁剪对抗。**

以下选项在 `rv32_defconfig` + 固定 fragment 后仍被启用，已确认接受：

```text
CONFIG_RISCV_APLIC=y
CONFIG_RISCV_IMSIC=y
CONFIG_INPUT=y
CONFIG_DEBUG_KERNEL=y
CONFIG_VIRTIO=y
CONFIG_PINCTRL=y
CONFIG_GPIOLIB=y
CONFIG_OVERLAY_FS=y
CONFIG_AUTOFS_FS=y
```

构建日志中还会出现其他由 defconfig/select 带入的核心子系统（如 RPMSG、IIO、
EXTCON、NVMEM、MAILBOX、DEVFREQ 等）；它们同样按 A+ 接受，不视为 rremu
实际具备对应设备。

处理约定：

- 实际可使用的设备仍只有：UART、RTC、syscon、PLIC、CLINT（M/OpenSBI）、RAM。
- `RISCV_APLIC` / `RISCV_IMSIC`：RISC-V 架构 `select` 强制项；DTS 不提供对应节点。
- `INPUT` / `PINCTRL` / `GPIOLIB`：无对应 input/pinctrl/gpio DTS 设备。
- `VIRTIO=y` 只是核心依赖项；`VIRTIO_MMIO` / `VIRTIO_BLK` / `VIRTIO_CONSOLE` 仍关闭，
  S3.4a 不使用 virtio。
- `OVERLAY_FS` / `AUTOFS_FS` 不参与 initramfs 启动路径。
- `DEBUG_KERNEL` 仅配置开关保留；不启用 `initcall_debug` 或其他主动调试选项。
- 以上均为 `forced accepted`，不属于 rremu 实际设备能力。
- 完整冻结配置保存在 `platforms/linux-test/rremu_rv32_test.config`。

#### 3.3.2 已关闭 jitterentropy / DRBG 链

启动定位确认 SHA3 热点来自 `jent_hash_time`（jitterentropy），不是 crypto self-test。
为避免启动期熵源初始化消耗大量指令，固定关闭：

```text
# CONFIG_CRYPTO_SEQIV is not set
# CONFIG_CRYPTO_ECHAINIV is not set
# CONFIG_CRYPTO_RNG_DEFAULT is not set
# CONFIG_CRYPTO_DRBG_MENU is not set
# CONFIG_CRYPTO_DRBG is not set
# CONFIG_CRYPTO_DRBG_HMAC is not set
# CONFIG_CRYPTO_JITTERENTROPY is not set
```

原因：`CRYPTO_RNG_DEFAULT → CRYPTO_DRBG_MENU → CRYPTO_DRBG → CRYPTO_JITTERENTROPY`
存在 Kconfig select 链；仅关闭 jitterentropy 不会生效，需要同时关闭
`CRYPTO_SEQIV` / `CRYPTO_ECHAINIV` 这两个 `CRYPTO_RNG_DEFAULT` 的来源。

该变更不影响当前 initramfs `/init` 的 write/reboot 路径。

### 3.4 固定 DTS

使用：

```text
platforms/rremu-linux.dts
```

固定条件：

```text
RAM: 128MiB
DTB/FDT 地址: 0x87f00000
bootargs: console=ttyS0 earlycon keep_bootcon
```

审核发现的不一致：

- 当前 RTC 节点声明了 `interrupts = <11>`，但 rremu 没有 RTC alarm IRQ。
- S3.4a 固定 DTS 中应移除 RTC 的 `interrupts` 和 `interrupt-parent`。
- Linux 只使用 RTC 读时间，不依赖 RTC 中断。

### 3.5 固定 OpenSBI 构建

```sh
cd /home/yori/yori-riscv-util/opensbi

make clean
make PLATFORM=generic \
  CROSS_COMPILE=riscv64-linux-gnu- \
  PLATFORM_RISCV_XLEN=32 \
  FW_PAYLOAD=y \
  FW_PAYLOAD_PATH=/home/yori/yori-riscv-util/linux/arch/riscv/boot/Image \
  FW_PAYLOAD_OFFSET=0x200000 \
  FW_PAYLOAD_ALIGN=0x1000 \
  FW_PAYLOAD_FDT_ADDR=0x87f00000
```

产物：

```text
build/platform/generic/firmware/fw_payload.elf
```

### 3.6 固定运行命令

```sh
cd /home/yori/yori-riscv-util/rremu

timeout -s INT 5 ./target/release/rremu \
  ../opensbi/build/platform/generic/firmware/fw_payload.elf \
  --load-base 0x80000000 \
  --dtb platforms/rremu-linux.dtb \
  -m 128M \
  --no-fb \
  -c c \
  > /tmp/linux-test.log 2>&1
```

固定规则：

- 单次测试硬上限：**5 秒**
- 不使用 `initcall_debug`
- 使用 CLI UART 直出捕获日志
- 只有确认能稳定越过 5 秒，才扩到 10 秒
- 扩展阶梯：`5s → 10s → 20s → 30s → 60s`

## 4. 失败分类

每次 5 秒测试记录：

1. 最后 5 行日志
2. 最后 kernel timestamp
3. 如果暂停：
   - `pc`
   - `priv`
   - `mcause`
   - `mcycle` / `minstret`

判定分类：

| 现象 | 优先怀疑 |
|---|---|
| 停在 initcall 且 jiffies 不前进 | SBI timer / S 中断 / CLINT |
| 无 `ttyS0` / 用户态无输出 | UART 8250 子集不完整 |
| 有 init 消息但不退出 | syscon poweroff / SBI SRST |
| page fault / access fault | Sv32 / PMP |
| 停在 initramfs unpack | initramfs / 文件系统配置 |
| `pc` 不变但 `instret` 增长 | 总线/设备返回导致死循环 |

## 5. 分阶段实施

### P0：固定测试资产

- 建立 `platforms/linux-test/`
- 固定 init.c
- 固定 initramfs 构建脚本
- 保存完整 `.config`
- 固定 build/run 脚本

完成条件：同一命令可重复生成相同 Image 和 payload。

### P1：达到 `/init`

- 用 5 秒测试
- 确认是否出现：
  ```text
  rremu: linux userspace init reached
  ```
- 如果未出现，记录最后日志和 PC，按第 4 节分类

### P2：确认 UART console

- 确认 8250 驱动 probe
- 确认 `ttyS0` 注册
- 确认用户态 write 能到 stdout
- 如果失败，补 UART 的 IIR/FCR/DLAB/MCR 语义，不换驱动

### P3：确认 timer

- 检查 `mtime`、`mtimecmp`
- 检查 `mip.STIP`、`mie.STIE`、`mideleg.STI`
- 检查 OpenSBI timer event 是否为空
- 修复 SBI timer / S 中断注入 / CLINT 64 位 mtimecmp

### P4：确认 poweroff

- 检查 syscon 是否被 Linux probe
- 检查 DTS 的 `syscon-poweroff`
- 检查 SBI SRST
- 必要时让 init 通过 `/dev/mem` 直接写 syscon

### P5：S3.4a 验收与合并

验收：

1. Linux 启动日志正常
2. 出现 init 消息
3. rremu 在测试时限内退出
4. `rremu-test` 50 用例通过
5. OpenSBI smoke 通过

达标后才合并 S3.4a。

### P6：S3.4b virtio

S3.4a 通过后：

- virtio-mmio
- virtio-blk
- rootfs
- 用户态 shell

## 6. 分支与提交规则

- S3.4a 在 `test/s3.4-linux-devices` 分支完成
- 每个修改单独最小提交
- 每个修复必须有对应测试或日志证据
- `main` 变化后，S3.4 分支 rebase
- S3.4a 未验收前不合并

## 7. 配置冻结规则

从本文件开始：

1. 第 3 节配置为固定基线。
2. 任何配置项变更必须说明：
   - 当前失败证据
   - 为什么必须改
   - 预期影响
3. 变更前等待指令。
4. 不临时切换 defconfig。
5. 不开启大范围 debug。
6. 不扩大 5 秒测试上限，除非确认已稳定越过当前上限。

## 8. 当前待办

- [ ] 将 initramfs 与配置脚本移入 `platforms/linux-test/`
- [ ] 移除 DTS 中 RTC 的 IRQ 属性
- [ ] 固定并保存完整 `.config`
- [ ] 固定 bootargs
- [ ] 用 5 秒协议确认 Linux 停止点
- [ ] 按失败分类单点修复
- [ ] 达到 init 与 poweroff
- [ ] 合并 S3.4a
- [ ] 开始 S3.4b virtio
