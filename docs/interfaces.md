# rremu 核心接口契约

本文定义 rremu 核心模块的接口边界。

## 1. Cpu

### 1.1 生命周期

```rust
Cpu::new(entry: u32) -> Cpu
Cpu::reset(&mut self, entry: u32)
Cpu::set_args(&mut self, a0: u32, a1: u32)
```

契约：

- `new/reset` 后特权级为 M，CSR 为复位值。
- `set_args` 只写 `a0/a1`，不改变 PC/CSR。
- `reset` 清空 reservation。

### 1.2 单步

```rust
Cpu::step<B: Bus, E: EventSink>(
    &mut self,
    bus: &mut B,
    sink: &mut E,
) -> Result<StepOutcome, CpuError>
```

契约：

- 执行至多一条指令。
- 正常执行、trap、宿主退出都推进 `cycle`；`instret` 只在成功执行指令时推进。
- 取指异常优先于中断。
- 中断只在取指成功后检查。
- 所有取指/数据访问必须经过 `Bus`。
- `StepOutcome::Executed` 携带原始编码，供 trace/dump。
- `StepOutcome::Exited` 只用于 `EXIT_MAGIC` 宿主退出协议。
- `CpuError::Unimplemented` 表示不可恢复的模型缺陷，不表示 guest trap。

### 1.3 CSR

当前：

```rust
Cpu::debug_csr_read(&self, addr: u16) -> Option<u32>
Cpu::architectural_csr_read(&self, addr: u16) -> Result<u32, TrapErr>
```

语义：

- 调试读：未知 CSR 返回 `None`。
- 架构读：未实现 CSR 返回 illegal。
- `sstatus/sie/sip` 是 `mstatus/mie/mip` 的掩码视图。
- 快照和调试使用调试读；CSR 指令路径使用架构读。

规则：

- 架构读：未实现 CSR MUST 返回 illegal。
- 调试读：未实现 CSR 返回 `None`，不得影响架构语义。
- CSR 指令路径 MUST 使用架构读/写。
- 外部调用方 MUST NOT 通过调试读绕过 CSR 存在性检查。

CSR 权限：

- 按 CSR 地址 bit[9:8] 检查特权级。
- 只读 CSR（bit[11:10]=0b11）写入 MUST illegal。
- 未实现 CSR 读写 MUST illegal。
- `csr_write` 只允许在权限/存在性检查通过后调用。

### 1.4 MMU

```rust
Cpu::translate<B: Bus>(&self, bus, va, access) -> Result<u32, TrapErr>
```

契约：

- 所有取指和数据访问必须经过此入口。
- Sv32 时逐级读取 PTE；PTE 非法、权限失败、A/D 更新失败按 RISC-V 语义产生 page fault。
- M-mode 在 `MPRV` 下的数据访问按 `MPP` 特权级翻译。
- 本接口不产生事件；内存访问事件由 `Bus` 负责。

### 1.5 PMP

```rust
Cpu::pmp_check(&self, addr: u32, access: AccessType) -> bool
```

契约：

- 低编号 PMP entry 优先。
- M-mode 对未锁定项默认放行；锁定项必须满足权限。
- S/U 无匹配项时拒绝。
- 结果必须在物理访问前决定。

### 1.6 Trap

```rust
Cpu::trap(&mut self, sink, cause, mtval) -> Result<StepOutcome, CpuError>
Cpu::trap_supervisor(&mut self, cause, mtval) -> Result<StepOutcome, CpuError>
```

契约：

- `trap` 负责 M/S 委派判断。
- M trap 写 `mepc/mcause/mtval`，进入 M-mode，按 `mtvec` 跳转。
- S trap 写 `sepc/scause/stval`，更新 `sstatus.SPP/SPIE/SIE`，按 `stvec` 跳转。
- 当前只支持 direct mode；vectored mode 返回 `CpuError::Unimplemented`。

### 1.7 中断

```rust
Cpu::sync_mip(&mut self, pending: &[u32])
Cpu::pending_interrupt(&self) -> Option<u32>
```

契约：

- 设备 pending cause 投影到 `mip` 的设备位。
- `STIP/SSIP` 等软件位不被设备覆盖。
- 优先级：MEI > MSI > MTI > SEI > SSI > STI。
- 是否可接收由当前特权级、`mideleg`、`mstatus.MIE/SIE` 共同决定。

