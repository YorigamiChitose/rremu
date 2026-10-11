# rremu 第二阶段：问题修正与测试外置规划

> 版本 v2.1 · 定稿日期 2026-10-11
> 来源：对 v1 代码库的独立软件工程审计（未经 v1 PLAN 预设），并对照 v1 PLAN 逐项核对后定稿。
> 性质：本文档是下一阶段优化修改的唯一依据；任何变更先改本文档再改代码（沿用 v1 治理规则）。
> v1 规划文档已整体作废删除，其中被证伪的设计结论收录在 §2，防止后续开发沿用错误假设。
> 修订（v2.1，人工审核）：tests/data 已删除，golden 基线导出任务作废；否决 in-crate 内联硬编码测试方案，耦合测试直接删除、职责移交新测试项目。

---

## 1. 项目现状（已验证事实）

RV32IMC_Zicsr 单周期 RISC-V 模拟器，Cargo workspace，11 个内部 crate，约 7900 行。

### 1.1 架构分层（维持不变）

```
bin（唯一 composition root）
  ├── cli（REPL）/ tui（ratatui）/ window（minifb）
machine（Machine/Builder/快照/断点/监视点）
cpu · devices（ram/uart/clint/fb）· loader（ELF）
bus（BusDevice/Bus trait · EmuEvent/EventSink）
isa · decode · expr（三叶 crate）
```

### 1.2 已验证的合格项（重构红线：不得破坏）

| 项 | 证据 |
|---|---|
| 依赖红线全部成立：isa/bus/expr/loader 零外部依赖；cpu 不 import 具体设备；bin 唯一组装点 | 各 Cargo.toml |
| 全库 0 unsafe、库 crate 0 unwrap/expect | grep 验证 |
| `Bus` trait 依赖倒置：cpu 只依赖 trait | rremu-bus |
| `peek_read` 无默认实现，强制设备声明无副作用读语义 | rremu-bus |
| 单一事件流（设备经 sink 参数上报，无共享所有权） | rremu-bus |
| lint 基线：unsafe_code=forbid、clippy unwrap_used=deny | workspace Cargo.toml |
| rust-version 1.99 / edition 2024 / --no-fb 懒启动 / minifb x11-only | 已落地 |

### 1.3 v1 PLAN 证伪记录（后续开发禁止沿用以下旧结论）

1. **中断优先级**：v1 规定 `pending_interrupt()` "cause 降序聚合（MEI>MTI>MSI）"。该规格三重错误：
   - 与 RISC-V spec 矛盾（正确序为 MEI > MSI > MTI，即外部 > 软件 > 时钟）；
   - v1 文字与自身"cause 降序"算法矛盾（MSI=1 < MEI=3 < MTI=7，排不出 MEI>MTI>MSI）；
   - 算法本身导致 §3.1 的使能位饿死 bug。
   **新结论：优先级裁剪不得发生在设备侧；CPU 拿到全部 pending cause 后按 spec 序遍历、跳过未使能者。**
2. **退出约定**：v1 声称 "tohost + ecall"。实际仅 tohost 有实现（machine 仅响应 Halt 事件）；ecall 非致命陷入后继续执行。无 tohost 且依赖 ecall 退出的固件会永久运行。**新结论：退出条件 = tohost 写入，ecall 仅在固件 handler 内写 tohost 间接退出；文档按此表述。**
3. **执行模型**：v1 §11 写"machine 独立线程"，§5.10 写"单线程协作式"，自相矛盾。**实际采用单线程协作式（TUI 事件轮询 + 档位批量 tick + 30fps 帧限），本阶段维持不变。**
4. **U→M 陷阱的 mstatus.MPP 写入规则**：v1 未定义，实现"一律 priv ← Machine"对 U-mode ecall 恰好正确属巧合。**新结论：MPP 写入规则须显式写入本文档 §3（trap 时 MPP ← 陷前 priv）。**

---

## 2. 测试外置决策（tests/ 与 tools/ 完全移除）

决策：本仓库移除 `bin/tests/` 与 `tools/`，rremu 的测试改由独立新项目承担。

状态（2026-10-11）：`tests/data/*.elf` 已人工删除，下方耦合测试当前为红，按 §2.2 处置。

### 2.1 隐藏耦合（已触发：tests/data 已删除，下列测试当前红）

`tests/data/*.elf` 被 gitignore、由 tools 脚本生成，但被库 crate 的 **in-crate 单测**引用，且无跳过机制：

