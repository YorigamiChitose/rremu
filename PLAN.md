# rremu 项目规划

> 版本 v1.0 · 定稿日期 2026-10-08
> 来源：与需求方逐项讨论确认。所有决策均已核对，本文档是架构与实现的依据；任何变更先改本文档再改代码。

---

## 1. 项目概述

**一句话定位**：rremu 是一个 RV32IMC_Zicsr（含最简 M-mode）的单周期 RISC-V 模拟器，提供 GDB 风格 CLI 调试器与 TUI 可视化调试两种前端，设备模型（串口、屏幕、定时器等）可插拔扩展。

**强制约束**：架构必须模块化、尽可能解耦，为后续扩展留可能性（需求方原话）。落地为 Cargo workspace 多 crate + trait 依赖倒置。

### 1.1 目标

| 维度 | 内容 |
|---|---|
| ISA | RV32I + M + C + Zicsr，最简 M-mode 陷阱（ecall/ebreak/mret，mstatus/mtvec/mepc/mcause/mie/mip） |
| 执行模型 | 单周期：一条指令 = 一个 cycle |
| CLI | 交互式调试器 REPL（rustyline），启动暂停在入口，`c` 开始运行 |
| TUI | ratatui 多面板实时视图 + 键盘控制 |
| 设备 | RAM、UART（NS16550A 子集）、CLINT、Framebuffer，经 `BusDevice` trait 插拔 |
| 屏幕 | TUI 内嵌渲染 + minifb 独立窗口，两者都支持 |
| 退出约定 | `tohost` 符号写入（riscv-tests 惯例）+ ecall |

### 1.2 非目标（明确不做，仅预留扩展点）

- RV64、A/F/D 扩展、S-mode、MMU/Sv32、PLIC、virtio
- 性能优化（JIT/块翻译）、多 hart（machine 层预留多实例可能）
- 反汇编库（decode 自带 Display 即可）

---

## 2. 技术决策总表

| # | 决策项 | 结论 | 定稿时间 |
|---|---|---|---|
| D1 | ISA | RV32IMC_Zicsr + 最简 M-mode；RV64/完整特权级仅预留 | 10-08 |
| D2 | CLI 定位 | GDB 风格 REPL，暂停在入口 | 10-08 |
| D3 | TUI | ratatui；面板与按键见 §7 | 10-08 |
| D4 | 特权级 | 最小 M-mode（为特权级显示与裸机固件） | 10-08 |
| D5 | 串口输入分流 | TUI 中 `i` 进入输入模式，Esc 返回 | 10-08 |
| D6 | 调速 | 固定档位 ←/→（1/2/4/8/16/32/64/全速），f 直达全速 | 10-08 |
| D7 | 启动行为 | 加载 ELF → 暂停入口 → 提示符 | 10-08 |
| D8 | 断点 | 加 `b addr`（addr 支持表达式） | 10-08 |
| D9 | 表达式语法 | `x5`/ABI 别名 + `*addr` 解引用；C 优先级 | 10-08 |
| D10 | 运行中断 | Ctrl+C 暂停回提示符（ctrlc crate） | 10-08 |
| D11 | 架构 | workspace 多 crate + trait 依赖倒置 | 10-08 |
| D12 | peek 读 | 表达式/监视点/x 走无副作用 `peek_read` | 10-08 |

---

## 3. 总体架构

```
┌─────────────────────────────────────────────────────────────┐
│  bin (rremu) —— clap 解析 · composition root（唯一组装点）    │
│     ├── rremu-cli   REPL 调试器（rustyline + rremu-expr）    │
│     ├── rremu-tui   TUI 视图（ratatui）                      │
│     └── rremu-window 独立窗口（minifb）                      │
├─────────────────────────────────────────────────────────────┤
│  rremu-machine —— Machine/Builder/快照/控制/断点/监视点       │
├──────────────┬───────────────────────┬──────────────────────┤
│  rremu-cpu   │  rremu-devices        │  rremu-loader        │
│  状态+执行    │  ram/uart/clint/fb    │  ELF 解析（零 IO）    │
├──────────────┴───────────────────────┴──────────────────────┤
│  rremu-bus —— BusDevice/Bus trait · EmuEvent/EventSink      │
├─────────────────────────────────────────────────────────────┤
│  rremu-isa（指令/寄存器/CSR 定义）   rremu-decode（纯函数解码）│
│  rremu-expr（表达式 AST+求值，独立叶 crate）                  │
└─────────────────────────────────────────────────────────────┘
```

