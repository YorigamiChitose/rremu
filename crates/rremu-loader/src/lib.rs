//! rremu-loader：ELF32 内存镜像解析（零 IO，读文件在 bin 层）。

use goblin::elf::{header, program_header, Elf};
use thiserror::Error;

/// 一段可写入内存的镜像片段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub addr: u32,
    pub data: Vec<u8>,
}

/// 解析结果：加载段 + 入口 + tohost 符号地址（riscv-tests 必需）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub segments: Vec<Segment>,
    pub entry: u32,
    pub tohost: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LoaderError {
    #[error("ELF parse failed: {0}")]
    Parse(String),
    #[error("not a RISC-V ELF (machine={machine})")]
    NotRiscv { machine: u16 },
    #[error("not a 32-bit ELF")]
    NotElf32,
    #[error("no PT_LOAD segments")]
    NoSegments,
}

/// 解析 ELF32 小端 RISC-V 镜像。只取 PT_LOAD 段的文件内容；符号表中查找 `tohost`。
pub fn load_elf(bytes: &[u8]) -> Result<Image, LoaderError> {
    let elf = Elf::parse(bytes).map_err(|e| LoaderError::Parse(e.to_string()))?;

    if elf.header.e_machine != header::EM_RISCV {
        return Err(LoaderError::NotRiscv { machine: elf.header.e_machine });
    }
    if elf.is_64 {
        return Err(LoaderError::NotElf32);
    }

    let mut segments = Vec::new();
    for ph in elf.program_headers.iter() {
        if ph.p_type != program_header::PT_LOAD || ph.p_filesz == 0 {
            continue;
        }
        let off = ph.p_offset as usize;
        let filesz = ph.p_filesz as usize;
        let end = off
            .checked_add(filesz)
            .filter(|&e| e <= bytes.len())
            .ok_or_else(|| LoaderError::Parse(format!("PT_LOAD range {}+{} out of file", ph.p_offset, ph.p_filesz)))?;
        segments.push(Segment { addr: ph.p_vaddr as u32, data: bytes[off..end].to_vec() });
    }
    if segments.is_empty() {
        return Err(LoaderError::NoSegments);
    }

    // 符号表提取 tohost（.symtab；riscv-tests 与自研固件均带此符号）
    let tohost = elf
        .syms
        .iter()
        .find(|sym| elf.strtab.get_at(sym.st_name) == Some("tohost"))
        .map(|sym| sym.st_value as u32);

    Ok(Image { segments, entry: elf.entry as u32, tohost })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 手工构造最小 ELF32 小端 RISC-V 镜像：一个 PT_LOAD 段 + 无符号表。
    fn minimal_elf(entry: u32, vaddr: u32, code: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        // ELF header (52 bytes)
        b.extend_from_slice(&[0x7f, b'E', b'L', b'F', 1, 1, 1, 0]); // magic, 32-bit, LE, v1, SYSV
        b.extend_from_slice(&[0u8; 8]); // padding
        b.extend_from_slice(&2u16.to_le_bytes()); // e_type = EXEC
        b.extend_from_slice(&243u16.to_le_bytes()); // e_machine = EM_RISCV
        b.extend_from_slice(&1u32.to_le_bytes()); // e_version
        b.extend_from_slice(&entry.to_le_bytes()); // e_entry
        b.extend_from_slice(&52u32.to_le_bytes()); // e_phoff
        b.extend_from_slice(&0u32.to_le_bytes()); // e_shoff
        b.extend_from_slice(&0u32.to_le_bytes()); // e_flags
        b.extend_from_slice(&52u16.to_le_bytes()); // e_ehsize
        b.extend_from_slice(&32u16.to_le_bytes()); // e_phentsize
        b.extend_from_slice(&1u16.to_le_bytes()); // e_phnum
        b.extend_from_slice(&40u16.to_le_bytes()); // e_shentsize
        b.extend_from_slice(&0u16.to_le_bytes()); // e_shnum
        b.extend_from_slice(&0u16.to_le_bytes()); // e_shstrndx
        // Program header (32 bytes)
        b.extend_from_slice(&1u32.to_le_bytes()); // p_type = PT_LOAD
        b.extend_from_slice(&84u32.to_le_bytes()); // p_offset = 52 + 32
        b.extend_from_slice(&vaddr.to_le_bytes()); // p_vaddr
        b.extend_from_slice(&vaddr.to_le_bytes()); // p_paddr
        b.extend_from_slice(&(code.len() as u32).to_le_bytes()); // p_filesz
        b.extend_from_slice(&(code.len() as u32).to_le_bytes()); // p_memsz
        b.extend_from_slice(&5u32.to_le_bytes()); // p_flags R+X
        b.extend_from_slice(&4u32.to_le_bytes()); // p_align
        // segment data
        b.extend_from_slice(code);
        b
    }

    #[test]
    fn minimal_elf_parses() {
        let code = [0x13u8, 0x00, 0x00, 0x00]; // nop
        let bytes = minimal_elf(0x8000_0000, 0x8000_0000, &code);
        let img = load_elf(&bytes).unwrap();
        assert_eq!(img.entry, 0x8000_0000);
        assert_eq!(img.segments.len(), 1);
        assert_eq!(img.segments[0].addr, 0x8000_0000);
        assert_eq!(img.segments[0].data, code.to_vec());
        assert_eq!(img.tohost, None);
    }

    #[test]
    fn rejects_non_riscv() {
        let mut bytes = minimal_elf(0, 0, &[0; 4]);
        // e_machine 位于 offset 18
        bytes[18..20].copy_from_slice(&0x3eu16.to_le_bytes()); // x86-64
        assert!(matches!(load_elf(&bytes), Err(LoaderError::NotRiscv { machine: 0x3e })));
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(load_elf(b"not an elf at all"), Err(LoaderError::Parse(_))));
    }

    #[test]
    fn hello_elf_parses_with_tohost() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/data/hello.elf");
        let bytes = std::fs::read(path)
            .unwrap_or_else(|e| panic!("缺少 {path}（先运行 tools/build-firmware.sh）：{e}"));
        let img = load_elf(&bytes).unwrap();
        assert_eq!(img.entry, 0x8000_0000);
        assert!(!img.segments.is_empty());
        // tohost 符号在 0x80000080（见 1.7 构建产物）
        assert_eq!(img.tohost, Some(0x8000_0080));
        // 代码段从入口地址开始，包含数据
        let text = img.segments.iter().find(|s| s.addr == 0x8000_0000).expect("text segment");
        assert!(text.data.len() >= 4);
    }
}
