# ROADMAP: RT-Thread → OpenSBI → Linux

> 目标：让 rremu 从「能跑 RT-Thread」逐步演进到「能跑 Linux」。
> 状态：**S0 / S1 / S2 / S3 已完成**；磁盘/真实块设备已移出计划。

## 阶段与状态

| 阶段 | 目标 | 状态 |
|---|---|---|
| S0 | 补齐 `rdtime`/`misa`/访存翻译钩子/PLIC/启动注入 | ✅ 已合并 |
| S1 | UART RX 中断 → PLIC，TUI/CLI 与 RT-Thread 交互 | ✅ 已合并 |
| — | UART 终端视图重构（ANSI 状态机：C0/CSI/SGR/UTF-8） | ✅ 已合并 |
| S2 | Supervisor / S-mode CSR / 委派 / PMP / DTB / `sfence.vma` / `sret`，**跑起 OpenSBI** | ✅ 已合并 |
| S3 | S-mode 基线、Sv32、S 中断、U/PMP、Linux 最小启动、回归 | ✅ 已完成 |

## 已具备能力

- **ISA**：RV32I + M + A + C + Zicsr，`fence`/`fence.i`、`sfence.vma`(NOP)、`wfi`(NOP)、
  `mret`/`sret`。
- **特权**：M / S / U 三级；`mstatus` 的 MPP/MPIE/MIE 与 SPP/SPIE/SIE。
- **CSR**：M-mode 全套（含 `mtval`/`medeleg`/`mideleg`/`pmpcfg`/`pmpaddr`）、
  S-mode（`sstatus/sie/sip/stvec/sscratch/sepc/scause/stval/satp`）、
  非特权计数器（`cycle/time/instret/timeh`）。
- **陷阱**：中断优先级 MEI>MSI>MTI>SEI>SSI>STI；按 `medeleg/mideleg` 委派到 S；`trap` 写全现场。
- **设备**：RAM、UART(NS16550A 子集，含 RX 中断)、CLINT、PLIC、Goldfish RTC、
  syscon(poweroff/reboot)、Framebuffer。
- **启动**：ELF 加载、PIE 按基址加载（`--load-base`）、DTB 加载并以 `a1` 传递（`--dtb`）、
  `a0=hartid`。
- **诊断**：结构化 dump（JSON）+ `rremu-report` 人可读报告；ANSI 终端视图。

## 已知限制（S3 处理）

- 磁盘、真实块设备、virtio-blk、rootfs 已移出计划；Linux 使用内置 initramfs。

## S3 计划（Linux 与 S-mode 完整化）

> **当前进度：S3.5 已完成**；S3 全部完成（Linux 进入 init 并 poweroff）。