### 3.1 crate 依赖方向（edge = "依赖"）

```
bin → cli, tui, window, machine, loader, expr(间接), clap, anyhow, ctrlc, tracing-subscriber
cli → machine, expr, rustyline, tracing
tui → machine, ratatui, crossterm
window → minifb, thiserror
machine → cpu, bus, devices, loader, expr, thiserror, tracing
devices → bus, thiserror, tracing
cpu → isa, decode, bus, thiserror, tracing
decode → isa, thiserror
bus → thiserror
isa → （无）
expr → （无）
```

### 3.2 依赖红线（违反即打回）

1. `isa` / `bus` / `expr` / `loader` 零外部依赖（thiserror 除外）且不做任何 IO
2. `cpu` 只依赖 `bus` 中的 trait，不得 import 任何具体设备
3. `devices` 不依赖 `cpu`
4. 前端（cli/tui/window）只能通过 `machine` 公开 API（快照/控制），不得摸寄存器与总线
5. `bin` 是唯一 composition root：所有 `Box<dyn>` 注入、设备地址分配只发生在这里

---

## 4. Workspace 布局

```
rremu/
├── Cargo.toml                 # [workspace] + [workspace.dependencies] + [workspace.lints]
├── PLAN.md                    # 本文档
├── crates/
│   ├── rremu-isa/             # Word/Address 类型、Instruction 枚举、CSR 常量、Priv、反汇编 Display
│   ├── rremu-decode/          # instr_len / decode16 / decode32，纯函数
│   ├── rremu-bus/             # BusDevice trait、Bus 路由、EmuEvent、EventSink、peek 语义
│   ├── rremu-cpu/             # Cpu 状态 + step 执行循环 + trap
│   ├── rremu-devices/         # ram / uart、clint / framebuffer
│   ├── rremu-loader/          # Image/Segment，goblin 解析 ELF32 + tohost 符号提取
│   ├── rremu-expr/            # 词法/递归下降/AST 求值 + EvalContext trait
│   ├── rremu-machine/         # MachineBuilder/Machine/RunResult/MachineSnapshot/断点监视点
│   ├── rremu-cli/             # REPL：命令解析与分发（rustyline）
│   ├── rremu-tui/             # TUI
│   └── rremu-window/          # 独立窗口
├── bin/                       # crate 名 rremu：main.rs + clap Args + 前端选择
├── tests/                     # 集成测试 + 固件缓存
│   ├── data/                  # hello.elf / rv32ui-p-add.elf 等（脚本生成，gitignore）
│   └── firmware/              # hello.S、链接脚本
└── tools/
    ├── build-firmware.sh      # 编译 hello.S → tests/data/hello.elf
    └── build-tests.sh         # 拉取并构建官方 riscv-tests 的 3 个代表测试
```

**lint 基线**（workspace 统一）：`unsafe_code = "forbid"`；库 crate `clippy::unwrap_used = "deny"`、`clippy::expect_used = "warn"`；edition 2024 / rust-version 1.99。

---

## 5. 模块详细设计

### 5.1 rremu-isa

- 类型：`pub type Word = u32; pub type Address = u32;`
- `Priv` 枚举：`User` / `Machine`（最简 M-mode，仅 U/M 两级；mstatus.MPP 只支持 00/11）
- `Instruction` 枚举，覆盖：
  - RV32I（40）：lui auipc jal jalr；beq bne blt bge bltu bgeu；lb lh lw lbu lhu；sb sh sw；addi slti sltiu xori ori andi slli srli srai add sub sll slt sltu xor srl sra or and；fence ecall ebreak
  - RV32M（8）：mul mulh mulhsu mulhu div divu rem remu
  - RV32C（26）：c.addi4spn c.lw c.sw；c.addi c.jal c.li c.addi16sp c.lui c.srli c.srai c.andi c.sub c.xor c.or c.and c.j c.beqz c.bnez；c.slli c.lwsp c.jr c.mv c.ebreak c.jalr c.add c.swsp
  - Zicsr（6）：csrrw csrrs csrrc csrrwi csrrsi csrrci