| 位置 | 引用 | 影响测试数 |
|---|---|---|
| machine/src/lib.rs:739 `HELLO_ELF`（panic 提示"先运行 tools/build-firmware.sh"） | ../../tests/data/hello.elf | ~12 个（tohost 退出/断点/监视点/快照/reset/CLINT timer） |
| machine/src/lib.rs:1076 `TIMER_ELF` | ../../tests/data/timer.elf | timer 中断链路 |
| loader/src/lib.rs:136 | ../../tests/data/hello.elf | 真实 ELF 解析 |

### 2.2 执行顺序（严格按序，禁止跳步）

1. **P0 · 删除依赖 tests/data 的 in-crate 测试（不修复、不重写）**：machine 的 HELLO_ELF/TIMER_ELF 固件测试族与 loader 的 hello ELF 测试直接删除，in-crate 测试恢复全绿。
   > 决策记录：曾提出的"in-crate 内联硬编码最小固件"方案经评审否决——与被测生产代码同仓库同源，无法提供独立校验视角，且手写指令字节维护成本高、表达力差；固件级验证职责整体移交新测试项目（§2.3）。
2. **P0 · 保活三个纯内存单测**（与新测试项目互补、零依赖）：
   - machine/src/lib.rs:913 `expr_alias_table_matches_isa`（ABI 别名双表漂移的唯一防线）；
   - UART RDR 读副作用 / peek 无副作用对比；
   - peek_read 各设备覆盖。
3. **P1 · 修订本文档**（后续变更继续走此流程）。
4. **P1 · 删除 `bin/tests/` 与 `tools/`**；同步删除 workspace 中仅服务测试的引用。

### 2.3 移除后新增义务

- **契约化**：machine 公开 API（`Machine`/`MachineSnapshot`/`RunResult`）+ `EmuEvent` 流 + loader 公共类型（`Image`/`LoaderError`）自本阶段起是新测试项目的跨仓库契约。对这些公共类型的破坏性变更视为接口变更，须先改本文档并知会测试项目。
- **tohost 机制进入无验证状态**：loader 的符号提取、bus 的 watch、machine 的 Exited 编码（`(code<<1)|1`）在生产代码中保留，验证责任完全移交新测试项目（列为首批用例）；in-crate 不引入任何临时替代验证（含内联硬编码固件，见 §2.2-1 决策记录）。
- `encode_c`（decode）被 decode/cpu 的 in-crate 测试使用，**保留**，加 `#[doc(hidden)]` 标注测试专用。

---

## 3. 问题清单（本阶段工作项）

优先级：P0 必修 / P1 应修 / P2 择机。

### 3.1 正确性

| # | 级 | 问题 | 位置 | 修法 |
|---|---|---|---|---|
| C1 | P0 | 中断优先级聚合在设备侧（max-cause）先裁剪、后查 mie，高优先级未使能时饿死已使能低优先级（如 MTI 未使能而 MSI 使能 → MSI 永不服务） | bus `pending_interrupt()` 323-328；cpu 173-180 | `check_interrupt()` 返回全部 pending cause（`Vec<u32>` 或迭代器）；CPU 按 spec 序 MEI>MSI>MTI 遍历、跳过未使能者。旧 timer 固件单测只开 MTIE，从未暴露；新测试项目须补"MSI+MTI 并存 + 差异使能"用例 |
| C2 | P1 | vectored mtvec 未实现 → `trap()` 返回 Unimplemented，machine 仅 log 后继续；mepc/mcause 已写而 pc 未动，下一轮重陷同一指令，静默死循环 | cpu 572-574；machine 415-419 | machine 收到 Unimplemented 进入 `RunState::Faulted(String)`（新增终态），run/step_n 立即返回，禁止静默 spin |
| C3 | P1 | ecall 无退出路径：无 tohost 固件永久运行 | machine run/step_n | 本阶段不做 ecall 退出（见 §1.3-2）；在 CLI/tui 文档与 help 中明示"退出依赖 tohost" |
| C4 | P2 | mstatus.MPP 写入规则未成文，现实现依赖巧合 | cpu trap | 显式实现：trap 时 MPP ← 陷前 priv；补 U-mode ecall 单测锁定 |

### 3.2 架构