> 每个阶段结束都必须同时满足：
> `cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、
> `rremu-test` 全量通过、OpenSBI smoke test 不回归。功能与端到端用例同阶段交付，
> 不接受「先上功能、测试后补」。

### S3.0 S-mode 基线与 S2 测试补强（✅ 已完成）

先修正 S2 遗留的语义问题，再进入 MMU；避免在错误的 S-mode 语义上叠加 Sv32。

- **ISA/CPU**：增加 `ECALL_FROM_S(9)` 并按 `Priv` 映射 ecall cause；补 S 中断 cause 常量。
- **CSR 权限**：按地址 bit[9:8] 检查特权级；只读 CSR（bit[11:10]=0b11）写入触发 illegal；
  `misa` 改为只读/WARL；`mstatus` 写入做基本 WARL 掩码。
- **AM**：`Event`/`decode_event` 支持 S-mode ecall；保存并暴露 boot `a0/a1`；
  增加最小 S-mode 入口辅助（`enter_supervisor`）；需要委派到 S 时增加 S 陷阱入口。
- **可测试性**：`--load-base` 重定位与 DTB 放置逻辑从 `bin` 下沉到可复用 API，
  harness 增加 `load_base`/`dtb`/boot args 等配置字段。
- **用例**：`smode_mret_sret_s`、`smode_sret_u`、`smode_csr_views`、`smode_ecall_s`、
  `smode_delegate_ecall`、`smode_illegal_mcsr`、`smode_sfence`、`boot_args`、`load_base_pie`。
- **验收**：现有 34 用例 + 新增 S-mode 用例全过；OpenSBI 启动日志不回归。

### S3.1 Sv32 地址翻译（✅ 已完成）

- `Cpu::translate` 改为携带访问类型的结果：`(va, Fetch|Load|Store) -> Result<pa, TrapErr>`。
- `satp.MODE=0` 或 M-mode（未启用 MPRV）保持 bare；`MODE=1` 执行 Sv32 两级页表遍历。
- 支持 4KiB 页、4MiB 大页、PTE 的 `V/R/W/X/U/G/A/D`；A/D 优先硬件置位并回写。
- 权限：S 访问 U 页需 `SUM`；`MXR` 控制可执行页的 load；U-mode 仅能访问 U 页。
- 异常：取指 page fault 12、load 13、store/AMO 15；`mtval/stval` 记录原始 VA。
- 取指跨 4KiB 页时逐段翻译/读取（含压缩指令），不得假设 `pa` 连续。
- `satp` 写入做 WARL；`sfence.vma` 仍可 NOP（无 TLB）。
- **用例**：identity mapping、非恒等映射、大页、R/W/X fault、U 页与 `SUM`、A/D、
  切换 `satp`、页边界取指。
- **验收**：旧用例与 OpenSBI 不回归；MMU 用例全过。

### S3.2 S 级中断注入（✅ 已完成）

- CPU 中断仲裁完整化：MEI > MSI > MTI > SEI > SSI > STI；已委派中断不在 M 处理；
  S 中断需 `mstatus.SIE` + `sie` + `sip` 同时成立。
- `trap_supervisor` 正确写 `scause/sepc/stval` 与 `sstatus.SPP/SPIE/SIE`。
- PLIC 按 context 独立：M ctx → `M_EI`，S ctx → `S_EI`；claim/complete 分 context。
- `mip`/`sip` 明确设备投影位与软件可写位，保留 OpenSBI 注入的 `sip.STIP`。
- CLINT 保持 M timer；S timer 由 OpenSBI 通过 `sip.STIP` 注入。
- **用例**：S 软件中断、S 定时器、PLIC → S 外部中断（UART RX）、未委派仍进 M、
  优先级/使能门控、`sret` 后中断状态恢复。
- **验收**：M/S 中断用例全过；OpenSBI 启动不回归。

### S3.3 U-mode 与 PMP 强制（✅ 已完成，待合并）

- PMP 匹配器支持 `OFF/TOR/NA4/NAPOT`、R/W/X、locked；M-mode 默认放行，S/U 必须匹配。
- 取指、load、store/AMO 均在物理地址上检查；失败映射为取指 1 / load 5 / store 7。
- 特权完整性：`sret` 回 U、U-mode CSR 限制、`MPRV/SUM/MXR` 必要组合；
  `TSR/TW` 检查可后置。
- **兼容性**：OpenSBI 配置的 PMP 必须能放行 S-mode；若失败优先修匹配器，不回退强制。
- **用例**：NAPOT 允许/拒绝、TOR 边界、locked、U 访问非 U 页失败、U syscall、
  OpenSBI 启动回归。

### S3.4 Linux 设备与最小启动（✅ S3.4a 已完成）

磁盘、真实块设备、virtio-blk、rootfs 明确移出本计划；Linux 使用内置 initramfs。

- **S3.4a 最小启动（✅ 已完成）**：goldfish RTC、syscon/test（poweroff/reset）；DTS 增加 RTC/syscon、
  `bootargs`、`stdout-path`；OpenSBI `FW_PAYLOAD`；新增 `platforms/run-linux.sh`。
  验收：Linux 启动日志、initramfs `/init`、`poweroff` 停机。

### S3.5 回归与清理（✅ 已完成）

- 永久回归用例已补齐：
  - S-mode ecall：`smode_delegate_ecall`
  - CSR 权限 / 委派：`smode_illegal_mcsr` / `smode_csr` / `smode_delegate_ecall`
  - PMP：`pmp_enforce_load` / `pmp_enforce_store`
  - MMU page fault：`sv32_page_fault` / `sv32_store_page_fault`
  - S 中断：`s_irq_software` / `s_irq_timer` / `s_irq_external`
  - CSR 存在性（S3.6）：`csr_unimplemented`
- 更新 README / ROADMAP / rremu-test 用例清单（53 个）。
- 限制保留：
  - vectored `mtvec/stvec` 未实现；`cte_vectored_mtvec` 继续断言 Fault。
  - `wfi` 按 NOP 执行；当前不影响 Linux idle/唤醒，暂不做 idle 优化。

### S3.6 定时器/唤醒正确性（REGULATOR + DETECT_HUNG_TASK，✅ 已完成）

冻结配置已启用：

```text
CONFIG_REGULATOR=y
CONFIG_DETECT_HUNG_TASK=y
```

根因不在 CLINT 定时器本身，而在 CSR 存在性探测：

- rremu 对未实现 CSR 的读取返回 0，而不是触发 illegal instruction。
- OpenSBI 通过“能否读取 CSR”探测可选扩展，因此误判 SSTC 可用。
- OpenSBI 随后把 SBI timer 写入 `stimecmp` CSR；该 CSR 实际未实现，写入被忽略。
- CLINT `mtimecmp` 从未被编程，Linux 收不到定时器中断，delayed work / `khungtaskd` 永远不被唤醒。

修复：

- 未实现 CSR 读写一律触发 illegal instruction。
- 补齐 OpenSBI 启动所需的 RV32 `mstatush`、`sstatush`。
- 补齐只读机器 ID CSR：`mvendorid`、`marchid`、`mimpid`。
- 增加未实现 CSR 读写回归测试。

验证：冻结配置同时启用两项后，30 秒内完成：

```text
debug_vm_pgtable
clk: Disabling unused clocks
Run /init as init process
reboot: Power down
Program exited with code 0.
```

详细记录见 `ROADMAP-S3.6.md`。

## 开发流程（分支与最小提交）

- `main` 始终保持可构建、可测试；**禁止直接在 `main` 上修改**。
- 每个任务从最新 `main` 新建测试分支，完成后经审查再合并：
  - 规划/文档：`test/s3-plan`
  - S3.0：`test/s3.0-smode-baseline`
  - S3.1：`test/s3.1-sv32`
  - S3.2：`test/s3.2-s-interrupts`
  - S3.3：`test/s3.3-u-pmp`
  - S3.4：`test/s3.4-linux`
- 跨仓库改动（rremu / rremu-am / rremu-test）在各自仓库建立**同名阶段分支**，分别提交，
  保留对应测试；不把跨仓库改动混进单个仓库提交。
- **最小提交**：一个提交只做一件逻辑变更；功能提交必须包含对应测试，或紧随一个测试提交。
- 提交信息遵循 Conventional Commits；未通过该阶段验收的分支不得合并。

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
