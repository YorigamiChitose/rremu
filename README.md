# rremu

> ⚠️ **本项目为 vibe coding 产物**：代码主要由 AI 在人工需求约束与审查下生成，
> 仅供学习/实验用途，不保证工程质量与长期维护。请以源码为准。

一个 **RV32IMAC_Zicsr（M/S/U 三级特权）单周期 RISC-V 模拟器与调试器**，用 Rust 写成。
提供 GDB 风格 CLI 调试器与 ratatui TUI 两种前端，设备经 trait 可插拔。

## 特性

- **ISA**：RV32I + M（乘除）+ A（原子 lr/sc + AMO）+ C（压缩指令）+ Zicsr；
  **M/S/U 三级特权**，`mret`/`sret`、`sfence.vma`/`wfi`（按 NOP）；
  `mstatus`（MPP/MPIE/MIE、SPP/SPIE/SIE）、`mepc/mcause/mtval`、`mie/mip`、
  `medeleg/mideleg` 陷阱委派、**PMP**（`pmpcfg/pmpaddr`，S/U 强制访问控制）、
  S-mode CSR（sstatus/sie/sip/stvec/sscratch/sepc/scause/stval/satp）；
  `fence.i` 按无缓存实现为 NOP
- **执行模型**：单周期（一条指令 = 一个 cycle）
- **前端**
  - CLI：rustyline 交互式 REPL，加载 ELF 后暂停在入口，`c` 开始运行
  - TUI：ratatui 多面板（寄存器 / 反汇编 / 内存事件 / UART）+ 键盘控制
  - 独立帧缓冲窗口（minifb，X11），**懒启动**：首次写 fb 才弹窗
- **设备**：RAM、UART（NS16550A 子集）、CLINT（mtime/mtimecmp/msip）、
  PLIC、Goldfish RTC、syscon（poweroff/reboot）、Framebuffer（RGB565 → 0x00RRGGBB），
  均经 `BusDevice` trait 接入总线。
  CLINT `mtimecmp` 复位为 `u32::MAX`（避免虚假定时器中断）；
  `mip` 的 MSIP/MTIP/MEIP 位由设备 pending 投影（软件只读）
- **调试能力**：断点、监视点、表达式求值（ABI 寄存器名 / `*addr` 解引用 /
  C 风格运算符）、无副作用内存 peek、事件环
- **退出约定**：M-mode `ecall` 且 `a7 == 0xCAFE_0000`（`EXIT_MAGIC`）为退出信号通道，模拟器以 `a0`（x10）为退出码终止并发出 `EmuEvent::Halt { code }`；syscon poweroff 发出 `EmuEvent::PowerOff`（退出码 0），reboot 发出 `EmuEvent::Reboot`（退出码 2）。其余 `ecall`（含 `a7 == -1` 的 AM yield）走标准 ecall-from-M 陷阱；`ebreak`/`c.ebreak` 走标准 breakpoint 陷阱

## 当前状态

已完成：

- RV32IMAC + Zicsr，M/S/U 三级特权
- Sv32 页表、S 级中断、PMP 强制访问控制
- OpenSBI v1.9 启动
- Linux 6.12 RV32 + 内置 initramfs 启动
- `/init` 用户态、UART 输出、syscon poweroff
- rremu-test 端到端回归 54 个用例

当前没有进行中的阶段规划或路线图。

## 已知限制

- vectored `mtvec`/`stvec` 未实现；`cte_vectored_mtvec` 按 Fault 断言。
- `wfi` 按 NOP 执行。
- 不提供磁盘、真实块设备、virtio-blk 或磁盘 rootfs；Linux 使用内置 initramfs。
- Goldfish RTC 仅实现时间/alarm 寄存器，不注入 alarm IRQ。
- syscon `0x5555` poweroff 退出码为 0；`0x7777` reboot 退出码为 2。

## 架构

Cargo workspace 多 crate + trait 依赖倒置；前端只能经 `rremu-machine` 公开
API 交互，`bin` 是唯一 composition root：

