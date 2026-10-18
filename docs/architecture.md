# rremu 核心架构与状态归属

## 1. 分层

```text
bin / cli / tui / window
        │
        ▼
rremu-machine
  ├── rremu-cpu
  │     └── rremu-isa
  ├── rremu-bus
  ├── rremu-devices
  ├── rremu-config
  ├── rremu-loader
  └── rremu-expr / rremu-decode
```

依赖方向必须保持：

```text
cpu   -> bus, isa, decode
bus   -> 无 CPU / Machine 依赖
devices -> bus
machine -> cpu, bus, devices, config, loader, expr, decode, isa
config -> 无运行时依赖
loader -> 无运行时依赖
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
- `CsrState`
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

当前 `Config::place_dtb()` 仍会修改配置，见“当前偏差”。

Target：解析后的配置不可变；DTB 放置属于 `Machine` 构造阶段。

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
      → bus.pending_interrupts
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

### 当前偏差

- `Machine::step_once()` 和 `Cpu::step()` 都调用 `bus.set_cycle()`。
- `BusRouter::pending_interrupts()` 不是只读函数：它会读取设备 IRQ line 并调用 `irq_assert` 更新 PLIC pending。
- `Cpu::csr_read()` 同时服务调试/快照和 CSR 指令路径，未知 CSR 返回 0；CSR 指令路径另有 illegal 检查。
- `Machine` 仍直接包含调试、反序列化、帧缓冲与反汇编方法。

这些偏差在 [`refactor-plan.md`](refactor-plan.md) 中有对应阶段。

## 4. 模块边界目标

### `rremu-cpu`

按职责拆分：

```text
lib.rs        Cpu 状态与公共入口
csr.rs        CSR 存在性、权限、读写
mmu.rs        Sv32 翻译
pmp.rs        PMP 检查
trap.rs       trap 入口与委派
interrupt.rs  中断投影与仲裁
exec/         指令执行
fetch.rs      取指
tests/        测试
```

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
| 修改 GPR/PC/CSR/cycle | `Cpu` 内部 |
| 修改设备寄存器 | 对应 `BusDevice` |
| 修改地址映射 | `Machine` 构造阶段 |
| 修改断点/监视点 | `DebugController` |
| 修改事件环 | `EventSink` 实现 |
| 修改 DTB 放置 | `Machine` 构造阶段（Target） |
| 修改 trap handler | `rremu-am::cte` 注册接口 |

## 6. 当前已知结构风险

| 风险 | 位置 | 处理阶段 |
|---|---|---|
| CPU 单文件过大 | `rremu-cpu/src/lib.rs` | 3 |
| Machine 混合核心/调试/前端 | `rremu-machine/src/lib.rs` | 4 |
| Bus 中断聚合有隐藏副作用 | `rremu-bus/src/lib.rs` | 5 |
| Config 解析后仍可修改 | `rremu-config/src/lib.rs` | 5 |
| HAL 裸静态状态分散 | `rremu-am` | 6 |
| 重复 cycle 同步 | Machine/CPU | 5 |
