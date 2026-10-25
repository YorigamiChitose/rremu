//! CSR 地址与位段常量。

pub const MSTATUS: u16 = 0x300;
/// RV32：mstatus 的高 32 位（SBE/MBE/GVA 等，当前恒为 0）。
pub const MSTATUSH: u16 = 0x310;
pub const MISA: u16 = 0x301;
pub const MIE: u16 = 0x304;
pub const MTVEC: u16 = 0x305;
pub const MSCRATCH: u16 = 0x340;
pub const MEPC: u16 = 0x341;
pub const MCAUSE: u16 = 0x342;
pub const MTVAL: u16 = 0x343;
pub const MIP: u16 = 0x344;
// ---- 浮点 CSR ----
pub const FFLAGS: u16 = 0x001;
pub const FRM: u16 = 0x002;
pub const FCSR: u16 = 0x003;
/// `fcsr` 低 5 位为 `fflags`。
pub const FCSR_FFLAGS_MASK: u32 = 0x1f;
/// `frm` 在 `fcsr` 中的起始位。
pub const FCSR_FRM_SHIFT: u32 = 5;
/// `frm` 在 `fcsr` 中的掩码。
pub const FCSR_FRM_MASK: u32 = 0x7 << FCSR_FRM_SHIFT;
/// `fcsr` 已实现位。
pub const FCSR_MASK: u32 = FCSR_FFLAGS_MASK | FCSR_FRM_MASK;
/// 机器模式计数器使能。
pub const MCOUNTEREN: u16 = 0x306;
// ---- S-mode CSR ----
pub const SSTATUS: u16 = 0x100;
/// RV32：sstatus 的高 32 位（当前恒为 0）。
pub const SSTATUSH: u16 = 0x110;
pub const SIE: u16 = 0x104;
pub const STVEC: u16 = 0x105;
pub const SSCRATCH: u16 = 0x140;
pub const SEPC: u16 = 0x141;
pub const SCAUSE: u16 = 0x142;
pub const STVAL: u16 = 0x143;
pub const SIP: u16 = 0x144;
pub const SATP: u16 = 0x180;
/// 监督者模式计数器使能。
pub const SCOUNTEREN: u16 = 0x106;

// satp 布局（RV32）
pub const SATP_MODE_MASK: u32 = 1 << 31;
pub const SATP_MODE_SV32: u32 = 1 << 31;
pub const SATP_ASID_MASK: u32 = 0x1ff << 22;
pub const SATP_PPN_MASK: u32 = 0x3f_ffff;

// ---- M-mode 委派 ----
pub const MEDELEG: u16 = 0x302;
pub const MIDELEG: u16 = 0x303;

// ---- PMP ----
pub const PMPCFG0: u16 = 0x3A0; // ..0x3A3
pub const PMPADDR0: u16 = 0x3B0; // ..0x3BF

pub const MCYCLE: u16 = 0xB00;
pub const MINSTRET: u16 = 0xB02;
pub const MVENDORID: u16 = 0xF11; // 只读
pub const MARCHID: u16 = 0xF12; // 只读
pub const MIMPID: u16 = 0xF13; // 只读
pub const MHARTID: u16 = 0xF14; // 只读

// 非特权计数器（只读；RT-Thread/OS 常用 rdtime/rdcycle）
pub const CYCLE: u16 = 0xC00;
pub const TIME: u16 = 0xC01;
pub const INSTRET: u16 = 0xC02;
/// RV32 的 time 高 32 位（0xC81）。
pub const TIMEH: u16 = 0xC81;

// misa 扩展位（RV32：MXL=01 位于 bit30）
pub const MISA_A: u32 = 1 << 0;
pub const MISA_C: u32 = 1 << 2;
pub const MISA_D: u32 = 1 << 3;
pub const MISA_F: u32 = 1 << 5;
pub const MISA_I: u32 = 1 << 8;
pub const MISA_M: u32 = 1 << 12;
pub const MISA_S: u32 = 1 << 18;
pub const MISA_U: u32 = 1 << 20;
pub const MISA_MXL_32: u32 = 1 << 30;