- CSR 地址常量：mstatus(0x300) misa(0x301) mie(0x304) mtvec(0x305) mscratch(0x340) mepc(0x341) mcause(0x342) mtval(0x343) mip(0x344) mcycle(0xB00) minstret(0xB02) mhartid(0xF14, RO)
- `impl Display for Instruction`：`addi sp, sp, 16` / `c.lwsp a0, 4(sp)` 格式（反汇编复用）
- ABI 别名表（zero/ra/sp/gp/tp/t0-t6/s0-s11/a0-a7/fp）用于 Display
- 验收：Display 单测 ≥ 6 例覆盖各类格式

### 5.2 rremu-decode

```rust
pub fn instr_len(first_half: u16) -> usize;              // 低 2 位 == 11 ? 4 : 2
pub fn decode32(word: u32, pc: Address) -> Result<isa::Instruction, DecodeError>;
pub fn decode16(hw: u16, pc: Address) -> Result<isa::Instruction, DecodeError>;
#[derive(Debug, Error)] pub enum DecodeError { Illegal { bits: u32, pc: u32 }, Unimplemented { bits: u32, pc: u32 } }
```
- 取指侧只看 `instr_len(first_half)` 决定取 2 还是 4 字节，32 位指令跨 4 字节边界由 cpu 层拼接，decode 保持纯函数
- 验收：RV32I 40 条每类至少 1 条单测；M/C/Zicsr 指令同步覆盖；非法 opcode 报 `Illegal`

### 5.3 rremu-bus

```rust
pub enum AccessSize { Byte, Half, Word }   // RV64 预留：DoubleWord（8 字节）
pub enum BusError { Unmapped { addr: u32 }, Misaligned { addr: u32, size: AccessSize }, Device { addr: u32, msg: String }, PeekUnsupported { addr: u32 }, Overlap { start, end } }

// 事件经 data_read/data_write/inst_read 的 sink 参数贯穿：cpu 把 sink 传给 bus，设备直接上报 UartOutput 等
// （单一事件流，避免设备持有 sink 引发的共享所有权）
pub trait BusDevice {
    fn read(&mut self, addr: u32, size: AccessSize, sink: &mut dyn EventSink) -> Result<u32, BusError>;   // 可有副作用
    fn write(&mut self, addr: u32, size: AccessSize, data: u32, sink: &mut dyn EventSink) -> Result<(), BusError>;
    fn peek_read(&mut self, addr: u32, size: AccessSize) -> Result<u32, BusError>; // 必须无副作用，设备各自实现，无默认实现
    fn write_bytes(&mut self, addr: u32, data: &[u8]) -> Result<(), BusError>;     // 镜像加载，无事件
    fn device_name(&self) -> &str;
    fn as_any_mut(&mut self) -> &mut dyn Any;                                       // machine 层安全下取具体设备
    fn check_interrupt(&self) -> Option<u32> { None }                               // 中断源设备覆写
    fn tick(&mut self, cycle: u64) {}                                               // cycle 下发（CLINT mtime）
}
pub trait Bus { // 依赖倒置：cpu 只依赖此 trait；对外只暴露三个语义访问方法
    fn inst_read(&mut self, addr: u32, size: AccessSize, sink: &mut dyn EventSink) -> Result<u32, BusError>;   // 取指原语：单次对齐物理读，发一条 IF（size 由 CPU/IFU 给定）；不含跨 XLEN 策略
    fn data_read(&mut self, addr: u32, size: AccessSize, sink: &mut dyn EventSink) -> Result<u32, BusError>;
    fn data_write(&mut self, addr: u32, size: AccessSize, data: u32, sink: &mut dyn EventSink) -> Result<(), BusError>;
    fn peek(&mut self, addr: u32, size: AccessSize) -> Result<u32, BusError>;      // 无副作用无事件
    fn write_bytes(&mut self, addr: u32, data: &[u8]) -> Result<(), BusError>;
    fn device_mut(&mut self, addr: u32) -> Option<&mut dyn BusDevice>;
    fn set_watch(&mut self, addr: Option<u32>);                                     // tohost 监视
    fn set_cycle(&mut self, cycle: u64);
    fn pending_interrupt(&mut self) -> Option<u32>;                                 // cause 降序聚合（MEI>MTI>MSI）
}
// MemKind 与三个语义方法一一对应；Display：Inst→IF、DataRead→DR、DataWrite→DW
pub enum MemKind { Inst, DataRead, DataWrite }
pub struct BusRouter { /* Vec<Region{Range, Box<dyn BusDevice>}>，map() 检查重叠；
                          私有助手 route_read/route_write 承担对齐+路由+事件（三个语义方法均为单次物理访问原语）；
                          watch 命中且 data&1==1 → 发 Halt{code: data>>1} */ }

pub enum EmuEvent {
    MemAccess { cycle: u64, kind: MemKind /* Inst|DataRead|DataWrite */, size: AccessSize, addr: u32, value: u32, device: String },
    UartOutput { byte: u8 }, UartInput { byte: u8 },
    Trap { cause: u32, pc: u32 }, Halt { code: i32 },
}
pub trait EventSink { fn emit(&mut self, ev: EmuEvent); }
```
- 小端；`data_read/data_write` 返回值低位对齐；对齐检查在总线层（Word/4、Half/2）
- `peek_read` 无默认实现：强制每个设备显式声明无副作用读语义（D12 的落地）