| # | 级 | 问题 | 位置 | 修法 |
|---|---|---|---|---|
| A1 | P1 | `as_any_mut` + downcast 使 BusDevice 抽象失效；machine 耦合 Ram/Uart/Fb/Clint 四具体类型；reset downcast 失败静默跳过 RAM 清零 | bus 117；machine 4 处 downcast | 拆分类 trait：`Resetable`/`InputSink`/`FrameSource`，或 trait 提供 `as_ram_mut() -> Option<&mut Ram>` 等类型化下取；reset 失败必须报错 |
| A2 | P1 | Machine 上帝对象（17 字段：调度/断点/监视点/快照/反汇编/注入/镜像），且自本阶段兼任测试契约 | machine 328-348 | 拆 `DebugController`/`Disassembler`/`FramebufferAccessor`；**对外契约方法签名保持稳定（§2.3）** |
| A3 | P2 | ABI 别名表 expr:194 / isa:283 双份，靠 machine 测试锁定 | 两处 | 抽 zero-dep 常量 crate `rremu-abi`，双 crate 共享，删除一致性测试对漂移的依赖（测试仍保留） |
| A4 | P2 | Instruction 扁平枚举 80+ 变体，扩展 F/D/A 需四处同步改 | isa 29-123 | 本阶段不动；F/D/A 立项时再按扩展分组重构 |

### 3.3 性能

| # | 级 | 问题 | 位置 | 修法 |
|---|---|---|---|---|
| F1 | P1 | `EmuEvent::MemAccess.device: String` 每访存堆分配 | bus 83 | 改 `&'static str`（设备名均为静态字面量） |
| F2 | P2 | EventSink 泛型与 dyn 混用，热路径 load/store 走 dyn，泛型无单态化收益 | cpu | 统一 `&mut dyn EventSink` |
| F3 | P2 | 监视点求值每步 `expr.clone()` | machine 429 | `eval` 接口改收 `&Expr`，去 clone |

### 3.4 健壮性

| # | 级 | 问题 | 位置 | 修法 |
|---|---|---|---|---|
| R1 | P2 | 设备 read/write/write_bytes 无内部边界检查，信任总线路由；三设备越界行为不一致（Device 错误 / panic / 部分检查） | ram 27-42、uart、clint、fb | 设备内部校验返回 `BusError`，统一语义 |
| R2 | P2 | `Cpu::reg(i)` 无边界检查（u8 可达 255） | cpu 100-102 | `assert!(i < 32)` 或改 `debug_assert` + 文档标注不变量 |
| R3 | P2 | `usize as u32` 截断（builder 校验段地址） | machine 278 | `u32::try_from` |

### 3.5 工程基建

| # | 级 | 问题 | 修法 |
|---|---|---|---|
| B1 | P1 | 本文档 §2 执行顺序落地（见 §2.2） | 按序执行 |
| B2 | P2 | rremu-window 单测仅覆盖 Error Display，核心 update/poll 零覆盖 | 无 X11 环境限制，列为低优先；可做 headless 逻辑抽测 |
| B3 | P2 | BusRouter 线性路由、Machine 持有 `Image` 全量 Vec 供 reset | 设备增多（PLIC/多 UART）时改区间索引；reset 拷贝用 `Arc<Image>`，暂缓 |

---

## 4. 里程碑

| 顺序 | 交付 | 含工作项 |
|---|---|---|
| M1 | 删除依赖 tests/data 的红测试（不重写）+ 保留单测点名保活，in-crate 全绿 | §2.2-1/2 |
| M2 | 删除 bin/tests 与 tools；PLAN v2 生效 | §2.2-3/4、B1 |
| M3 | 中断重构（check_interrupt 多值返回 + CPU 侧 spec 序裁决） | C1 |
| M4 | Faulted 终态 + MPP 规则成文 | C2、C4 |
| M5 | 抽象收敛（类 trait 拆分、Machine 拆分、契约章节维护） | A1、A2、§2.3 |
| M6 | 性能与健壮性清理 | F1-F3、R1-R3 |
| （外部） | 新测试项目：自建官方 riscv-tests 集成；首批用例 = tohost 链路、UART 输出、CLINT timer 链路、MSI+MTI 差异使能、U-mode ecall | C1、C3、C4、§2.3 |

## 5. 验收口径

- `cargo test` 全绿且不依赖任何外部工具链、外部仓库、gitignore 缓存；
- `cargo clippy` 无 error（维持 unwrap_used=deny 基线）；
- C1 修复后：MSI 使能 + MTI 未使能 + 两者同时 pending → 必须进入 MSI handler；
- C2 修复后：vectored mtvec 场景 machine 进入 Faulted 终态，进程可感知退出；
- §1.2 红线项逐条复查不回退（依赖方向、0 unsafe、peek_read 无默认实现、单一事件流）。
