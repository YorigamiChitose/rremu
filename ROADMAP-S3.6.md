# S3.6 定时器/唤醒正确性

> 状态：**已完成**；S3.4a 已完成，磁盘/真实块设备已移出计划。

## 目标

在启用以下两项后，Linux 仍能进入内置 initramfs 的 `/init` 并 poweroff：

```text
CONFIG_REGULATOR=y
CONFIG_DETECT_HUNG_TASK=y
```

验收标准：

```text
debug_vm_pgtable
clk: Disabling unused clocks
Run /init as init process
reboot: Power down
Program exited with code 0.
```

## 已知事实

- 冻结配置关闭两项后，约 4 秒 kernel time 完成启动并 poweroff。
- `CONFIG_REGULATOR` 单独启用即可复现 init 卡住。
- `CONFIG_DETECT_HUNG_TASK` 单独启用即可复现 init 卡住。
- 卡住时：
  - 无 `clk: Disabling unused clocks`
  - PC 在 `arch_cpu_idle` / `cpu_idle_poll_ctrl`
  - `instret` 持续增长
- `REGULATOR` 的 `regulator_init_complete()` 会安排 30 秒 delayed work。
- `DETECT_HUNG_TASK` 会创建 `khungtaskd`，内部使用 `schedule_timeout_interruptible()`。

## 根因与修复

根因不是 CLINT 定时器本身，而是 **CSR 存在性探测错误**：

1. rremu 对未实现 CSR 的读取返回 0，而不是 illegal instruction。
2. OpenSBI 用“读取是否成功”探测可选扩展，误判 SSTC 可用。
3. OpenSBI 的 SBI timer 因此写 `stimecmp` CSR，而不是 CLINT `mtimecmp`。
4. rremu 未实现 `stimecmp`，写入被忽略；CLINT 定时器从未编程。
5. Linux 收不到定时器中断，delayed work 与 `khungtaskd` 永远不唤醒。

修复内容：

- 未实现 CSR 的读写触发 illegal instruction。
- 实现 RV32 `mstatush` / `sstatush`（高 32 位当前恒为 0）。
- 实现只读机器 ID CSR：`mvendorid`、`marchid`、`mimpid`。
- 增加未实现 CSR 读写回归测试。

## 验证结果

冻结配置同时启用两项后，30 秒测试通过：

```text
[    4.333920] debug_vm_pgtable: Validating architecture page table helpers
[    4.367724] clk: Disabling unused clocks
[    4.554902] Run /init as init process
[    4.578888] reboot: Power down
Program exited with code 0.
```

单独启用 `REGULATOR`、单独启用 `DETECT_HUNG_TASK` 也分别在 30 秒内通过。

此前 `待区分` 中的“真实唤醒缺陷 vs 仿真时间膨胀”已有结论：

```text
真实缺陷：SBI timer 被错误路由到未实现的 stimecmp CSR
```

## 待区分

1. **真实唤醒缺陷**
   - SBI timer / CLINT `mtimecmp`
   - STIP / S 级中断注入
   - idle 唤醒
   - delayed work / 调度器唤醒
2. **仿真时间膨胀**
   - 30 秒 kernel time 在 wall time 下远未到期
   - 当前约 30 秒 wall time 只推进 4–5 秒 kernel time
3. **调度/上下文切换问题**
   - 创建睡眠任务后，`kernel_init` 未恢复
   - idle 循环或 trap 返回路径错误

## 复现矩阵

| 配置 | 目的 |
|---|---|
| 冻结配置，两项关闭 | 基线，必须成功 |
| 冻结配置，仅 `REGULATOR=y` | 隔离 regulator delayed work |
| 冻结配置，仅 `DETECT_HUNG_TASK=y` | 隔离 khungtaskd 睡眠 |
| 冻结配置，两项开启 | 最终目标 |
| 最小配置，仅 `REGULATOR=y` | 排除其他驱动干扰 |
| 最小配置，仅 `DETECT_HUNG_TASK=y` | 排除其他驱动干扰 |

## 时间阶梯

```text
30s → 60s → 300s
```

只有在 30s 有明确推进时才升到 60s，再升到 300s。

300s 用于判断 30 秒 delayed work 是否只因仿真时间不足而未到期。
`DETECT_HUNG_TASK` 的长时间睡眠可能需要更长时间，不能只用 300s 下结论。

## 诊断记录

实际定位过程：

1. 仅启用 `REGULATOR`，60s / 300s 仍停在 `debug_vm_pgtable`。
2. 检查 CLINT：`mtimecmp` 始终为 `0xffffffff`，`mtime` 持续增长。
3. 记录 SBI TIME ecall：Linux 已请求 timer，但 OpenSBI 未写 CLINT。
4. 定位到 OpenSBI 误判 SSTC，原因是对未实现 CSR 的读取成功。
5. 修复 CSR 存在性检查，并补齐 RV32 `mstatush` 等必要 CSR。

## 非目标

- 磁盘、真实块设备、virtio-blk、rootfs
- 磁盘镜像与持久化 rootfs

## 完成条件

- [x] 冻结配置启用 `REGULATOR` 与 `DETECT_HUNG_TASK` 后仍能完成启动。
- [x] 至少一条永久回归用例覆盖本次根因（`unimplemented_csr_*`）。
- [x] 更新 README / ROADMAP 限制说明。