**取指位宽与跨 XLEN 读取（bus 纯净原语 + CPU 取指单元决策；事件位宽与物理读取一致）**
- **职责划分**：`Bus::inst_read(addr, size, sink)` 是**单次对齐物理读原语**——只做"路由 + 按 `size` 对齐校验 + 设备读 + 发一条对应位宽的 `IF` 事件"，不判指令长度、不感知 RVC/XLEN、不拆字不拼合。**跨 XLEN 如何拆取是 CPU/ISA 策略**，由 `Cpu::fetch_word`（取指单元 IFU）决定并连续调用原语完成；总线因此与 ISA 完全解耦。这与"反汇编非对齐拼字 `peek_word_maybe_unaligned` 在 machine 层"一致，也贴合真实硬件（IFU 拆非对齐传输，前端总线只做对齐访问）。
- **RV32 取指编排**（`Cpu::fetch_word`，调用前已保证 `pc` 偶数）：
  - **对齐**（`pc & 3 == 0`）：一次 `inst_read(pc, Word)` → **一条** `IF W`（即便该地址是 16 位 RVC，也按 XLEN 读 4 字节，CPU 用低 16 位判长）。
  - **跨 XLEN**（`pc & 3 == 2`，仅 RVC 允许 2 字节对齐）：32 位指令横跨两个对齐 Word，IFU 调两次 `inst_read(pc, Half)` 与 `inst_read(pc+2, Half)`（各自 2 字节对齐、不跨 Word 边界），**各发一条 `IF H`（共两条）**，在 CPU 内拼成 `lo | (hi << 16)`。
  - `pc` 奇数：CPU 在拆分前直接陷 `INSTR_MISALIGNED`；原语对任何 `size` 对齐违例返 `Misaligned{addr,size}`，CPU 映射为 `INSTR_MISALIGNED`，其余总线错误（Unmapped/Device）映射为 `INSTR_ACCESS_FAULT`。
- 数据访问 `data_read/data_write` 同为单次对齐原语，严格按传入 `AccessSize`（B/H/W）发对应 `size` 的记录；**数据非对齐不拆分**，仍返 `Misaligned`（CPU 陷 LOAD/STORE_MISALIGNED）——跨 XLEN 拆分只用于取指。TUI value 按类型定宽显示且保留前导 0（B=2 位 hex、H=4 位、W=8 位），再用空格右对齐到 W 宽度。
- **RV64 扩展规划（仅预留，暂不实现）**：同一规则随 XLEN=8 泛化——只要 4 字节指令落在一个对齐 XLEN 块内（`pc % 8 ∈ {0,2,4}`）就读一次对齐 `DoubleWord`（一条 `IF DW`）；仅当跨过 8 字节边界（`pc % 8 == 6`）才拆成边界两侧两个对齐 `Half`（两条 `IF H`）。决策仍在 `Cpu::fetch_word`（随 XLEN 参数化），bus 只新增宽度。落地需要：① `AccessSize` 增加 `DoubleWord`；② `EmuEvent::MemAccess.value` 与 `BusDevice::read` 返回值、`data` 入参由 `u32` 扩到 `u64`（或泛型化承载）；③ `data_read/data_write` 支持 `DoubleWord`；④ TUI value 增加 DW=16 位 hex 档位（右对齐宽度随之变为 18 列）；⑤ `peek`/表达式解引用/`x` 命令宽度随 XLEN。扩展点已标记于 `inst_read`/`AccessSize`/`Cpu::fetch_word` 注释。
- 验收：bus 侧——`inst_read` 原语单次单事件（`IF W`/`IF H`）、严格按 `size` 报对齐违例；cpu 侧——对齐取指一条 `IF W`、跨 XLEN 取指两条 `IF H`（addr/value 正确）且指令正确执行（`exec_cross_boundary_32bit_instr`）、奇数 `pc` 陷 `INSTR_MISALIGNED`；加上路由/未映射/注册重叠与 EventSink 收集单测