// mstatus 位段
pub const MSTATUS_SIE: u32 = 1 << 1;
pub const MSTATUS_MIE: u32 = 1 << 3;
pub const MSTATUS_SPIE: u32 = 1 << 5;
pub const MSTATUS_MPIE: u32 = 1 << 7;
pub const MSTATUS_SPP: u32 = 1 << 8;
pub const MSTATUS_MPRV: u32 = 1 << 17;
pub const MSTATUS_SUM: u32 = 1 << 18;
pub const MSTATUS_MXR: u32 = 1 << 19;
/// `mstatus.FS` 起始位。
pub const MSTATUS_FS_SHIFT: u32 = 13;
pub const MSTATUS_FS_MASK: u32 = 0b11 << MSTATUS_FS_SHIFT;
pub const MSTATUS_FS_OFF: u32 = 0b00 << MSTATUS_FS_SHIFT;
pub const MSTATUS_FS_INITIAL: u32 = 0b01 << MSTATUS_FS_SHIFT;
pub const MSTATUS_FS_CLEAN: u32 = 0b10 << MSTATUS_FS_SHIFT;
pub const MSTATUS_FS_DIRTY: u32 = 0b11 << MSTATUS_FS_SHIFT;
/// `mstatus.SD`：只读汇总位，`FS == Dirty` 时为 1。
pub const MSTATUS_SD: u32 = 1 << 31;

/// sstatus 是 mstatus 的视图，仅暴露 S 相关位与 FS；SD 在读取时单独汇总。
pub const SSTATUS_MASK: u32 =
    MSTATUS_SIE | MSTATUS_SPIE | MSTATUS_SPP | MSTATUS_SUM | MSTATUS_MXR | MSTATUS_FS_MASK;

// S 级中断位（sie/sip 的 S 部分：SSIE/STIE/SEIE）
pub const SIE_SSIE: u32 = 1 << 1;
pub const SIE_STIE: u32 = 1 << 5;
pub const SIE_SEIE: u32 = 1 << 9;
pub const SIE_S_MASK: u32 = SIE_SSIE | SIE_STIE | SIE_SEIE;
pub const MSTATUS_MPP_SHIFT: u32 = 11;
pub const MSTATUS_MPP_MASK: u32 = 0b11 << MSTATUS_MPP_SHIFT;
pub const MSTATUS_MPP_U: u32 = 0b00 << MSTATUS_MPP_SHIFT;
pub const MSTATUS_MPP_S: u32 = 0b01 << MSTATUS_MPP_SHIFT;
pub const MSTATUS_MPP_M: u32 = 0b11 << MSTATUS_MPP_SHIFT;
/// mstatus 中当前实现接受的软件可写位（其余位保持原值）。
pub const MSTATUS_WRITE_MASK: u32 =
    SSTATUS_MASK | MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP_MASK | MSTATUS_MPRV;

// mie 使能位
pub const MIE_MSIE: u32 = 1 << 3; // Machine Software Interrupt Enable
pub const MIE_MTIE: u32 = 1 << 7; // Machine Timer Interrupt Enable
pub const MIE_MEIE: u32 = 1 << 11; // Machine External Interrupt Enable

// mip pending 位（硬件只读，反映设备 pending 状态）
pub const MIP_MSIP: u32 = 1 << 3;
pub const MIP_MTIP: u32 = 1 << 7;
pub const MIP_MEIP: u32 = 1 << 11;

// S 级 pending 位（sip 视图）
pub const MIP_SSIP: u32 = 1 << 1;
pub const MIP_STIP: u32 = 1 << 5;
pub const MIP_SEIP: u32 = 1 << 9;
pub const MIP_S_MASK: u32 = MIP_SSIP | MIP_STIP | MIP_SEIP;

// counter-enable 位（mcounteren / scounteren）
pub const COUNTEREN_CY: u32 = 1 << 0;
pub const COUNTEREN_TM: u32 = 1 << 1;
pub const COUNTEREN_IR: u32 = 1 << 2;
