# rremu 核心架构与状态归属

## 1. 分层

```text
bin / cli / tui / window
        │
        ▼
rremu-machine
  ├── rremu-cpu
  │     ├── rremu-isa
  │     ├── rremu-decode
  │     └── rremu-fp
  ├── rremu-bus
  ├── rremu-devices
  ├── rremu-config
  ├── rremu-loader
  └── rremu-expr / rremu-decode
```

依赖方向必须保持：

```text
cpu   -> bus, isa, decode, fp
bus   -> 无 CPU / Machine 依赖
devices -> bus
machine -> cpu, bus, devices, config, loader, expr, decode, isa
config -> 无运行时依赖
loader -> 无运行时依赖
fp    -> 无运行时依赖（no_std 叶子 crate）
```

禁止：

- `rremu-bus` 依赖 `rremu-cpu` 或 `rremu-machine`
- 设备直接持有 `Cpu`
- 设备直接读取 `Machine` 状态
- 前端直接依赖 `rremu-isa` / `rremu-bus` 内部实现（现有 re-export 仅作为兼容边界）

## 2. 所有权

### 2.1 Cpu

所有者：`rremu_cpu::Cpu`

拥有：

- 32 个通用寄存器
- PC
- 当前特权级
- `CsrState`（含 `fcsr`）
- 32 个 64-bit FPR（FLEN=64，`.s` 使用 NaN boxing）
- `mstatus.FS/SD` 视图
- `cycle`
- `instret`
- `lr/sc` reservation

不得由 `Machine` 或设备直接修改。`Machine` 只能通过 CPU 公共访问器读取，并通过 `step`、`reset`、`set_args` 等生命周期接口操作。

### 2.2 Bus

所有者：`rremu_bus::BusRouter`

拥有：

- 地址区间到设备的映射
- 设备对象所有权
- 总线当前 cycle
- 设备 tick 调度
- 设备中断 line 的聚合入口
- 内存访问事件发出

设备和 CPU 不拥有总线；CPU 通过 `Bus` trait 访问，设备通过 `BusDevice` trait 被访问。

### 2.3 Machine

所有者：`rremu_machine::Machine`

拥有：

- `Cpu`
- `BusRouter`
- 可选 `Image`
- 入口、RAM/设备基址
- `RunState`
- `RingCollector`
- `Terminal`
- UART 直出回调
- trace
- 最近 trap 快照
- `DebugController`
- `FramebufferAccessor`

`Machine` 是前端唯一入口；前端不得直接操作 `Cpu` 或 `BusRouter`。

### 2.4 Config

所有者：`rremu_config::Config`

拥有：

- 默认布局
- 启动参数
- 诊断容量
- host 路径
- 覆盖后的解析结果

当前 DTB 地址通过 `Config::dtb_address()` 纯计算，`Config::with_boot_dtb()` 返回新配置。
`Config` 字段为 crate 内部可见，对外只提供只读 getter 与消费式 builder；
并提供 `ResolvedConfig` 类型别名，表示解析后的配置。

### 2.5 Device

每个 `BusDevice` 拥有自己的寄存器状态和内部队列。

设备不得：

- 保存 CPU 指针
- 访问其它设备
- 直接修改 PLIC pending（中断控制器通过 `irq_assert` 接收）
- 在 `peek_read` 中改变状态

## 3. 运行时数据流

```text
Machine::step_once
  → bus.set_cycle(cpu.cycle)
  → Cpu::step
      → fetch
      → bus.service_interrupts
      → exec
          → bus.inst/data read/write
          → MMU translate
          → PMP check
      → trap / executed
  → EventSink
      → MachineSink
          → UART Terminal / CLI sink
          → RingCollector
  → Machine RunState
```

### 关键约束

- `Cpu::step()` 是 cycle 同步的唯一入口。
- `BusRouter::service_interrupts()` 读取设备 IRQ line、更新 PLIC pending，并返回 cause。
- CSR 读取分为 `debug_csr_read()` 与 `architectural_csr_read()`。
- `Machine` 包含调试、反序列化、帧缓冲与反汇编方法。

## 4. 模块边界

### `rremu-cpu`

按职责拆分：

```text
lib.rs        Cpu 状态与公共入口
csr.rs        CSR 存在性、权限、读写
mmu.rs        Sv32 翻译
pmp.rs        PMP 检查
trap.rs       trap 入口与委派
interrupt.rs  中断投影与仲裁
fp.rs         FPR / NaN boxing / fcsr 访问
exec/         指令执行（含 exec/fp.rs）
fetch.rs      取指
tests/        测试
```

### `rremu-fp`

```text
softfloat.rs  解码、规范化、单次舍入/编码
add.rs / muldiv.rs / sqrt.rs       f32 算术
add64.rs / muldiv64.rs / sqrt64.rs f64 算术
fma.rs        FMA
minmax.rs     fmin / fmax
convert.rs    fcvt.*
format.rs     ±0/NaN/Inf/subnormal 分类与顺序
flags.rs      fflags
round.rs      舍入模式
```

纯整数/位操作实现，不依赖宿主浮点环境；确定性 golden 向量回归由 `rremu-test` harness 承担。

### `rremu-machine`

```text
lib.rs          Machine 生命周期
builder.rs      from_config / 设备映射
runtime.rs      step/run/state
events.rs       RingCollector / MachineSink
snapshot.rs     快照
debugger.rs     断点/监视点/peek/反汇编/eval
framebuffer.rs  帧缓冲访问
```

### `rremu-bus`

```text
access.rs  AccessSize / MemKind / BusError
event.rs   EmuEvent / EventSink
device.rs  BusDevice 及能力 trait
router.rs  Bus / BusRouter / Region
```

### `rremu-config`

```text
defaults.rs  默认常量
layout.rs    设备布局
boot.rs      启动参数
error.rs     配置错误
lib.rs       Config / Overrides / resolve / validate
```

### `rremu-am`

见 [`../../rremu-am/docs/architecture.md`](../../rremu-am/docs/architecture.md)。

## 5. 状态修改规则

| 操作 | 允许者 |
|---|---|
| 修改 GPR/FPR/PC/CSR/cycle | `Cpu` 内部 |
| 修改设备寄存器 | 对应 `BusDevice` |
| 修改地址映射 | `Machine` 构造阶段 |
| 修改断点/监视点 | `DebugController` |
| 修改事件环 | `EventSink` 实现 |
| 修改 DTB 放置 | `Machine` 构造阶段 |
| 修改 trap handler | `rremu-am::cte` 注册接口 |