### 1.8 浮点状态

```rust
Cpu::fprs(&self) -> &[u64; 32]
Cpu::fpr(&self, i: u8) -> u64
Cpu::fpr_s(&self, i: u8) -> Fp32
Cpu::fpr_d(&self, i: u8) -> Fp64
Cpu::fcsr(&self) -> u32
Cpu::restore_fp_state(&mut self, fprs: &[u64; 32], fcsr: u32, fs: u32)
```

契约：

- FLEN=64；`.s` 读取要求高 32 位为全 1 的 NaN box，否则返回 canonical NaN。
- `.s` 写入统一置 NaN box；FPR 写入后 `mstatus.FS=Dirty`。
- `fcsr` 包含 `fflags/frm`；异常标志 sticky，只有值变化时 `FS=Dirty`。
- `mstatus.FS=Off` 时 F/D 指令与 FP CSR 访问 illegal。
- `mstatus.SD` 只在读取时由 `FS=Dirty` 合成，写入忽略。
- `restore_fp_state` 是快照恢复 FP 状态的唯一入口。

## 2. Bus

### 2.1 访问原语

```rust
Bus::inst_read(addr, size, sink) -> Result<u32, BusError>
Bus::data_read(addr, size, sink) -> Result<u32, BusError>
Bus::data_write(addr, size, value, sink) -> Result<(), BusError>
```

契约：

- 一次调用对应一次对齐物理访问。
- 对齐违例返回 `BusError::Misaligned`。
- 未映射或跨越设备区间返回 `BusError::Unmapped`。
- 访问成功时发出且只发出一个对应的 `MemAccess` 事件。
- 指令取指使用 `MemKind::Inst`，数据读写分别使用 `DataRead/DataWrite`。
- 交叉页/跨 XLEN 的拆分由 CPU 负责，Bus 不感知。

### 2.2 调试/批量接口

```rust
Bus::peek(addr, size) -> Result<u32, BusError>
Bus::write_bytes(addr, data) -> Result<(), BusError>
```

契约：

- `peek` MUST NOT 改变设备/CPU 状态。
- `peek` MUST NOT 发事件。
- `write_bytes` 用于镜像加载，MUST NOT 发内存访问事件。
- `write_bytes` 不得跨越设备区间。

### 2.3 周期与设备

```rust
Bus::set_cycle(cycle: u64)
BusDevice::tick(cycle)
```

契约：

- `set_cycle` 把当前 cycle 下发给所有设备。
- `tick` 只更新设备内部时序状态（如 CLINT mtime），不得访问 CPU。
- 当前由 `Cpu::step` 在每步开始前调用 `set_cycle`；`Machine` 不再重复调用。

### 2.4 中断

当前：

```rust
Bus::service_interrupts(&mut self) -> Vec<u32>
```

重要：这不是纯查询。

实际行为：

1. 读取所有设备的 `irq_line()`。
2. 对每个 asserted source 调用所有设备的 `irq_assert()`。
3. PLIC 接收并置 pending。
4. 收集所有设备的 `check_interrupt()` cause。

契约：

- `irq_line` 是 level 语义。
- `irq_assert` 只用于把 source 传给中断控制器。
- `check_interrupt` 返回当前全部 pending cause，不裁剪优先级。
- 优先级由 CPU 裁决。
- 当前名称为 `service_interrupts()`，明确表示有副作用。

## 3. BusDevice

```rust
BusDevice::read/write/peek_read/write_bytes
BusDevice::check_interrupt/irq_line/irq_assert/tick
BusDevice::as_resetable/as_input_sink/as_frame_source
```

契约：

- 地址是绝对地址；设备内部自行计算 offset。
- `read/write` 可有副作用。
- `peek_read` MUST 无副作用，且每个设备必须显式实现。
- `write_bytes` 默认不支持时返回 `Device` 错误。
- 能力下取只返回设备真实支持的能力。
- 设备不得直接调用其它设备或 CPU。

### 3.1 UART

- `THR` 写发出 `UartOutput` 事件。
- RX 数据 + IER.RX 使能 → `irq_line = UART_IRQ`。
- THRE pending + IER.TX 使能 → `irq_line = UART_IRQ`。
- 读 IIR 清 THRE pending；写 THR 重新置 THRE pending。
- RX 中断优先于 THRE。
- `peek_read` 不得弹 RX FIFO。

