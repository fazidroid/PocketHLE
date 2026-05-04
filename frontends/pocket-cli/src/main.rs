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
        } => cmd_run(
            &path,
            cpu,
            halt_on_unimplemented,
            max_slices,
            instructions_per_slice,
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

fn cmd_run(
    path: &std::path::Path,
    backend: CpuBackend,
    halt_on_unimplemented: bool,
    max_slices: u64,
    instructions_per_slice: u64,
) -> Result<()> {
    let mut emu = match backend {
        CpuBackend::Stub => Emulator::with_stub_cpu(),
        #[cfg(feature = "unicorn")]
        CpuBackend::Unicorn => Emulator::with_unicorn_cpu()?,
    };
    emu.set_halt_on_unimplemented(halt_on_unimplemented);
    emu.max_slices = max_slices;
    emu.instruction_budget_per_slice = instructions_per_slice;
    let p = emu.load_pe(path)?;
    println!(
        "Loaded {} ({} machine), {} sections, {} imports",
        p.image.source_path,
        p.image.machine_name(),
        p.image.sections.len(),
        p.image.imports.len()
    );
    println!(
        "Registered API stubs: {}",
        emu.dispatcher().registered_count()
    );
    emu.run()?;
    println!("Emulator exited cleanly.");
    Ok(())
}
