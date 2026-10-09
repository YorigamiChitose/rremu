//! rremu：可执行入口（composition root，M1.11）。
//!
//! 职责：解析命令行参数 → 配置日志 → 加载 ELF → 构建 Machine →
//! 注册 Ctrl+C → 选择前端（CLI/TUI）→ 运行。所有组装只发生在这里。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Context};
use clap::Parser;

use rremu_cli::Repl;
use rremu_loader::load_elf;
use rremu_machine::MachineBuilder;
use rremu_window::Window;

#[derive(Parser)]
#[command(name = "rremu", version, about = "RISC-V RV32IMC 模拟器与调试器")]
struct Args {
    /// ELF 文件路径
    elf: String,

    /// 启动 TUI 界面
    #[arg(short, long)]
    tui: bool,

    /// 批处理命令（可重复；与 --tui 冲突）
    #[arg(short, long = "cmd", value_name = "CMD")]
    cmds: Vec<String>,

    /// RAM 大小（K/M/G 后缀）
    #[arg(short, long, value_name = "SIZE", default_value = "16M")]
    mem: String,

    /// RAM 基址
    #[arg(long, value_name = "ADDR", default_value = "0x80000000", value_parser = parse_addr)]
    base: u32,

    /// 覆盖 ELF 入口地址
    #[arg(long, value_name = "ADDR", value_parser = parse_addr)]
    entry: Option<u32>,

    /// 禁用 Framebuffer 设备（不映射 fb，CLI 模式不自动开窗）
    #[arg(long)]
    no_fb: bool,

    /// 日志详细度（-v info / -vv debug / -vvv trace）
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() {
    let code = match try_main() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("rremu: {e:#}");
            1
        }
    };
    std::process::exit(code);
}

fn try_main() -> anyhow::Result<i32> {
    let args = Args::parse();

    init_tracing(args.verbose);

    if args.tui && !args.cmds.is_empty() {
        anyhow::bail!("--tui and --cmd are mutually exclusive");
    }

    let bytes = std::fs::read(&args.elf)
        .with_context(|| format!("failed to read ELF file: {}", args.elf))?;
    let image = load_elf(&bytes)
        .with_context(|| format!("failed to parse ELF: {}", args.elf))?;

    let mem_size = parse_size(&args.mem)?;
    let mut builder = MachineBuilder::new()
        .ram(args.base, mem_size)
        .uart(0x1000_0000)
        .clint(0x0200_0000);
    // 默认映射 fb 设备；--no-fb 时跳过（CLI 不会自动开窗，TUI 也无内嵌视图）
    if !args.no_fb {
        builder = builder.framebuffer(0x2000_0000, 640, 480);
    }
    let builder = builder.load(image);
    let entry = args.entry;
    let builder = if let Some(entry) = entry {
        builder.entry(entry)
    } else {
        builder
    };
    let machine = builder.build().context("failed to build machine")?;

    if args.tui {
        // TUI 使用 crossterm raw 模式，Ctrl+C 作为 KeyEvent 处理，不走 ctrlc crate
        return Ok(rremu_tui::run(machine));
    }

    let pause_flag = Arc::new(AtomicBool::new(false));
    let pf = pause_flag.clone();
    ctrlc::set_handler(move || {
        pf.store(true, Ordering::Relaxed);
    })
    .context("failed to set Ctrl+C handler")?;

    // CLI 模式 + fb 已配置 → 自动开 minifb 窗口（PLAN §11 M5）。
    // 开窗失败（无 X11/Wayland）→ 降级为无窗模式继续运行，仅打印警告。
    let window = if !args.no_fb {
        match Window::new(640, 480, "rremu framebuffer") {
            Ok(w) => Some(w),
            Err(e) => {
                eprintln!("rremu: warning: could not open framebuffer window ({e}); continuing without window");
                None
            }
        }
    } else {
        None
    };

    let mut repl = Repl::new(machine, pause_flag, window);
    let code = if args.cmds.is_empty() {
        repl.run_interactive()
    } else {
        repl.run_batch(&args.cmds)
    };
    Ok(code)
}

fn init_tracing(verbose: u8) {
    let level = match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

fn parse_size(s: &str) -> anyhow::Result<u32> {
    let s = s.trim();
    let last = match s.as_bytes().last() {
        Some(&b) => b,
        None => anyhow::bail!("empty size"),
    };
    let (num_part, mult): (&str, u64) = match last {
        b'k' | b'K' => (&s[..s.len() - 1], 1024),
        b'm' | b'M' => (&s[..s.len() - 1], 1024 * 1024),
        b'g' | b'G' => (&s[..s.len() - 1], 1024 * 1024 * 1024),
        _ => (s, 1),
    };
    let n: u64 = num_part.parse().map_err(|_| anyhow!("invalid size: {s}"))?;
    let total = n
        .checked_mul(mult)
        .ok_or_else(|| anyhow!("size too large: {s}"))?;
    u32::try_from(total).map_err(|_| anyhow!("size exceeds 32-bit address space: {s}"))
}

/// clap value_parser：支持 `0x` 前缀十六进制与十进制。
fn parse_addr(s: &str) -> Result<u32, String> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).map_err(|e| format!("invalid hex address: {e}"))
    } else {
        s.parse::<u32>().map_err(|e| format!("invalid address: {e}"))
    }
}