```
bin (rremu)            clap 解析、配置解析、设备映射、前端选择（唯一组装点）
├── rremu-cli          REPL 调试器（rustyline + rremu-expr）
├── rremu-tui          TUI（ratatui + crossterm）
└── rremu-window       独立窗口（minifb）
rremu-config           统一配置：默认值 + Overrides + resolve（唯一真相源，叶 crate）
rremu-machine          Machine/快照/断点/监视点/事件环/dump
├── rremu-cpu          CPU 状态与执行循环、trap
├── rremu-devices      ram / uart / clint / framebuffer
└── rremu-loader       ELF32 解析（零 IO）
rremu-bus              BusDevice/Bus 路由、EmuEvent/EventSink
rremu-isa              类型、指令枚举、CSR 常量、反汇编 Display
rremu-decode           解码（纯函数）
rremu-expr             表达式词法/解析/求值（独立叶 crate）
```

## 配置

`rremu-config` 集中 rremu 侧所有硬编码值与可变参数，是**唯一真相源**：

- **默认值**（设备基址/尺寸、Framebuffer、诊断容量）以 `pub const` 集中定义；
- **外部参数**（`--base` / `--mem` / `--entry` / `--no-fb` / `--dump-on-exit`）
  经纯数据 `Overrides` 传入，由 `Config::resolve` 合并默认值并校验
  （`ConfigError`：RAM 为 0/溢出、FB 维度为 0/溢出）；
- `Machine::from_config(&Config, image)` 据此映射设备并构造机器；CLI/TUI 前端
  亦从 `Config` 读取路径等；`rremu-config` **不依赖 clap**。

`rremu-am/arch.rs` 为**人工镜像**（guest 侧独立 `no_std` 仓库，不依赖本 crate），
两边一致性由 `rremu-test` 的护栏测试锁定。

## 构建要求

- Rust **1.99+**，edition 2024
- Linux/X11（minifb 仅启用 x11 后端；无显示环境时窗口自动降级为无窗模式）
- `unsafe_code = "forbid"`

```sh
cargo build --release
# 产物：target/release/rremu
```

## 快速开始

```sh
# 交互 REPL（启动暂停在入口）
rremu <ELF>
rremu> c

# 批处理
rremu <ELF> -c c

# TUI
rremu --tui <ELF>
```

### 命令行参数

| 参数 | 说明 |
|---|---|
| `<ELF>` | ELF32 文件路径 |
| `-t, --tui` | 启动 TUI（与 `-c` 互斥） |
| `-c, --cmd <CMD>` | 批处理命令，可重复 |
| `-m, --mem <SIZE>` | RAM 大小，K/M/G 后缀（默认 16M） |
| `--base <ADDR>` | RAM 基址（默认 0x80000000） |
| `--entry <ADDR>` | 覆盖 ELF 入口 |
| `--no-fb` | 不映射 framebuffer 设备，也不开窗 |
| `--dump-on-exit <FILE>` | 程序终止时自动写出机器状态 dump（JSON，默认关闭） |
| `--dtb <FILE>` | 加载设备树 DTB 到 RAM 并以 boot `a1` 传给 guest（OpenSBI/Linux 需要） |
| `--load-base <ADDR>` | 为 PIE/可重定位镜像指定加载基址（段与入口整体偏移；OpenSBI 用） |
| `-v...` | 日志级别：-v info / -vv debug / -vvv trace |

### REPL 命令

| 命令 | 说明 |
|---|---|
| `c` | 继续运行（退出 / 陷入 / 断点 / Ctrl+C 时停） |
| `si [N]` | 单步 N 条指令（默认 1） |
| `x N ADDR` | 查看 ADDR 起 N 个字（无副作用 peek，支持非对齐） |
| `p EXPR` | 求值表达式（如 `p x5+4`、`p *0x80000000`） |
| `b EXPR` | 在表达式求得的地址设断点 |
| `w EXPR` | 监视表达式，值变化时停下 |
| `d N` | 删除第 N 个断点/监视点 |
| `info r` | 查看寄存器与 CSR |
| `info w` | 列出断点与监视点 |
| `uart` | 进入 UART 输入模式（按键原样映射为字节注入 RX，`Esc` 返回；输入模式下 Ctrl+C 仍传给程序为 0x03） |
| `dump FILE` | 写出结构化机器状态 dump（JSON）到 FILE |
| `help` / `q` | 帮助 / 退出 |

