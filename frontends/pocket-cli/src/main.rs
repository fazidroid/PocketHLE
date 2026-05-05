//! Linux command-line frontend for PocketHLE.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use pocket_core::Emulator;

#[derive(Parser, Debug)]
#[command(
    name = "pockethle",
    version,
    about = "High-level Windows Mobile / Pocket PC emulator (CLI frontend)",
    long_about = None
)]
struct Cli {
    /// Logging verbosity (-v info, -vv debug, -vvv trace).
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Print info about a PE32 file (game .exe extracted from a CAB).
    PeInfo { path: PathBuf },
    /// Extract every file from a Windows Mobile `.CAB` into a directory.
    UnpackCab { cab: PathBuf, out_dir: PathBuf },
    /// Extract a CAB and print info about the largest PE inside.
    InspectCab {
        cab: PathBuf,
        /// Optional output directory (defaults to a temp dir under
        /// `$XDG_CACHE_HOME/pockethle`).
        #[arg(short, long)]
        out_dir: Option<PathBuf>,
    },
    /// Run a PE file in the emulator. Without `--cpu unicorn` this
    /// uses the trace-only stub CPU and only logs API requests.
    Run {
        path: PathBuf,
        /// CPU backend.
        #[arg(long, default_value = "stub")]
        cpu: CpuBackend,
        /// Halt as soon as the guest calls an unimplemented API.
        #[arg(long, default_value_t = false)]
        halt_on_unimplemented: bool,
        /// Maximum number of host-resumed slices (each slice can
        /// run up to `--instructions-per-slice` instructions).
        #[arg(long, default_value_t = 1024)]
        max_slices: u64,
        #[arg(long, default_value_t = 1_000_000)]
        instructions_per_slice: u64,
        /// Write a JSON-lines trace of every dispatched API call to
        /// the given file. Useful for diffing runs and for offline
        /// analysis (`jq`, etc.).
        #[arg(long)]
        trace_json: Option<PathBuf>,
        /// Mount a host directory as the WinCE `\Application\` root.
        /// `CreateFileW` requests inside that prefix are translated
        /// to host paths.
        #[arg(long)]
        rom_dir: Option<PathBuf>,
        /// Mount the host directory at a custom guest prefix instead
        /// of `\Application\` (e.g. `--rom-prefix \\Storage\\`).
        #[arg(long, default_value = "\\Application\\")]
        rom_prefix: String,
        /// Open a host window and render the framebuffer live.
        /// Requires the `display` cargo feature.
        #[arg(long, default_value_t = false)]
        display: bool,
        /// Periodically write the framebuffer as PPM files into the
        /// given directory (one file per emit). Works in any
        /// environment, no extra dependencies.
        #[arg(long)]
        dump_frames_to: Option<PathBuf>,
        /// Stop emulation after this many distinct rendered frames.
        /// Combined with `--dump-frames-to`, gives a deterministic
        /// way to capture proof-of-rendering screenshots.
        #[arg(long, default_value_t = 0)]
        max_frames: u64,
    },
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum CpuBackend {
    Stub,
    #[cfg(feature = "unicorn")]
    Unicorn,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let level = match cli.verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level)).init();

    match cli.command {
        Command::PeInfo { path } => cmd_pe_info(&path),
        Command::UnpackCab { cab, out_dir } => cmd_unpack_cab(&cab, &out_dir),
        Command::InspectCab { cab, out_dir } => cmd_inspect_cab(&cab, out_dir.as_deref()),
        Command::Run {
            path,
            cpu,
            halt_on_unimplemented,
            max_slices,
            instructions_per_slice,
            trace_json,
            rom_dir,
            rom_prefix,
            display,
            dump_frames_to,
            max_frames,
        } => cmd_run(
            &path,
            cpu,
            halt_on_unimplemented,
            max_slices,
            instructions_per_slice,
            trace_json.as_deref(),
            rom_dir.as_deref(),
            &rom_prefix,
            display,
            dump_frames_to.as_deref(),
            max_frames,
        ),
    }
}