### 5.4 rremu-cpu

```rust
pub struct Cpu { x: [u32; 32], pc: u32, priv_: Priv, csr: CsrState, cycle: u64 }
impl Cpu {
    pub fn step<B: Bus, E: EventSink>(&mut self, bus: &mut B, sink: &mut E) -> Result<StepOutcome, CpuError>;
}
pub enum StepOutcome { Executed { instr: isa::Instruction, len: u8 }, Trapped { cause: u32 } }
```
- step 流程：中断检查（取指前）→ fetch（`Cpu::fetch_word` 取指单元：对齐一次 `inst_read(Word)`；跨 XLEN 见 §5.3 两次 `inst_read(Half)` 拼合；发 IF 事件）→ decode（低 16 位判长 RVC/32）→ execute → writeback；x0 写忽略；mcycle/minstret 递增
- trap 流程：`mepc ← pc; mcause ← cause; priv_ ← Machine; pc ← mtvec`（mtvec 仅支持 DIRECT 模式，VECTORED 报 Unimplemented）
- mret：`pc ← mepc; priv_ ← mstatus.MPP`；ecall/ebreak/非法指令/地址未对齐 → trap
- 语义要点：全部运算 u32 wrapping；移位量取低 5 位；`mulh` 系列按符号组合实现并有黄金值单测；DIV/REM 除零与溢出按 spec 返回定义值（不 trap）
- 验收：单测（寄存器写回、x0 恒零、分支跳转、trap 进出）

### 5.5 rremu-devices

**RAM**：`Ram::new(base, size)`，Vec<u8> 小端，全宽度读写，peek 等价 read。

**UART**（NS16550A 子集，默认基址 0x1000_0000）：

| offset | 寄存器 | 行为 |
|---|---|---|
| 0 | THR(写) / RDR(读) | 写 → `EmuEvent::UartOutput`；读 → 弹出输入 FIFO |
| 1 | IER | 可写可读，暂不产生中断 |
| 2 | FCR/ISR | 可写可读 |
| 3 | LCR | 可写可读 |
| 5 | LSR | 恒 `0x60`（THR/TSR 空）；若输入 FIFO 非空置 bit0=0x01 |
| 其余 | — | 读 0，写忽略 |

- 输入注入：`Uart::push_input(u8)`（machine 转发）；peek_read RDR 返回队首但不弹出
- 验收：写 THR 触发事件；RDR 读副作用与 peek 无副作用对比单测

**CLINT**（基址 0x0200_0000）：msip(+0x0000)、mtimecmp(+0x4000)、mtime(+0xBFF8)；mtime ≥ mtimecmp 时 `check_interrupt()` 返回 MTI，msip 置位返回 MSI。

**Framebuffer**（基址 0x2000_0000）：固定宽高（默认 640×480，RGB565 → 内部 u32）；字节写入按像素组装；`dirty` 标记 + `frame(&self) -> &[u32]`；peek 直读。

### 5.6 rremu-loader

```rust
pub struct Image { pub segments: Vec<Segment>, pub entry: u32, pub tohost: Option<u32> }
pub struct Segment { pub addr: u32, pub data: Vec<u8> }
pub fn load_elf(bytes: &[u8]) -> Result<Image, LoaderError>;
```
- goblin（`elf32` + `endian_fd`），只接受 RISC-V ELF32 little-endian
- 从 symtab 提取 `tohost` 地址（riscv-tests 必需；无符号则 None）
- 不做文件 IO（读文件在 bin）
- 验收：手写最小 ELF 单测 + hello.elf 真实解析

### 5.7 rremu-expr

文法（递归下降，C 优先级，低→高）：