CLI 模式下 UART 输出会实时直写 stdout，不再等待暂停；可直接重定向保存：

```sh
rremu firmware.elf -c c > uart.log
```

### TUI 按键

| 按键 | 功能 |
|---|---|
| `Space` | 运行 / 暂停 |
| `s` | 单步一条 |
| `f` | 直达 full 档 |
| `←` / `→` | 切换速度档（single/slow/middle/fast/full） |
| `i` | 进入 UART 输入模式（`Esc` 返回；输入模式下 Ctrl+C 仍可退出） |
| `b` | 切换内嵌 framebuffer 视图 |
| `p` | 循环面板信息档：full → no-insn → no-insn,io → uart-full（全屏 UART） |
| `D` | 写 dump（需配置 `--dump-on-exit` 路径） |
| `r` | 复位机器 |
| `q` / `Ctrl+C` | 退出 |

帧节奏约 30fps：`single` 每秒执行 1 条；`slow/middle/fast/full` 分别按帧预算的
25%/50%/75%/100% 全速执行，即约为峰值速度的四档。
顶栏 `ips` 显示最近 1s 的实测执行速度（instructions/s，K/M/G 缩写）；暂停时保留最后测量值。

## 设备内存地图

| 基址 | 设备 | 范围 |
|---|---|---|
| `0x0200_0000` | CLINT | 0xC000（msip / mtimecmp / mtime） |
| `0x0010_0000` | syscon/test | 0x1000（poweroff/reboot） |
| `0x0010_1000` | Goldfish RTC | 0x1000 |
| `0x0c00_0000` | PLIC | 0x400000（priority/pending/enable/claim） |
| `0x1000_0000` | UART | 8 字节 |
| `0x2000_0000` | Framebuffer | 640×480 ×2 字节（RGB565） |
| `0x8000_0000` | RAM | 默认 16 MiB |

## 运行外部固件（OpenSBI）

rremu 已具备运行 **OpenSBI v1.9（RV32 generic FW_JUMP）** 的能力（S-mode 启动、PMP、委派）。
平台定义与一键脚本在 [`platforms/`](platforms/)：

```sh
cd platforms && ./run-opensbi.sh        # 或 --tui
```

平台说明见 [`platforms/README.md`](platforms/README.md)，端到端测试见 [`../rremu-test`](../rremu-test)。

Linux 启动脚本（需要 RV32 Linux Image，建议内置 initramfs）：

```sh
# 构建固定 initramfs 和 Linux Image
./platforms/linux-test/build-linux.sh

# 构建 DTB/OpenSBI FW_PAYLOAD 并启动 Linux
./platforms/run-linux.sh        # 或 --tui
```

成功时退出码为 0，并输出：

```text
rremu: linux userspace init reached
reboot: Power down
Program exited with code 0.
```

## 测试

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

仓库内测试均为纯内存单测（无外部工具链依赖）。固件级验证（ecall+a7=magic 退出、UART 输出、CLINT 中断等端到端场景）由独立的外部测试项目 `rremu-test`（配合 HAL 库 `rremu-am`）承担。

### 结果 dump

rremu 支持把终止时的机器状态序列化为结构化 JSON（`format_version=1`）：
寄存器/CSR（含 `mtval`）、最近 32 条指令 trace（trap 时冻结为崩溃路径）、
最近 64 条 IO 事件、终止原因、最近 trap、ELF 引用（路径 + sha256）。触发方式：

- `dump FILE`（REPL 命令；批量模式下程序退出后仍可执行）
- `--dump-on-exit FILE`（启动参数）：程序**停止**（退出/故障/挂起）时自动写；
  TUI 模式下同样生效，另可按 `D` 手动写

dump 由 `rremu-report`（rremu-am workspace 内的工具）解析为人类可读报告
（退出码语义化、mcause 名称化、寄存器/地址符号化、trace 反汇编）；
report 会校验 `format_version`，并在 ELF sha256 与 dump 记录不符时告警。
全程不依赖串口输出。
