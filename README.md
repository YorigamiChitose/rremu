# rremu

> ⚠️ **本项目为 vibe coding 产物**：代码主要由 AI 在人工需求约束与审查下生成，
> 仅供学习/实验用途，不保证工程质量与长期维护。请以源码为准。

一个 **RV32IMC_Zicsr（最简 M-mode）单周期 RISC-V 模拟器与调试器**，用 Rust 写成。
提供 GDB 风格 CLI 调试器与 ratatui TUI 两种前端，设备经 trait 可插拔。

## 特性

- **ISA**：RV32I + M（乘除）+ C（压缩指令）+ Zicsr；最简 M-mode 陷阱
  （ecall / ebreak / mret，mstatus / mtvec / mepc / mcause / mie / mip）
- **执行模型**：单周期（一条指令 = 一个 cycle）
- **前端**
  - CLI：rustyline 交互式 REPL，加载 ELF 后暂停在入口，`c` 开始运行
  - TUI：ratatui 多面板（寄存器 / 反汇编 / 内存事件 / UART）+ 键盘控制
  - 独立帧缓冲窗口（minifb，X11），**懒启动**：首次写 fb 才弹窗
- **设备**：RAM、UART（NS16550A 子集）、CLINT（mtime/mtimecmp/msip）、
  Framebuffer（RGB565 → 0x00RRGGBB），均经 `BusDevice` trait 接入总线
- **调试能力**：断点、监视点、表达式求值（ABI 寄存器名 / `*addr` 解引用 /
  C 风格运算符）、无副作用内存 peek、事件环
- **退出约定**：`ebreak`/`c.ebreak` 为退出信号通道，模拟器以 `a0`（x10）为退出码终止并发出 `EmuEvent::Halt { code }`

## 架构

Cargo workspace 多 crate + trait 依赖倒置；前端只能经 `rremu-machine` 公开
API 交互，`bin` 是唯一 composition root：

```
bin (rremu)            clap 解析、设备地址分配、前端选择（唯一组装点）
├── rremu-cli          REPL 调试器（rustyline + rremu-expr）
├── rremu-tui          TUI（ratatui + crossterm）
└── rremu-window       独立窗口（minifb）
rremu-machine          Machine/Builder/快照/断点/监视点/事件环
├── rremu-cpu          CPU 状态与执行循环、trap
├── rremu-devices      ram / uart / clint / framebuffer
└── rremu-loader       ELF32 解析（零 IO）
rremu-bus              BusDevice/Bus 路由、EmuEvent/EventSink
rremu-isa              类型、指令枚举、CSR 常量、反汇编 Display
rremu-decode           解码（纯函数）
rremu-expr             表达式词法/解析/求值（独立叶 crate）
```

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
| `help` / `q` | 帮助 / 退出 |

### TUI 按键

| 按键 | 功能 |
|---|---|
| `Space` | 运行 / 暂停 |
| `s` | 单步一条 |
| `f` | 直达全速档 |
| `←` / `→` | 切换速度档（1/2/4/8/16/32/64/full） |
| `i` | 进入 UART 输入模式（`Esc` 返回；输入模式下 Ctrl+C 仍可退出） |
| `b` | 切换内嵌 framebuffer 视图 |
| `r` | 复位机器 |
| `q` / `Ctrl+C` | 退出 |

帧节奏约 30fps：步进档每帧执行固定批数，全速档在帧预算内连续批量执行。

## 设备内存地图

| 基址 | 设备 | 范围 |
|---|---|---|
| `0x0200_0000` | CLINT | 0xC000（msip / mtimecmp / mtime） |
| `0x1000_0000` | UART | 8 字节 |
| `0x2000_0000` | Framebuffer | 640×480 ×2 字节（RGB565） |
| `0x8000_0000` | RAM | 默认 16 MiB |

## 测试

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

仓库内测试均为纯内存单测（无外部工具链依赖）。固件级验证（ebreak 退出、UART 输出、CLINT 中断、riscv-tests 等）由独立的外部测试项目承担。