```
expr    := or
or      := and  ("||" and)*
and     := bitor ("&&" bitor)*
bitor   := bitxor ("|" bitxor)*
bitxor  := bitand ("^" bitand)*
bitand  := eq  ("&" eq)*
eq      := rel  (("=="|"!=") rel)*
rel     := shift (("<"|">"|"<="|">=") shift)*
shift   := add  (("<<"|">>") add)*
add     := mul  (("+"|"-") mul)*
mul     := unary (("*"|"/"|"%") unary)*
unary   := ("-"|"~"|"!") unary | primary
primary := NUMBER | REG | "pc" | "*" unary | "(" expr ")"
NUMBER  := "0x" hex+ | "0b" bin+ | dec+
REG     := "x" 0..31 | ABI 别名
```

求值语义表：

| 运算 | 语义 |
|---|---|
| `+ - *` | u32 wrapping |
| `/ %` | **有符号**（i32，C 语义向零截断）；除零 → `EvalError::DivideByZero`；INT_MIN/-1 按硬件惯例返回原值 |
| `<<` | 左移 wrapping，移位量取低 5 位 |
| `>>` | **逻辑**右移（对齐 RV SRL） |
| `< > <= >=` | **有符号**（i32 比较） |
| `== !=` | 按 u32 |
| `& ^ \|` | 按位 |
| `&& \|\|` | 短路，结果 0/1 |
| `! ~ -`（单目） | 逻辑非 0/1、按位非、wrapping 取负 |
| `*e` | 解引用：先求 e，再 `EvalContext::peek_word(addr)` 读 XLEN 字（无副作用） |
| 寄存器 | `x0`..`x31`（x0 恒 0）、`pc`、ABI 别名 |

```rust
pub enum Expr { /* Num, Reg(RegId), Pc, Deref, Un, Bin, Logic */ }
pub trait EvalContext { fn reg(&self, id: RegId) -> u32; fn pc(&self) -> u32; fn peek_word(&mut self, addr: u32) -> Result<u32, EvalError>; }
pub fn parse(src: &str) -> Result<Expr, ParseError>;   // ParseError { pos, msg }
impl Expr { pub fn eval(&self, ctx: &mut dyn EvalContext) -> Result<u32, EvalError>; }
```
- 已知取舍：寄存器别名表在 `expr` 与 `isa` 各一份（保持双叶 crate 零耦合），由 machine 层一致性测试锁定两表同步
- 验收：优先级/括号/别名/解引用/除零/错误位置单测 ≥ 15 例

### 5.8 rremu-machine

- `MachineBuilder`：`ram(base,size)` `uart(base)` `clint(base)` `framebuffer(base,w,h)` `entry(addr)` `load(Image)` → 校验地址不重叠 → `build() -> Machine`
- `Machine` 持有：Cpu、Bus、设备句柄（uart push_input 等）、EventSink（环形缓冲收集器）、tohost 地址、断点表、监视点表、运行状态
- 控制方法（前端唯一入口）：

```rust
impl Machine {
    pub fn step_n(&mut self, n: u64) -> RunResult;
    pub fn run(&mut self, pause: &AtomicBool) -> RunResult;      // 全速；pause 供 Ctrl+C
    pub fn reset(&mut self);
    pub fn snapshot(&self) -> MachineSnapshot;
    pub fn peek(&mut self, addr: u32) -> Result<u32, BusError>;  // 走 BusDevice::peek_read
    pub fn eval(&mut self, expr: &Expr) -> Result<u32, EvalError>; // EvalContext 实现 = 寄存器 + peek
    pub fn add_breakpoint(&mut self, addr: u32) -> u32;           // 返回编号
    pub fn add_watchpoint(&mut self, expr: Expr) -> u32;
    pub fn remove_watchpoint(&mut self, id: u32) -> bool;
    pub fn watchpoints(&self) -> &[Watchpoint];
    pub fn uart_push_input(&mut self, byte: u8);
    pub fn set_event_ring_capacity(&mut self, n: usize);          // 默认 mem 16 / uart 1024
}
pub enum RunResult { Exited { code: i32 }, HitBreakpoint { pc: u32 }, HitWatchpoint { id: u32, old: u32, new: u32 }, Paused, Trapped { cause: u32 } }
```
- 运行中每条指令后按序检查：tohost 写入（bus 事件拦截）→ 断点（pc 匹配）→ 监视点（重算比对）→ pause 标志
- tohost 退出：写入值 `(code << 1) | 1` → `Exited { code }`；riscv-tests 失败码同样透传
- `MachineSnapshot { cycle, pc, priv_, gprs: [u32;32], csrs: CsrSnapshot, state: RunState, mem_ring: VecDeque<MemAccess>, uart_out: Vec<u8>, uart_in_len, watchpoints: Vec<WatchInfo> }`
- 验收：断点/监视点/peek/快照单测；hello.elf `run()` → `Exited { code: 0 }` 且 UART 事件流包含 "hello"

