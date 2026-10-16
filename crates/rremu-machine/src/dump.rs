//! dump 模块：机器状态的结构化序列化格式（format_version = 1）。
//!
//! rremu 在终止时把机器状态序列化为 JSON；由离线工具（rremu-report）解析。
//! 数值一律用 hex 字符串（避免 JSON double 精度损失，贴合 RISC-V 习惯）；
//! 事件用字符串标签（`#[serde(tag = "type")]`）。

use serde::{Deserialize, Serialize};

use rremu_bus::EmuEvent;

use crate::{Machine, RunState, TraceEntry};

/// dump 格式版本。
pub const FORMAT_VERSION: u32 = 1;

/// dump 保留的最大事件数（= 2 × trace 深度）。
pub const EVENT_CAPACITY: usize = 64;

/// u32 格式化为 8 位 hex 字符串（含 0x 前缀）。
pub fn hex(v: u32) -> String {
    format!("0x{v:08x}")
}

/// 顶层 dump 结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dump {
    pub format_version: u32,
    pub rremu_version: String,
    pub reason: Reason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elf: Option<ElfRef>,
    pub machine: MachineState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trap: Option<Trap>,
    pub trace: Vec<TraceItem>,
    #[serde(default)]
    pub events: Vec<EventItem>,
}

/// 终止原因。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reason {
    /// 正常/异常退出（halt code）。
    Halt { code: i32 },
    /// 模拟器不可恢复错误。
    Faulted { reason: String },
    /// 未退出（步数上限/暂停）。
    Paused,
}

/// ELF 引用（供离线工具符号化）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElfRef {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

impl ElfRef {
    /// 从路径构造（计算 sha256；读取失败时 sha256 为 None）。
    pub fn from_path(path: &str) -> Self {
        let sha256 = std::fs::read(path).ok().map(|b| sha256_hex(&b));
        Self { path: path.to_owned(), sha256 }
    }
}

/// 字节串的 sha256 十六进制摘要。
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// 机器状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineState {
    pub cycle: u64,
    pub instret: u64,
    pub pc: String,
    #[serde(rename = "priv")]
    pub priv_: String,
    pub gprs: Vec<String>,
    pub csrs: CsrState,
}

/// CSR 集合。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsrState {
    pub mstatus: String,
    pub mtvec: String,
    pub mepc: String,
    pub mcause: String,
    pub mtval: String,
    pub mie: String,
    pub mip: String,
    pub mcycle: u64,
    pub minstret: u64,
}

/// 最近一次 trap（取自事件环）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trap {
    pub cause: String,
    pub pc: String,
    pub mtval: String,
}

/// 指令 trace 条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceItem {
    pub pc: String,
    pub bits: String,
    pub len: u8,
}

/// IO 事件条目（字符串标签）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventItem {
    Mem { cycle: u64, kind: String, size: String, addr: String, value: String, device: String },
    Trap { cause: String, pc: String },
    UartOut { byte: u8 },
    UartIn { byte: u8 },
}

impl EventItem {
    fn from_event(ev: &EmuEvent) -> Option<Self> {
        match ev {
            EmuEvent::MemAccess { cycle, kind, size, addr, value, device } => Some(EventItem::Mem {
                cycle: *cycle,
                kind: kind.to_string(),
                size: size.to_string(),
                addr: hex(*addr),
                value: hex(*value),
                device: device.clone(),
            }),
            EmuEvent::Trap { cause, pc } => {
                Some(EventItem::Trap { cause: hex(*cause), pc: hex(*pc) })
            }
            EmuEvent::UartOutput { byte } => Some(EventItem::UartOut { byte: *byte }),
            EmuEvent::UartInput { byte } => Some(EventItem::UartIn { byte: *byte }),
            EmuEvent::Halt { .. } => None,
        }
    }
}

