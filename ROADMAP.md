# ROADMAP: RT-Thread → OpenSBI → Linux

> 目标：让 rremu 从「能跑 RT-Thread」逐步演进到「能跑 Linux」。
> 状态：**S0 / S1 / S2 已完成**；**S3（Linux）待做**。

## 阶段与状态

| 阶段 | 目标 | 状态 |
|---|---|---|
| S0 | 补齐 `rdtime`/`misa`/访存翻译钩子/PLIC/启动注入 | ✅ 已合并 |
| S1 | UART RX 中断 → PLIC，TUI/CLI 与 RT-Thread 交互 | ✅ 已合并 |
| — | UART 终端视图重构（ANSI 状态机：C0/CSI/SGR/UTF-8） | ✅ 已合并 |
| S2 | Supervisor / S-mode CSR / 委派 / PMP / DTB / `sfence.vma` / `sret`，**跑起 OpenSBI** | ✅ 已合并 |
| S3 | Sv32 地址翻译、S/U 完整、中断注入 S、RTC/virtio、SBI shutdown、**跑 Linux** | ⏳ 待做 |

## 已具备能力

- **ISA**：RV32I + M + A + C + Zicsr，`fence`/`fence.i`、`sfence.vma`(NOP)、`wfi`(NOP)、
  `mret`/`sret`。
- **特权**：M / S / U 三级；`mstatus` 的 MPP/MPIE/MIE 与 SPP/SPIE/SIE。
- **CSR**：M-mode 全套（含 `mtval`/`medeleg`/`mideleg`/`pmpcfg`/`pmpaddr`）、
  S-mode（`sstatus/sie/sip/stvec/sscratch/sepc/scause/stval/satp`）、
  非特权计数器（`cycle/time/instret/timeh`）。
- **陷阱**：中断优先级 MEI>MSI>MTI；按 `medeleg/mideleg` 委派到 S；`trap` 写全现场。
- **设备**：RAM、UART(NS16550A 子集，含 RX 中断)、CLINT、PLIC、Framebuffer。
- **启动**：ELF 加载、PIE 按基址加载（`--load-base`）、DTB 加载并以 `a1` 传递（`--dtb`）、
  `a0=hartid`。
- **诊断**：结构化 dump（JSON）+ `rremu-report` 人可读报告；ANSI 终端视图。

## 已知限制（S3 处理）

- **PMP 仅存储、不强制访问控制**（OpenSBI 能配置，但 S/U 越权不被拦截）。
- **`satp` 仅存储**，未实现 Sv32 页表遍历（`Cpu::translate` 仍 bare 直通）。
- **S 级中断未注入**：`mideleg` 已存，但中断仍全部在 M 处理；CLINT/PLIC 中断未转成
  `sip`/`scause` 交 S。
- 无 RTC / virtio 等 Linux 常用设备。

## S3 计划（Linux）

1. **Sv32 地址翻译**：填充 `rremu-cpu::translate`（页表遍历 + 故障→`scause` page fault）。
2. **中断注入**：PLIC/CLINT → `mip`/`sip`，按 `mideleg` 交 S；`stimecmp`/Sstc 可选。
3. **S/U 特权完整**：U 模式访存、`sret` 回 U、必要的 PMP 强制。
4. **设备**：goldfish RTC、virtio（按 Linux 需求）。
5. **构建**：OpenSBI `FW_PAYLOAD`（或 `FW_JUMP` + Linux Image）；SBI `shutdown` 统一停机。
6. **DTB**：扩展 `platforms/rremu.dts`（新增 RTC/virtio 节点、`bootargs`）。

## 持续任务：测试补强（rremu-test）

后续将落大量新功能/新提交，`rremu-test` 的端到端覆盖必须同步跟上。

- **原则**：每个新功能合并时至少补一个 `guest/src/bin/*.rs` + `Tests.toml` 条目，
  保持「功能 = 用例」可追溯，避免只靠 rremu 单测。
- **当前覆盖**：34 用例，集中在 M-mode ISA / CTE / CSR / 设备（见 `rremu-test/README.md`）。
- **待补缺口**（按优先级）：
  1. **S-mode**：`sret` 回 S/U、`sfence.vma`、S-CSR（sstatus/sie/sip/stvec/sepc/scause/stval/satp）。
  2. **委派与中断**：`medeleg/mideleg` 委派后陷阱在 S 处理；PLIC → MEI 链路；S 级中断注入。
  3. **PMP**：配置后的访问控制（S3 强制实现后）。
  4. **启动注入**：`--dtb` 传入 `a1`、`a0=hartid`、`--load-base` PIE 加载。
  5. **dump 正确性**：寄存器/CSR/trace/last_trap/sha256 字段与 `rremu-report` 解析。
  6. **RV32A/边界**：`lr`/`sc` 失败语义、AMO 溢出、非对齐、CLINT `mtimecmp` 边界。
  7. **回归**：为已修复缺陷各留一条永久用例（防回退）。

## 关键约定

- **退出协议**：M-mode `ecall` + `a7 = 0xCAFE_0000`，以 `a0` 为退出码。
- **平台定义**：`platforms/rremu.dts`（地址与 `rremu-config` 默认值一致；
  `rremu-am/arch.rs` 为人工镜像，由 rremu-test 护栏测试锁定）。
- **OpenSBI 运行**：见 `platforms/README.md`（PIE `--load-base`、`FW_JUMP_FDT_ADDR` 须在 RAM 内、
  `FW_TEXT_START` 保持默认 0）。
