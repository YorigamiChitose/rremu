//! trap cause 常量与名称。

// 异常（bit31=0）
pub const INSTR_MISALIGNED: u32 = 0;
pub const INSTR_ACCESS_FAULT: u32 = 1;
pub const ILLEGAL_INSTR: u32 = 2;
pub const BREAKPOINT: u32 = 3;
pub const LOAD_MISALIGNED: u32 = 4;
pub const LOAD_ACCESS_FAULT: u32 = 5;
pub const STORE_MISALIGNED: u32 = 6;
pub const STORE_ACCESS_FAULT: u32 = 7;
pub const ECALL_FROM_U: u32 = 8;
pub const ECALL_FROM_S: u32 = 9;
pub const ECALL_FROM_M: u32 = 11;
pub const INSTR_PAGE_FAULT: u32 = 12;
pub const LOAD_PAGE_FAULT: u32 = 13;
pub const STORE_PAGE_FAULT: u32 = 15;

// 中断（bit31=1）
pub const S_SI: u32 = 0x8000_0001; // Supervisor Software Interrupt
pub const S_TI: u32 = 0x8000_0005; // Supervisor Timer Interrupt
pub const S_EI: u32 = 0x8000_0009; // Supervisor External Interrupt
pub const M_SI: u32 = 0x8000_0003; // Machine Software Interrupt
pub const M_TI: u32 = 0x8000_0007; // Machine Timer Interrupt
pub const M_EI: u32 = 0x8000_000b; // Machine External Interrupt

/// mcause → 人类可读名称（bit31 区分中断/异常）。
pub fn cause_name(mcause: u32) -> &'static str {
    let code = mcause & 0x7fff_ffff;
    if mcause >> 31 == 1 {
        match code {
            1 => "supervisor software interrupt",
            5 => "supervisor timer interrupt",
            9 => "supervisor external interrupt",
            3 => "machine software interrupt",
            7 => "machine timer interrupt",
            11 => "machine external interrupt",
            _ => "unknown interrupt",
        }
    } else {
        match code {
            INSTR_MISALIGNED => "instruction address misaligned",
            INSTR_ACCESS_FAULT => "instruction access fault",
            ILLEGAL_INSTR => "illegal instruction",
            BREAKPOINT => "breakpoint",
            LOAD_MISALIGNED => "load address misaligned",
            LOAD_ACCESS_FAULT => "load access fault",
            STORE_MISALIGNED => "store address misaligned",
            STORE_ACCESS_FAULT => "store access fault",
            ECALL_FROM_U => "environment call from U-mode",
            ECALL_FROM_S => "environment call from S-mode",
            ECALL_FROM_M => "environment call from M-mode",
            INSTR_PAGE_FAULT => "instruction page fault",
            LOAD_PAGE_FAULT => "load page fault",
            STORE_PAGE_FAULT => "store/AMO page fault",
            _ => "unknown exception",
        }
    }
}