### 5.9 rremu-cli

- rustyline 18；提示符 `rremu> `；空行重复上一条 `si`（可选增强，非必做）
- 命令解析：首 token 分发；`x n m` 中 n 为十进制计数、m 为表达式；`p/w/b` 取行剩余整体作为表达式；未知命令 → 报错并提示 `help`
- 输出格式：`p` → `= 26 (0x1a)`；`x` → 每行 4 字 `0x80000000:  0x12345678  ...`；`info w` → 编号/表达式/当前值/命中次数
- Ctrl+C：运行态由 ctrlc 处理器置 `AtomicBool` → `run()` 返回 `Paused` 回提示符；提示符态由 rustyline 自身处理（Interrupted，清行重来）
- `-c` 批处理：逐条执行并回显；任一条导致 `Exited` 则忽略剩余并按退出码结束进程；全部执行完且 stdin 非 TTY → 自动 `q`
- 验收：脚本会话集成测试（spawn 子进程喂命令断言输出）

### 5.10 rremu-tui

- 面板：顶栏（cycle/速度档/状态/pc）│ 左栏寄存器（x0-x31/pc/priv/CSR：mstatus mtvec mepc mcause mie mip mcycle minstret）│ 右上反汇编（pc 前 4 后 12，当前行高亮）│ 右下最近存储访问 ring 16 条（#cycle/读写/宽度/地址/数据/设备名）│ 底部串口终端 │ 状态栏按键提示
- 按键：Space 暂停/继续；s 单步；←/→ 档位 1/2/4/8/16/32/64/全速；f 全速；i 串口输入模式（Esc 返回）；r 复位；q 退出
- 线程模型：machine 独立线程按档位 tick 执行；TUI 主线程 ~30fps 拉快照渲染；控制经 mpsc；断点/监视点命中 → 自动暂停并高亮提示（管理命令仍在 CLI，TUI 只展示）
- 反汇编：调用 `isa::Instruction` Display；decode 失败行显示 `.word 0x????????`

### 5.11 bin（rremu）

- `Args`（clap derive）→ `MachineBuilder` 配置 → 选择前端（`--tui` ? tui : cli）
- 日志：`-v` 映射 `info/debug/trace`，默认 `warn`；tracing-subscriber 输出 stderr（不污染 stdout 的 UART 输出）
- 错误：库错误经 `anyhow` 聚合，带上下文（"加载 ELF 失败: …"）

---

## 6. CLI 命令规格

| 命令 | 功能 | 停止/备注 |
|---|---|---|
| `help` | 列出命令 | |
| `c` | 全速运行 | 停止条件：tohost/ecall 退出、断点、监视点、Ctrl+C |
| `si [n]` | 单步 n 条，默认 1 | 逐条回显已执行指令与 pc |
| `b addr` | 断点 | addr 支持表达式；返回编号 |
| `x n m` | 从 m 读 n 个 XLEN 字 | m 支持表达式；peek 读；每行 4 字 |
| `p 表达式` | 求值打印 | `= dec (hex)` 双格式 |
| `w 表达式` | 监视点 | 编号自增；每条指令后重算，变化即打印 `old → new` 并暂停 |
| `info r` | 寄存器 + pc + priv + CSR | |
| `info w` | 监视点列表 | 编号/表达式/当前值/命中次数 |
| `d n` | 删除监视点 n | |
| `q` | 退出 | 退出码 0 |

## 7. TUI 规格

见 §5.10（此处为索引）。

## 8. 外部参数规格

