# rremu 第一层重构执行计划

> 本文件只描述执行顺序，不代表代码已经拆分。
>
> 全项目函数级矩阵见 [`../../CORE-REVIEW-AND-REFACTOR-PLAN.md`](../../CORE-REVIEW-AND-REFACTOR-PLAN.md)。

## 当前进度

```text
阶段 1：文档与基线        完成
阶段 2：Bus/Config/ISA 拆分 完成
阶段 3：CPU 主体拆分       进行中
  - lib.rs、mmu、pmp、csr、trap、interrupt、fetch、exec 已拆出
  - tests 与 exec 内部子模块仍待继续拆分
```

本阶段提交：

```text
a647ba6 docs(core): define architecture, interfaces, and invariants
7f580ec refactor(bus): split access, event, device and router modules
167d9db refactor(config): split defaults, layout, boot, and error modules
1fc1c11 refactor(isa): split types, instructions, CSR, and cause modules
5c8182b refactor(cpu): move unit tests into tests module
7900f5b refactor(cpu): move Sv32 translation into mmu module
ee4b666 refactor(cpu): move PMP checks into pmp module
53c6953 refactor(cpu): move CSR state and access logic into csr module
ec083fc refactor(cpu): move trap and interrupt logic into modules
15a7d13 refactor(cpu): move instruction fetch into fetch module
d978ba0 refactor(cpu): move execution and memory operations into exec module
```

## 1. 原则

```text
先文档
后结构
再接口
最后优化
```

每个提交只做一件事：

- 只移动代码，不改行为
- 只改接口，不同时拆文件
- 只做优化，不同时改契约

## 2. 阶段

### 阶段 1：文档与基线

- 建立 `docs/` 文档
- 记录基线提交
- 记录公共 API 和状态归属
- 不修改代码

### 阶段 2：低风险拆分

顺序：

1. `rremu-bus`
2. `rremu-config`
3. `rremu-isa`

目标：

```text
bus:
  access.rs
  event.rs
  device.rs
  router.rs

config:
  defaults.rs
  layout.rs
  boot.rs
  error.rs

isa:
  csr.rs
  cause.rs
  instruction.rs
```

要求：

- 公共 re-export 保持
- 现有测试全部保留
- 只移动代码

### 阶段 3：CPU 拆分

顺序：

1. 测试迁出 `lib.rs`
2. `mmu.rs`
3. `pmp.rs`
4. `csr.rs`
5. `trap.rs`
6. `interrupt.rs`
7. `fetch.rs`
8. `exec/`

目标结构：

```text
src/
├── lib.rs
├── fetch.rs
├── csr.rs
├── mmu.rs
├── pmp.rs
├── trap.rs
├── interrupt.rs
├── exec/
│   ├── mod.rs
│   ├── system.rs
│   ├── memory.rs
│   └── alu.rs
└── tests/
```

要求：

- `Cpu::step` 公共签名不变
- 每步单独提交
- 先移动测试，再移动实现
- 不在此阶段改 CSR 接口

### 阶段 4：Machine 拆分

顺序：

1. `DebugController` → `debugger.rs`
2. `RingCollector/MachineSink` → `events.rs`
3. `from_config` → `builder.rs`
4. `step_once/run` → `runtime.rs`
5. `snapshot` → `snapshot.rs`
6. `FramebufferAccessor` → `framebuffer.rs`

要求：

- CLI/TUI 调用方式不变
- 事件顺序不变
- 快照字段兼容

### 阶段 5：接口显式化

顺序：

1. CSR 架构读 / 调试读拆分
2. `pending_interrupts` 改名或拆分为 poll/service
3. `set_cycle` 唯一责任方
4. Config resolve 后不可变
5. DTB 放置迁出 Config

每项必须：

- 更新 `interfaces.md`
- 增加/调整单元测试
- 保持行为等价，除非明确记录行为变化

### 阶段 6：HAL 收敛

在 `rremu-am` 中：

- 新建 `state.rs`
- 集中 `HANDLER/S_HANDLER/DEFAULT_TRAP_INFO/BOOT_A0/A1`
- 拆 `cte.rs`
- 明确单 hart 假设

### 阶段 7：验证

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

第一层重构完成前不恢复第三层测试扩展；恢复后运行完整 `rremu-test` 54 用例。

## 3. 禁止事项

- 不在拆分提交中改变公共行为
- 不为了减少行数而拆出无语义模块
- 不新增 `static mut`
- 不删除现有测试
- 不把第二层 `platforms/` 或第三层测试混入本轮

## 4. 完成标准

```text
公共 API/ABI 稳定
核心接口有文档
副作用显式
状态有唯一 owner
没有未说明的全局状态
第一层测试通过
```
