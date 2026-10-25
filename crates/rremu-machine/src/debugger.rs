//! 断点、监视点与调试状态。

use std::collections::BTreeSet;

use rremu_expr::Expr;

use crate::RunResult;

/// 监视点：跟踪某表达式的值变化（值语义，EXPR 与 p 命令同模型）。
#[derive(Debug, Clone)]
pub struct Watchpoint {
    pub id: usize,
    pub expr: Expr,
    pub last_value: u32,
    pub hit_count: u32,
}

/// 调试控制器：管理断点、监视点及其命中/放行逻辑。
///
/// 断点在取指前检查；监视点在指令执行后检查。`bp_suppress` 使刚命中的断点
/// 地址在下一次放行一次（经典调试器越过断点语义）。
pub struct DebugController {
    breakpoints: BTreeSet<u32>,
    watchpoints: Vec<Watchpoint>,
    next_wp_id: usize,
    bp_suppress: Option<u32>,
}

impl DebugController {
    pub(crate) fn new() -> Self {
        Self {
            breakpoints: BTreeSet::new(),
            watchpoints: Vec::new(),
            next_wp_id: 1,
            bp_suppress: None,
        }
    }

    /// 取指前断点检查：命中且未被 suppress 则返回 `HitBreakpoint` 并置 suppress。
    pub(crate) fn check_breakpoint(&mut self, pc: u32) -> Option<RunResult> {
        if self.breakpoints.contains(&pc) && self.bp_suppress != Some(pc) {
            self.bp_suppress = Some(pc);
            Some(RunResult::HitBreakpoint { addr: pc })
        } else {
            None
        }
    }

    /// 执行后清除一次 suppress（无论是否命中，每步至多放行一次）。
    pub(crate) fn clear_suppress(&mut self) {
        self.bp_suppress = None;
    }

    /// 更新某监视点的值；若变化则返回 `(old, new)`，否则 `None`。
    pub(crate) fn update_if_changed(&mut self, id: usize, new: u32) -> Option<(u32, u32)> {
        let wp = self.watchpoints.iter_mut().find(|w| w.id == id)?;
        if new != wp.last_value {
            let old = wp.last_value;
            wp.last_value = new;
            wp.hit_count += 1;
            Some((old, new))
        } else {
            None
        }
    }

    /// 所有监视点的 (id, expr) 快照：调用方求值后再回调 update_if_changed。
    pub(crate) fn watchpoint_entries(&self) -> Vec<(usize, Expr)> {
        self.watchpoints
            .iter()
            .map(|w| (w.id, w.expr.clone()))
            .collect()
    }

    pub(crate) fn add_breakpoint(&mut self, addr: u32) -> bool {
        self.breakpoints.insert(addr)
    }

    pub(crate) fn remove_breakpoint(&mut self, addr: u32) -> bool {
        self.breakpoints.remove(&addr)
    }

    pub(crate) fn breakpoints(&self) -> &BTreeSet<u32> {
        &self.breakpoints
    }

    pub(crate) fn add_watchpoint(&mut self, expr: Expr, initial: u32) -> usize {
        let id = self.next_wp_id;
        self.next_wp_id += 1;
        self.watchpoints.push(Watchpoint {
            id,
            expr,
            last_value: initial,
            hit_count: 0,
        });
        id
    }

    pub(crate) fn remove_watchpoint(&mut self, id: usize) -> bool {
        let before = self.watchpoints.len();
        self.watchpoints.retain(|w| w.id != id);
        self.watchpoints.len() != before
    }

    pub(crate) fn watchpoints(&self) -> &[Watchpoint] {
        &self.watchpoints
    }

    pub(crate) fn reset(&mut self) {
        self.bp_suppress = None;
    }
}
