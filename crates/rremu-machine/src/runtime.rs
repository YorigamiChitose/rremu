//! Machine 执行循环与运行状态转换。

use std::sync::atomic::{AtomicBool, Ordering};

use rremu_bus::{EmuEvent, EventSink};
use rremu_cpu::StepOutcome;

use crate::events::MachineSink;
use crate::{Machine, RunResult, RunState};

impl Machine {
    /// 单步执行 n 条；中途退出（Halt/Trap）即返回。
    pub fn step_n(&mut self, n: u64) -> RunResult {
        self.state = RunState::Running;
        for _ in 0..n {
            if let Some(rr) = self.step_once() {
                return rr;
            }
        }
        self.state = RunState::Paused;
        RunResult::Paused
    }

    /// 全速运行；pause 置位后停止（Ctrl+C 由前端控制）。
    /// 非致命陷入（ecall/ebreak 等）已被 CPU 重定向至 mtvec，run 不停止；
    /// 仅 Halt、断点/监视点命中或 pause 置位时返回。
    pub fn run(&mut self, pause: &AtomicBool) -> RunResult {
        self.state = RunState::Running;
        loop {
            if pause.load(Ordering::Relaxed) {
                self.state = RunState::Paused;
                return RunResult::Paused;
            }
            match self.step_once() {
                Some(RunResult::Exited { code }) => {
                    self.state = RunState::Exited(code);
                    return RunResult::Exited { code };
                }
                Some(RunResult::Faulted { reason }) => {
                    return RunResult::Faulted { reason };
                }
                Some(rr @ (RunResult::HitBreakpoint { .. } | RunResult::HitWatchpoint { .. })) => {
                    self.state = RunState::Paused;
                    return rr;
                }
                Some(RunResult::Paused) | None => {}
            }
        }
    }

    /// 执行一条指令；需要停止时返回 Some(RunResult)。
    /// 断点在取指前检查（命中则不执行该指令）；监视点在执行后检查。
    /// 注意：陷入（ecall/ebreak/misaligned/中断）是非致命的 —— CPU 已将 pc
    /// 重定向至 mtvec，陷入向量代码将在下一轮 step_once 中执行。state 保持
    /// Running，以便前端可继续 c/si；陷入信息经 EmuEvent::Trap 进入 mem 环。
    fn step_once(&mut self) -> Option<RunResult> {
        let pc_before = self.cpu.pc();

        // 断点：命中则不执行，直接返回；刚命中过的地址放行一次（越过断点）
        if let Some(rr) = self.dbg.check_breakpoint(pc_before) {
            self.state = RunState::Paused;
            return Some(rr);
        }
        self.dbg.clear_suppress();

        let stepped = {
            let mut sink = MachineSink::new(
                &mut self.collector,
                &mut self.terminal,
                &mut self.uart_sink,
            );
            self.cpu.step(&mut self.bus, &mut sink)
        };
        match stepped {
            Ok(StepOutcome::Executed { len, bits, .. }) => {
                self.push_trace(pc_before, bits, len);
                if let Some(code) = self.collector.take_halt() {
                    self.state = RunState::Exited(code);
                    return Some(RunResult::Exited { code });
                }
                // 监视点：检查每个被跟踪表达式的值是否变化
                if let Some(rr) = self.check_watchpoints(pc_before) {
                    self.state = RunState::Paused;
                    return Some(rr);
                }
                None
            }
            // ebreak/c.ebreak 宿主陷阱：以 a0 为退出码终止，发出 Halt 事件
            Ok(StepOutcome::Exited { code }) => {
                self.collector.emit(EmuEvent::Halt { code });
                self.state = RunState::Exited(code);
                Some(RunResult::Exited { code })
            }
            // 非致命陷入：CPU 已重定向 pc 至 mtvec，本步无停止结果
            Ok(StepOutcome::Trapped { .. }) => {
                let c = self.cpu.csr_state();
                self.last_trap = Some((c.mcause, c.mepc, c.mtval));
                // 冻结崩溃前的调用路径（此后 handler 指令不再污染）
                self.trace_at_trap = Some(self.trace.iter().copied().collect());
                None
            }
            Err(e) => {
                // 不可恢复错误（如 vectored mtvec 未实现）：终止模拟，避免静默死循环。
                let reason = e.to_string();
                self.state = RunState::Faulted(reason.clone());
                Some(RunResult::Faulted { reason })
            }
        }
    }


}