```rust
#[derive(Parser)]
#[command(name = "rremu", version, about = "RISC-V RV32IMC 模拟器与调试器")]
pub struct Args {
    pub elf: String,                                          // 位置参数
    #[arg(short, long)]                        pub tui: bool, // 与 --cmd 冲突
    #[arg(short, long = "cmd", value_name = "CMD")] pub cmds: Vec<String>,
    #[arg(short, long, value_name = "SIZE", default_value = "16M")] pub mem: String,   // K/M/G 后缀
    #[arg(long, value_name = "ADDR", default_value = "0x80000000")] pub base: u32,
    #[arg(long, value_name = "ADDR")] pub entry: Option<u32>,
    #[arg(short, long, action = clap::ArgAction::Count)] pub verbose: u8, // 0..3 档
}
```
- `parse_size()`：`16M → 0x1000000`，非法后缀报错
- 预留不实现：`--no-uart` `--window` `--trace`
- `--no-fb`：禁用 fb 设备映射 + 不自动开窗
- 示例：`rremu hello.elf` / `rremu hello.elf -c c` / `rremu hello.elf -c "b 0x80000100" -c c -c "info r"` / `rremu hello.elf -t -m 64M -v` / `rremu hello.elf --no-fb -c c`

## 9. 外部依赖规格

**原则**：版本收口 `[workspace.dependencies]`（成员 `workspace = true`）；caret + lockfile；库 crate 白名单仅 thiserror/tracing；专属依赖进专属 crate。

| crate | 版本 | 特性 | 位置 |
|---|---|---|---|
| thiserror | 2.0 | 默认 | bus/decode/cpu/loader/machine |
| tracing | 0.1 | 默认 | 全部库 crate |
| tracing-subscriber | 0.3 | env-filter, fmt | bin |
| goblin | 0.10 | default-features=false; elf32, elf64*, endian_fd | loader |
| anyhow | 1.0 | 默认 | bin |
| clap | 4.6 | derive | bin 独占 |
| rustyline | 18 | 默认 | cli |
| ctrlc | 3.5 | 默认 | bin |
| ratatui | 0.30 | 默认 | tui |
| crossterm | 0.29 | 默认 | tui |
| minifb | 0.29 | default-features=false; x11 only | window |

\* goblin 0.10.x 上游缺陷：仅开 elf32 无法编译（0.10.7 实测），故同时启用 elf64；解析路径仍只走 elf32。

**显式排除**：object（goblin 足够）、libc（ctrlc 封装）、env_logger/log（统一 tracing）、第三方反汇编库。测试零外部依赖。

## 10. 测试策略

**总原则：只用 3 个代表性测试验证，不追求跑全套。**

| 层级 | 内容 | 所在 |
|---|---|---|
| 单元 | decode 逐指令、expr ≥15 例、bus 路由、uart 副作用、cpu 语义 | 各 crate `#[cfg(test)]` |
| 代表性集成（3 个，固定） | `rv32ui-p-add`（I 基础）/ `rv32um-p-mul`（M 乘除）/ `rv32uc-p-rvc`（C 压缩） | `tests/` 集成测试 |
| 端到端 | hello world：ELF → UART stdout → `Exited{0}` | `tests/` |

固件产出（本机已确认具备 `riscv64-unknown-elf-gcc`）：

- `tools/build-firmware.sh`：手写 `tests/firmware/hello.S` + 链接脚本 → `riscv64-unknown-elf-gcc -march=rv32imc_zicsr -mabi=ilp32 -nostdlib` → `tests/data/hello.elf`
- `tools/build-tests.sh`：`git clone --depth 1` 官方 riscv-tests → 用其 `env/` 头与链接脚本单独编译上述 3 个 `.S`（`-march=rv32imc/-mabi=ilp32`）→ 缓存 `tests/data/*.elf`（gitignore）
- 兜底：若官方源码构建受阻，3 个代表测试降级为自研等价固件（Rust no_std 或汇编，断言 PASS/FAIL 写 tohost），先跑通后替换

## 11. 风险与对策

| 风险 | 对策 |
|---|---|
| RVC 16/32 混合取指、跨 4 字节边界 | instr_len 先行；cpu 层取两个 halfword 拼接；rv32uc-p-rvc 兜底 |
| M 扩展符号语义（mulh/div 特例） | 黄金值单测逐个锁定；rv32um-p-mul 兜底 |
| peek_read 被误实现为有副作用 | trait 无默认实现 + code review 红线 + uart RDR 对比单测 |
| Ctrl+C 与 rustyline raw 模式冲突 | 运行态才依赖 ctrlc 标志；提示符态交给 rustyline 自身 Interrupted |
| 官方 riscv-tests 构建受阻 | §10 兜底方案：自研等价固件 |
| TUI 刷新拖慢模拟 | 快照节流 + machine 独立线程，TUI 只渲染不阻塞 |
| 表达式别名双表漂移 | machine 层一致性测试锁定 expr 与 isa 的 ABI 表 |

