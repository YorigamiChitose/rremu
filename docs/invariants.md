# rremu 架构不变量

本文件列出第一层重构期间不得改变的行为。任何重构提交如果违反其中一条，必须停止并重新评估。

## 1. CPU 执行

| 编号 | 不变量 |
|---|---|
| CPU-1 | 正常执行一条指令推进一个 cycle |
| CPU-2 | trap 也推进 cycle |
| CPU-3 | `instret` 只统计成功执行，不统计 trap 本身 |
| CPU-4 | 一条指令的执行结果只由当前架构状态和 bus 响应决定 |
| CPU-5 | 不可恢复模型缺陷使用 `CpuError::Unimplemented`，不得伪装成 guest trap |

## 2. 取指与异常优先级

| 编号 | 不变量 |
|---|---|
| IF-1 | 取指对齐/访问异常优先于中断 |
| IF-2 | 中断只在取指成功之后检查 |
| IF-3 | 4 字节地址的 Word 读不得跨 4KiB 页 |
| IF-4 | `pc & 3 == 2` 时先读低 half，确认是 32 位指令后才读 `pc+2` |
| IF-5 | 页尾 16 位指令不得误触发下一页 page fault |

## 3. 中断

| 编号 | 不变量 |
|---|---|
| IRQ-1 | 优先级固定为 MEI > MSI > MTI > SEI > SSI > STI |
| IRQ-2 | 目标为 M 的中断：当前低于 M 时全局使能；当前为 M 时需 `mstatus.MIE` |
| IRQ-3 | 目标为 S 的已委派中断：当前 U 时接收，当前 S 时需 `SIE` |
| IRQ-4 | 当前为 M 时不接收已委派给 S 的中断 |
| IRQ-5 | `sync_mip` 不得覆盖软件写入的 `STIP/SSIP` |
| IRQ-6 | PLIC claim 后清除对应 pending |

## 4. CSR

| 编号 | 不变量 |
|---|---|
| CSR-1 | 未实现 CSR 的架构读写必须 illegal |
| CSR-2 | 只读 CSR 写入必须 illegal |
| CSR-3 | 特权级不足必须 illegal |
| CSR-4 | `sstatus/sie/sip` 是 `mstatus/mie/mip` 的 S 位掩码视图 |
| CSR-5 | 调试读不得改变架构 CSR 状态，也不得绕过 CSR 存在性检查 |
| CSR-6 | OpenSBI 通过 CSR 读成功与否探测扩展；不得恢复“未知读返回 0”的架构行为 |
| CSR-7 | RV32 `mstatush/sstatush` 当前恒为 0，但必须存在 |

## 5. MMU / Sv32

| 编号 | 不变量 |
|---|---|
| MMU-1 | 所有取指/访存都经过唯一 `translate` 收口 |
| MMU-2 | `satp.MODE=0` 时 bare 直通 |
| MMU-3 | `satp.MODE=1` 时走两级 Sv32 页表 |
| MMU-4 | 非法 PTE、权限不足、A/D 失败产生对应 page fault |
| MMU-5 | M-mode 的 `MPRV` 数据访问按 `MPP` 特权级翻译；取指仍按当前特权级 |
| MMU-6 | A/D 位硬件置位必须写回 PTE |
| MMU-7 | translate 不发内存访问事件；事件由 Bus 发 |

## 6. PMP

| 编号 | 不变量 |
|---|---|
| PMP-1 | 第一个匹配的 PMP entry 决定权限 |
| PMP-2 | 低编号优先 |
| PMP-3 | M-mode 未锁定项默认绕过；锁定项必须满足权限 |
| PMP-4 | S/U 无匹配项时拒绝 |
| PMP-5 | PMP 检查发生在物理访问前 |
| PMP-6 | 当前 PMP granularity 为 G=0（4 字节）；`pmpaddr` 全 32 位可写，OpenSBI 报告的 `log2gran=2` 对应此模型 |

## 7. Bus

| 编号 | 不变量 |
|---|---|
| BUS-1 | 公共访问原语一次对应一次对齐物理访问 |
| BUS-2 | 访问成功只发一个对应 `MemAccess` 事件 |
| BUS-3 | 对齐违例返回 `Misaligned` |
| BUS-4 | 未映射/越界返回 `Unmapped` |
| BUS-5 | `peek` 无副作用且不发事件 |
| BUS-6 | `write_bytes` 不发事件，不得跨设备边界 |
| BUS-7 | `irq_line` 是 level 语义 |
| BUS-8 | 中断优先级不在 Bus 内裁剪，由 CPU 裁决 |

## 8. 事件

| 编号 | 不变量 |
|---|---|
| EV-1 | UART 输出先更新 Terminal / CLI sink，再进入 RingCollector |
| EV-2 | `PowerOff` 对应宿主 exit 0 |
| EV-3 | `Reboot` 对应宿主 exit 2 |
| EV-4 | `Halt { code }` 仍表示 guest 退出协议或显式宿主退出 |
| EV-5 | 事件环容量到达上限时丢弃最旧事件 |

## 9. Machine

| 编号 | 不变量 |
|---|---|
| M-1 | `from_config` 是设备映射和镜像加载的唯一入口 |
| M-2 | 镜像段必须完全位于 RAM 范围 |
| M-3 | 入口优先级为 `cfg.entry` > ELF entry |
| M-4 | 断点在取指前检查，监视点在执行后检查 |
| M-5 | trap 不自动终止 Machine |
| M-6 | `snapshot_compact` 不克隆事件环 |
| M-7 | 快照是只读视图 |

## 10. Config

| 编号 | 不变量 |
|---|---|
| CFG-1 | 默认布局常量唯一来源为 `rremu-config` |
| CFG-2 | `validate` 拒绝设备重叠、非法尺寸、非法 RAM 范围 |
| CFG-3 | DTB 必须放在 RAM 顶部保留区内 |
| CFG-4 | Target：resolve 后的配置不可变 |

## 11. HAL ABI

| 编号 | 不变量 |
|---|---|
| ABI-1 | `Context` / `SContext` 为 `repr(C)`，字段顺序固定 |
| ABI-2 | `Context` 35 个 u32，分配 144 字节，16 字节对齐 |
| ABI-3 | `gpr[2]` 保存 trap 前原始 sp |
| ABI-4 | `gpr[17]` 用于 Event 解码 |
| ABI-5 | `TrapInfo` 为 `repr(C)`，2 个 u32 |
| ABI-6 | 默认 trap 退出码为 `0x10` |
| ABI-7 | `enter_supervisor` 设置 MPP=S、mepc、a0/a1 后 `mret` |
| ABI-8 | `halt` 使用 M-mode ecall + `EXIT_MAGIC` |

## 12. 测试映射

第一层重构期间优先运行：

```text
rremu:
  cargo test -p rremu-cpu
  cargo test -p rremu-bus
  cargo test -p rremu-machine
  cargo test -p rremu-config
  cargo test -p rremu-devices

rremu-am:
  cargo test --workspace
```

第三层 `rremu-test` 已部分恢复规范回归；当前 57 个用例，第一层重构不得减少覆盖。
