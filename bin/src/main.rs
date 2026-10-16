//! rremu：可执行入口（composition root）。
//!
//! 职责：解析命令行参数 → 配置日志 → 加载 ELF → 构建 Machine →
//! 注册 Ctrl+C → 选择前端（CLI/TUI）→ 运行。所有组装只发生在这里。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Context};
use clap::Parser;

use rremu_cli::{FbDisplay, Repl};
use rremu_config::{Config, Overrides};
use rremu_loader::load_elf;
use rremu_machine::Machine;
use rremu_window::Window;

/// minifb 窗口的 FbDisplay 适配（newtype：trait 与类型均在外部，
/// 孤儿规则要求本地包装）。
struct FbWindow(Window);

impl FbDisplay for FbWindow {
    fn update(&mut self, pixels: &[u32]) -> bool {
        self.0.update(pixels)
    }

    fn poll(&mut self) -> bool {
        self.0.poll()
    }
}

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

    /// 禁用 Framebuffer 设备（不映射 fb，也不创建帧缓冲窗口）
    #[arg(long)]
    no_fb: bool,

    /// 程序终止时自动写出机器状态 dump（JSON）到指定文件（默认关闭）
    #[arg(long, value_name = "FILE")]
    dump_on_exit: Option<String>,

    /// 设备树 DTB 文件；加载到 RAM 并以 boot a1 传给 guest（OpenSBI/Linux 需要）
    #[arg(long, value_name = "FILE")]
    dtb: Option<String>,

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

    // 外部参数 → 统一 Config（唯一解析入口）
    let mem_size = parse_size(&args.mem)?;
    let mut cfg = Config::resolve(Overrides {
        ram_base: Some(args.base),
        ram_size: Some(mem_size),
        no_fb: args.no_fb,
        entry: args.entry,
        elf: Some(args.elf.clone()),
        dump_on_exit: args.dump_on_exit.clone(),
    })
    .context("invalid configuration")?;

    // 可选 DTB：放到 RAM 顶部下方 1MB 处，作为 boot a1
    let dtb = match &args.dtb {
        Some(p) => Some(std::fs::read(p).with_context(|| format!("failed to read DTB: {p}"))?),
        None => None,
    };
    if dtb.is_some() {
        let ram_end = args.base.checked_add(mem_size).context("ram range overflow")?;
        cfg.boot.dtb = Some((ram_end - 0x10_0000) & !7);
    }

    let bytes = std::fs::read(&args.elf)
        .with_context(|| format!("failed to read ELF file: {}", args.elf))?;
    let image = load_elf(&bytes)
        .with_context(|| format!("failed to parse ELF: {}", args.elf))?;

    let mut machine = Machine::from_config(&cfg, Some(image)).context("failed to build machine")?;
    if let (Some(bytes), Some(addr)) = (&dtb, cfg.boot.dtb) {
        machine.write_mem(addr, bytes).context("failed to load DTB")?;
    }

    if args.tui {
        // TUI 使用 crossterm raw 模式，Ctrl+C 作为 KeyEvent 处理，不走 ctrlc crate
        return Ok(rremu_tui::run(machine, cfg));
    }

    let pause_flag = Arc::new(AtomicBool::new(false));
    let pf = pause_flag.clone();
    ctrlc::set_handler(move || {
        pf.store(true, Ordering::Relaxed);
    })
    .context("failed to set Ctrl+C handler")?;

    // CLI 模式：窗口懒启动 —— 启动时不开窗，注入窗口工厂；REPL 检测到
    // fb 首次写入（dirty）才创建窗口并推送当前帧。不写 fb 的程序/测试
    // 不会被弹窗打扰；开窗失败（无 X11）→ 丢弃工厂降级无窗模式，仅警告一次。
    // 注：minifb 在 workspace 依赖中禁用 wayland 后端、仅启用 x11，
    // 以获得 X11 装饰条（标题栏/边框，可鼠标拖动）；Wayland 后端因
    // xdg-decoration 协议缺失会导致无边框窗口无法移动。
    // 无 fb（--no-fb）即无窗口工厂
    let window_factory: Option<rremu_cli::WindowFactory> = cfg.layout.fb.map(|fb| {
        let factory: rremu_cli::WindowFactory = Box::new(move || {
            match Window::new(fb.width as usize, fb.height as usize, "rremu framebuffer") {
                Ok(w) => Some(Box::new(FbWindow(w)) as Box<dyn FbDisplay>),
                Err(e) => {
                    eprintln!(
                        "rremu: warning: could not open framebuffer window ({e}); continuing without window"
                    );
                    None
                }
            }
        });
        factory
    });

    let mut repl = Repl::new(machine, cfg, pause_flag, window_factory);
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