### 3.2 PLIC

- `check_interrupt` 返回 M/S context 可接收的 MEI/SEI。
- claim 后清对应 pending。
- 设备 IRQ 通过 `irq_assert` 进入 pending。

### 3.3 RTC / syscon

- RTC 只实现时间/alarm 寄存器子集，不提供 alarm IRQ。
- syscon `0x5555` → `EmuEvent::PowerOff`。
- syscon `0x7777` → `EmuEvent::Reboot`。
- `PowerOff` 映射宿主 exit 0；`Reboot` 映射 exit 2。

## 4. EventSink / Machine

### 4.1 事件顺序

当前 `MachineSink`：

```text
UartOutput
  → Terminal::feed
  → CLI uart_sink
  → RingCollector
```

其它事件直接进入 `RingCollector`。

契约：

- UART 输出事件 MUST 先更新终端状态，再进入事件环。
- `RingCollector` 的 halt 状态由 `Halt/PowerOff/Reboot` 设置。
- `PowerOff` / `Reboot` 不再通过 `Halt { code }` 混淆。

### 4.2 Machine 生命周期

```rust
Machine::from_config(cfg, image) -> Result<Machine, MachineError>
Machine::step_n(n) -> RunResult
Machine::run(pause) -> RunResult
Machine::reset() -> Result<(), MachineError>
```

契约：

- `from_config` 是设备映射和镜像加载的唯一入口。
- 段地址必须位于 RAM 范围内。
- 入口优先级：`cfg.entry` > ELF entry；都没有返回 `NoEntry`。
- `step_n/run` 只通过 `step_once` 执行。
- 断点在取指前检查；监视点在执行后检查。
- trap 不是终止条件，除非产生不可恢复错误。

### 4.3 快照

```rust
Machine::snapshot() -> MachineSnapshot
Machine::snapshot_compact() -> MachineSnapshot
```

契约：

- `snapshot` 包含事件环。
- `snapshot_compact` 不克隆事件环，供隐藏面板的前端使用。
- 快照是只读视图；不得用于修改 Machine。

### 4.4 调试接口

```rust
Machine::peek / disasm_at / disasm_range / eval
```

契约：

- `peek` 无副作用。
- `disasm` 通过无副作用 peek 读取。
- `eval` 只使用调试读路径，不影响 guest 状态。

## 5. Config

```rust
Config::default()
Config::validate(&self) -> Result<(), ConfigError>
Config::resolve(Overrides) -> Result<Config, ConfigError>
Config::layout(&self) -> &Layout
Config::diag(&self) -> &Diag
Config::paths(&self) -> &Paths
Config::boot(&self) -> &Boot
Config::entry(&self) -> Option<u32>
Config::with_layout(self, Layout) -> Config
Config::with_fb(self, Option<FbLayout>) -> Config
Config::with_boot_hartid(self, u32) -> Config
Config::with_boot_dtb_addr(self, Option<u32>) -> Config
Config::dtb_address(&self) -> Result<u32, ConfigError>
Config::with_boot_dtb(self) -> Result<Config, ConfigError>
```

当前契约：

- `resolve` 合并默认值与覆盖项并校验。
- `Config` 字段对外不可见；读取走 getter，修改走消费式 builder。
- `ResolvedConfig` 是解析后配置的类型别名。
- `dtb_address` 只计算地址，不修改配置。
- `with_boot_dtb` 返回带 DTB 地址的新配置。
- 不再提供公开的 `&mut self` 配置修改接口。

## 6. 错误语义

| 错误 | 来源 | 语义 |
|---|---|---|
| `BusError::Unmapped` | Bus | 地址未映射或越界 |
| `BusError::Misaligned` | Bus | 对齐违例 |
| `BusError::Device` | 设备 | 设备级失败 |
| `CpuError::Unimplemented` | CPU | 模型不可恢复缺陷 |
| `MachineError::NoEntry` | Machine | 无入口 |
| `MachineError::SegmentOutOfRange` | Machine | 镜像段非法 |
| `ConfigError` | Config | 配置范围/重叠/尺寸非法 |

## 7. 公共 API 约束

当前实现必须保持：

- `Cpu::step` 公共签名
- `Bus` trait 的访问语义
- `Machine` 的公共运行/快照 API
- `EmuEvent` 的现有变体语义
- 前端可编译