impl Dump {
    /// 从 machine 构造 dump（纯读取，无副作用）。`elf` 引用由调用方提供。
    pub fn from_machine(machine: &Machine, elf: Option<ElfRef>) -> Self {
        let snap = machine.snapshot();
        let reason = match &snap.state {
            RunState::Exited(code) => Reason::Halt { code: *code },
            RunState::Faulted(r) => Reason::Faulted { reason: r.clone() },
            _ => Reason::Paused,
        };

        let gprs: Vec<String> = snap.gprs.iter().map(|v| hex(*v)).collect();

        let csrs = CsrState {
            mstatus: hex(snap.csrs.mstatus),
            mtvec: hex(snap.csrs.mtvec),
            mepc: hex(snap.csrs.mepc),
            mcause: hex(snap.csrs.mcause),
            mtval: hex(snap.csrs.mtval),
            mie: hex(snap.csrs.mie),
            mip: hex(snap.csrs.mip),
            mcycle: snap.csrs.mcycle,
            minstret: snap.csrs.minstret,
        };

        // 事件：mem 环 + uart 环合并，截取最近 EVENT_CAPACITY 条
        let mut events: Vec<EventItem> = machine
            .mem_ring()
            .iter()
            .chain(machine.uart_ring().iter())
            .filter_map(EventItem::from_event)
            .collect();
        if events.len() > EVENT_CAPACITY {
            events.drain(..events.len() - EVENT_CAPACITY);
        }

        // trap 从 Machine 持久保留的 last_trap 取（不受事件环淘汰影响）
        let trap = machine
            .last_trap()
            .map(|(cause, pc, mtval)| Trap { cause: hex(cause), pc: hex(pc), mtval: hex(mtval) });

        // trace 优先用 trap 时刻冻结的崩溃路径；否则用当前 trace
        let to_item = |t: &TraceEntry| TraceItem { pc: hex(t.pc), bits: hex(t.bits), len: t.len };
        let trace: Vec<TraceItem> = match machine.trace_at_trap() {
            Some(frozen) => frozen.iter().map(to_item).collect(),
            None => machine.trace().iter().map(to_item).collect(),
        };

        Dump {
            format_version: FORMAT_VERSION,
            rremu_version: env!("CARGO_PKG_VERSION").to_owned(),
            reason,
            elf,
            machine: MachineState {
                cycle: snap.cycle,
                instret: snap.csrs.minstret,
                pc: hex(snap.pc),
                priv_: snap.priv_.to_string(),
                gprs,
                csrs,
            },
            trap,
            trace,
            events,
        }
    }

    /// 序列化为格式化 JSON 字符串。
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// 从 JSON 字符串反序列化。
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // 测试内放行
mod tests {
    use super::*;
    use crate::tests::test_machine;
    use rremu_loader::{Image, Segment};

    fn nop_image(entry: u32) -> Image {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x0000_0013u32.to_le_bytes()); // addi x0, x0, 0
        Image { segments: vec![Segment { addr: entry, data: bytes }], entry }
    }

    #[test]
    fn dump_roundtrip_json() {
        let base = 0x8000_0000;
        let mut m = test_machine(base, 0x1000, nop_image(base));
        m.step_n(1);
        let d = Dump::from_machine(&m, Some(ElfRef { path: "prog.elf".to_owned(), sha256: None }));
        assert_eq!(d.format_version, FORMAT_VERSION);
        assert_eq!(d.reason, Reason::Paused);
        assert_eq!(d.machine.pc, hex(base + 4));
        assert_eq!(d.machine.gprs.len(), 32);
        assert_eq!(d.trace.len(), 1);
        assert_eq!(d.elf.as_ref().unwrap().path, "prog.elf");

        let json = d.to_json().unwrap();
        let back = Dump::from_json(&json).unwrap();
        assert_eq!(back.format_version, d.format_version);
        assert_eq!(back.machine.pc, d.machine.pc);
        assert_eq!(back.trace.len(), 1);
    }

    #[test]
    fn elf_ref_computes_sha256() {
        let p = std::env::temp_dir().join("rremu_dump_sha_test.bin");
        std::fs::write(&p, b"hello").unwrap();
        let r = ElfRef::from_path(p.to_str().unwrap());
        assert_eq!(
            r.sha256.as_deref(),
            Some("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
        );
        let _ = std::fs::remove_file(&p);
        // 不存在的路径 → sha256 为 None
        assert!(ElfRef::from_path("/nonexistent/xyz.elf").sha256.is_none());
    }

    #[test]
    fn dump_includes_trap_and_crash_trace() {
        let base = 0x8000_0000;
        let mut bytes = Vec::new();
        for _ in 0..3 {
            bytes.extend_from_slice(&0x0000_0013u32.to_le_bytes()); // nop
        }
        bytes.extend_from_slice(&0x0000u16.to_le_bytes()); // illegal
        let img = Image { segments: vec![Segment { addr: base, data: bytes }], entry: base };
        let mut m = test_machine(base, 0x1000, img);
        m.step_n(4);
        let d = Dump::from_machine(&m, None);
        let trap = d.trap.as_ref().unwrap();
        assert_eq!(trap.cause, hex(2)); // ILLEGAL_INSTR
        assert_eq!(trap.pc, hex(base + 12));
        // trace 为崩溃前 3 条 nop，不含之后的 handler
        assert_eq!(d.trace.len(), 3);
        assert_eq!(d.trace[2].pc, hex(base + 8));
    }

    #[test]
    fn dump_reason_halt_and_trap_field() {
        let base = 0x8000_0000;
        // lui a7, 0xCAFE0 ; addi a0, x0, 7 ; ecall
        let code: [u32; 3] = [0xCAFE_08B7, 0x0070_0513, 0x0000_0073];
        let mut bytes = Vec::new();
        for w in code {
            bytes.extend_from_slice(&w.to_le_bytes());
        }
        let img = Image { segments: vec![Segment { addr: base, data: bytes }], entry: base };
        let mut m = test_machine(base, 0x1000, img);
        m.step_n(3);
        let d = Dump::from_machine(&m, None);
        assert_eq!(d.reason, Reason::Halt { code: 7 });
    }
}