fn cmd_pe_info(path: &std::path::Path) -> Result<()> {
    let img = pocket_core::pe::load_file(path).context("loading PE")?;
    println!("Source: {}", img.source_path);
    println!(
        "Machine: 0x{:04x} ({})  Subsystem: {}",
        img.machine,
        img.machine_name(),
        img.subsystem
    );
    println!(
        "ImageBase: 0x{:08x}   SizeOfImage: 0x{:x}   EntryPoint: 0x{:08x}",
        img.image_base,
        img.size_of_image,
        img.entry_va()
    );
    println!("Sections:");
    for s in &img.sections {
        println!(
            "  {:>8}  va=0x{:08x}  size=0x{:06x}  flags=0x{:08x}{}{}{}",
            s.name,
            img.image_base + s.virtual_address,
            s.virtual_size,
            s.characteristics,
            if s.is_readable() { " R" } else { "" },
            if s.is_writable() { " W" } else { "" },
            if s.is_executable() { " X" } else { "" },
        );
    }
    println!("Imports:");
    for (dll, syms) in pocket_core::pe::imports_by_dll(&img) {
        println!("  {} ({} symbols)", dll, syms.len());
        for s in syms {
            let mut display = s.binding.to_string_short();
            if let pocket_core::pe::ImportBinding::Ordinal(o) = &s.binding {
                if let Some(name) = pocket_core::winceapi::resolve_ordinal(&dll, *o) {
                    display = format!("{name} (ord {o})");
                }
            }
            println!("    iat=0x{:08x}  {}", s.iat_va, display);
        }
    }
    Ok(())
}

fn cmd_unpack_cab(cab: &std::path::Path, out_dir: &std::path::Path) -> Result<()> {
    let entries = pocket_core::cab::extract_all(cab, out_dir).context("extracting cab")?;
    println!("Extracted {} files to {}", entries.len(), out_dir.display());
    for e in entries {
        println!(
            "  {:>14}  {:>8} bytes  {}",
            e.short_name,
            e.size,
            e.extracted_path.display()
        );
    }
    Ok(())
}

fn cmd_inspect_cab(cab: &std::path::Path, out_dir: Option<&std::path::Path>) -> Result<()> {
    let dest = match out_dir {
        Some(p) => p.to_path_buf(),
        None => {
            let base = std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    let home = std::env::var_os("HOME").unwrap_or_default();
                    PathBuf::from(home).join(".cache")
                });
            base.join("pockethle").join("cab-extracted")
        }
    };
    std::fs::create_dir_all(&dest)?;
    let (files, header) = pocket_core::cab::extract_with_header(cab, &dest)?;
    if let Some(h) = &header {
        println!(
            "Install header: provider={:?}, app_name={:?}",
            h.provider, h.app_name
        );
    }
    println!("Files ({}):", files.len());
    let mut largest: Option<&pocket_core::cab::CabFile> = None;
    for f in &files {
        println!("  {:>14}  {:>8} bytes", f.short_name, f.size);
        if largest.map(|l| l.size).unwrap_or(0) < f.size {
            largest = Some(f);
        }
    }
    if let Some(big) = largest {
        println!(
            "\nLargest file is {}, treating as the game executable.",
            big.short_name
        );
        cmd_pe_info(&big.extracted_path)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_run(
    path: &std::path::Path,
    backend: CpuBackend,
    halt_on_unimplemented: bool,
    max_slices: u64,
    instructions_per_slice: u64,
    trace_json: Option<&std::path::Path>,
    rom_dir: Option<&std::path::Path>,
    rom_prefix: &str,
    display: bool,
    dump_frames_to: Option<&std::path::Path>,
    max_frames: u64,
) -> Result<()> {
    let mut emu = match backend {
        CpuBackend::Stub => Emulator::with_stub_cpu(),
        #[cfg(feature = "unicorn")]
        CpuBackend::Unicorn => Emulator::with_unicorn_cpu()?,
    };
    emu.set_halt_on_unimplemented(halt_on_unimplemented);
    emu.max_slices = max_slices;
    emu.instruction_budget_per_slice = instructions_per_slice;
    if let Some(p) = trace_json {
        let f = std::fs::File::create(p)
            .with_context(|| format!("creating trace file {}", p.display()))?;
        emu.set_trace_sink(Box::new(std::io::BufWriter::new(f)));
        println!("Tracing API calls to {} (JSON lines)", p.display());
    }
    let summary = {
        let p = emu.load_pe(path)?;
        format!(
            "Loaded {} ({} machine), {} sections, {} imports",
            p.image.source_path,
            p.image.machine_name(),
            p.image.sections.len(),
            p.image.imports.len()
        )
    };
    println!("{summary}");
    if let Some(dir) = rom_dir {
        emu.mount_dir(rom_prefix, dir);
        println!(
            "Mounted host directory {} at guest prefix {:?}",
            dir.display(),
            rom_prefix
        );
    }
    println!(
        "Registered API stubs: {}",
        emu.dispatcher().registered_count()
    );

    let mut hooks: Vec<Box<dyn pocket_core::kernel::FrameHook>> = Vec::new();
    if let Some(dir) = dump_frames_to {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating frame dump dir {}", dir.display()))?;
        let dir = dir.to_path_buf();
        hooks.push(Box::new(DumpFrameHook::new(dir, max_frames)));
        println!(
            "Dumping framebuffer snapshots to {}",
            dump_frames_to.unwrap().display()
        );
    }
    if display {
        #[cfg(feature = "display")]
        {
            hooks.push(Box::new(display_window::DisplayHook::new()?));
            println!("Display window opened (close it to exit emulator).");
        }
        #[cfg(not(feature = "display"))]
        {
            anyhow::bail!(
                "--display requires building pockethle with `--features display` (minifb)."
            );
        }
    }

    if hooks.is_empty() {
        emu.run()?;
    } else {
        let mut combined = MultiHook { hooks };
        emu.run_with_hook(&mut combined)?;
    }
    println!("Emulator exited cleanly.");
    Ok(())
}

// ----- frame hooks -----

struct MultiHook {
    hooks: Vec<Box<dyn pocket_core::kernel::FrameHook>>,
}

impl pocket_core::kernel::FrameHook for MultiHook {
    fn on_frame(
        &mut self,
        state: &pocket_core::kernel::KernelState,
    ) -> pocket_core::kernel::FrameAction {
        let mut action = pocket_core::kernel::FrameAction::Continue;
        for h in self.hooks.iter_mut() {
            if h.on_frame(state) == pocket_core::kernel::FrameAction::Stop {
                action = pocket_core::kernel::FrameAction::Stop;
            }
        }
        action
    }
}

struct DumpFrameHook {
    dir: PathBuf,
    last_dumped_frame: u64,
    written: u64,
    max_frames: u64,
}

impl DumpFrameHook {
    fn new(dir: PathBuf, max_frames: u64) -> Self {
        Self {
            dir,
            last_dumped_frame: 0,
            written: 0,
            max_frames,
        }
    }
}

impl pocket_core::kernel::FrameHook for DumpFrameHook {
    fn on_frame(
        &mut self,
        state: &pocket_core::kernel::KernelState,
    ) -> pocket_core::kernel::FrameAction {
        let counter = state.framebuffer.frame_counter;
        if counter == self.last_dumped_frame {
            return pocket_core::kernel::FrameAction::Continue;
        }
        self.last_dumped_frame = counter;
        let path = self.dir.join(format!("frame_{:06}.ppm", self.written));
        let ppm = state.framebuffer.snapshot_ppm();
        if let Err(e) = std::fs::write(&path, ppm) {
            log::warn!("failed to write {}: {e}", path.display());
            return pocket_core::kernel::FrameAction::Continue;
        }
        log::info!("wrote {}", path.display());
        self.written += 1;
        if self.max_frames > 0 && self.written >= self.max_frames {
            return pocket_core::kernel::FrameAction::Stop;
        }
        pocket_core::kernel::FrameAction::Continue
    }
}

#[cfg(feature = "display")]
mod display_window {
    use anyhow::{Context, Result};
    use minifb::{Window, WindowOptions};
    use pocket_core::kernel::{FrameAction, FrameHook, KernelState};

    pub struct DisplayHook {
        window: Window,
        buffer: Vec<u32>,
        last_frame: u64,
    }

    impl DisplayHook {
        pub fn new() -> Result<Self> {
            let w = pocket_core::kernel::FB_WIDTH as usize;
            let h = pocket_core::kernel::FB_HEIGHT as usize;
            let mut window = Window::new(
                "PocketHLE",
                w,
                h,
                WindowOptions {
                    resize: true,
                    scale: minifb::Scale::X2,
                    ..WindowOptions::default()
                },
            )
            .context("opening minifb window")?;
            window.set_target_fps(60);
            Ok(Self {
                window,
                buffer: vec![0; w * h],
                last_frame: 0,
            })
        }
    }

    impl FrameHook for DisplayHook {
        fn on_frame(&mut self, state: &KernelState) -> FrameAction {
            if state.framebuffer.frame_counter != self.last_frame {
                self.last_frame = state.framebuffer.frame_counter;
                let rgba = state.framebuffer.snapshot_rgba8888();
                for (i, px) in self.buffer.iter_mut().enumerate() {
                    let off = i * 4;
                    let r = rgba[off] as u32;
                    let g = rgba[off + 1] as u32;
                    let b = rgba[off + 2] as u32;
                    *px = (r << 16) | (g << 8) | b;
                }
            }
            let w = pocket_core::kernel::FB_WIDTH as usize;
            let h = pocket_core::kernel::FB_HEIGHT as usize;
            if self.window.is_open() {
                let _ = self.window.update_with_buffer(&self.buffer, w, h);
                FrameAction::Continue
            } else {
                FrameAction::Stop
            }
        }
    }
}
