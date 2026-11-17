// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! IDL Code Generator CLI
//!
//! Subcommands:
//! - parse: validate and optionally pretty-print
//! - gen cpp|rust: generate code
//! - check: validate only (CI-friendly)
//! - fmt: reformat IDL using the pretty-printer

use clap::{Args, Parser as ClapParser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[path = "hddsgen/commands.rs"]
mod commands;

use commands::{run_check, run_fmt, run_gen, run_parse};

#[derive(ClapParser, Debug)]
#[command(name = "hddsgen", version = env!("HDDS_VERSION"), about = "IDL 4.2 parser and code generator")]
struct Cli {
    /// Deprecated one-shot flags (kept for backward compatibility)
    #[arg(long, hide = true)]
    output_cpp: bool,

    #[arg(long, hide = true)]
    output_rust: bool,

    /// Output file for legacy flags
    #[arg(long, hide = true)]
    output: Option<PathBuf>,

    /// Namespace for C++ (legacy flag)
    #[arg(long, hide = true)]
    namespace_cpp: Option<String>,

    /// Legacy input when using legacy flags
    input: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Validate IDL and optionally pretty-print
    Parse(ParseCmd),
    /// Generate code in a target language
    Gen(GenCmd),
    /// Validate only (non-zero on error)
    Check(CheckCmd),
    /// Reformat IDL via pretty-printer
    Fmt(FmtCmd),
}

#[derive(Args, Debug)]
struct ParseCmd {
    /// Input file path or '-' for stdin
    input: PathBuf,
    /// Include directories (-I) for resolving #include "..." in IDL
    #[arg(short = 'I', long = "include")]
    include: Vec<PathBuf>,
    /// Print pretty-printed IDL
    #[arg(long)]
    pretty: bool,
    /// Emit JSON diagnostics
    #[arg(long)]
    json: bool,
}

#[derive(Clone, Copy, ValueEnum, Debug)]
enum Lang {
    Cpp,
    Rust,
    Python,
    C,
    /// `no_std` Rust for embedded (hdds-micro)
    Micro,
    /// Header-only C for MCUs (STM32, AVR, PIC, ESP32)
    CMicro,
    /// TypeScript with CDR2 serialization
    #[value(name = "typescript", alias = "ts")]
    TypeScript,
}

#[derive(Clone, Copy, ValueEnum, Debug, Default)]
pub enum BuildSystem {
    /// Cargo.toml (Rust default)
    #[default]
    Cargo,
    /// CMakeLists.txt (C/C++ default)
    Cmake,
    /// Makefile
    Make,
}

/// `--default-extensibility` values.
#[derive(Clone, Copy, ValueEnum, Debug)]
pub enum DefaultExtArg {
    /// FINAL: members in order, no evolution
    Final,
    /// APPENDABLE: members may be appended (the DDS-XTypes 1.3 default)
    Appendable,
    /// MUTABLE: members identified by id
    Mutable,
}

#[derive(Clone, Copy, ValueEnum, Debug, Default)]
pub enum CStandardArg {
    /// C89/C90 (ANSI C): compiles with `-std=c89 -pedantic-errors`; still
    /// includes `<stdint.h>`
    C89,
    /// C99 (default) - mixed declarations allowed
    #[default]
    C99,
    /// C11 - adds _`Static_assert`
    C11,
}

/// Serde rename style for JSON serialization
#[derive(Clone, Copy, ValueEnum, Debug)]
pub enum SerdeRenameStyle {
    /// camelCase (JavaScript/TypeScript)
    Camel,
    /// `PascalCase` (C#/.NET)
    Pascal,
    /// kebab-case
    Kebab,
}

#[derive(Args, Debug)]
// CLI flags are naturally boolean; a state machine would be overkill here
#[allow(clippy::struct_excessive_bools)]
struct GenCmd {
    /// Target language
    #[arg(value_enum)]
    lang: Lang,
    /// Input file path or '-' for stdin
    input: PathBuf,
    /// Include directories (-I) for resolving #include "..." in IDL
    #[arg(short = 'I', long = "include")]
    include: Vec<PathBuf>,
    /// Output file (stdout if omitted)
    #[arg(short, long)]
    out: Option<PathBuf>,
    /// Output directory: `mod.rs` for Rust and micro, else the input's name
    /// with the language's extension (`Shapes.hpp`, `Shapes.py`, `Shapes.h`,
    /// `Shapes.ts`)
    #[arg(long, value_name = "DIR")]
    out_dir: Option<PathBuf>,
    /// Wrap C++ output in namespace (`A::B::C`)
    #[arg(long)]
    namespace_cpp: Option<String>,
    /// Generate example `main()` showing serialization/deserialization
    #[arg(long)]
    example: bool,
    /// Build system to generate (cargo/cmake/make)
    /// Default: cargo for Rust and micro, cmake for C++, make for C and c-micro.
    /// Python and TypeScript generate no build system file.
    #[arg(long, value_enum)]
    build_system: Option<BuildSystem>,
    /// Path to the local crate a generated example should depend on, for the rust and
    /// micro targets with --example. The rust target needs the `hdds` crate, the micro
    /// target needs `hdds-micro`; passing the repository root works for both, the path
    /// is resolved to the crate it contains. Without it, a generated example depends on
    /// the crates.io release this hddsgen targets, which fails while that release is
    /// unpublished.
    #[arg(long, value_name = "PATH")]
    hdds_path: Option<PathBuf>,
    /// C language standard (for the C target; C-micro output is C99)
    /// c89: ANSI C (`-std=c89 -pedantic-errors`), needs `<stdint.h>`
    /// c99: default
    /// c11: adds _`Static_assert`
    #[arg(long, value_enum, default_value = "c99")]
    c_standard: CStandardArg,
    /// Generate FastDDS-compatible C++ with getter/setter methods (for C++ target only)
    /// This allows generated types to be used with both `FastDDS` and HDDS backends
    #[arg(long)]
    fastdds_compat: bool,
    /// Add `serde::Serialize` and `serde::Deserialize` derives to generated Rust types
    #[arg(long)]
    serde: bool,
    /// Rename style for serde serialization (requires --serde)
    /// camel: camelCase (JS/TS), pascal: `PascalCase` (C#), kebab: kebab-case
    #[arg(long, value_enum, requires = "serde")]
    serde_rename: Option<SerdeRenameStyle>,
    /// Generate separate files for each included IDL.
    /// Each .idl file becomes its own output file, with proper #include directives.
    /// Requires --out-dir. Without this flag, all content is inlined into one file.
    #[arg(long, requires = "out_dir")]
    separate: bool,
    /// Extensibility of a struct, union, enum or bitmask without an
    /// extensibility annotation. Default: appendable, as DDS-XTypes 1.3
    /// 7.3.1.2.1.8 specifies (an info line lists those types); `final`
    /// gives the layout of hddsgen 1.x. An enum or a bitmask is
    /// never mutable: `mutable` leaves them appendable
    #[arg(long, value_enum, value_name = "KIND")]
    default_extensibility: Option<DefaultExtArg>,
}

#[derive(Args, Debug)]
struct CheckCmd {
    /// Input file path or '-' for stdin
    input: PathBuf,
    /// Include directories (-I) for resolving #include "..." in IDL
    #[arg(short = 'I', long = "include")]
    include: Vec<PathBuf>,
    /// Emit JSON diagnostics
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct FmtCmd {
    /// Input file path or '-' for stdin
    input: PathBuf,
    /// Include directories (-I) for resolving #include "..." in IDL
    #[arg(short = 'I', long = "include")]
    include: Vec<PathBuf>,
    /// Output file (stdout if omitted)
    #[arg(short, long)]
    out: Option<PathBuf>,
}

fn main() {
    let cli = Cli::parse();

    if cli.command.is_none() && (cli.output_cpp || cli.output_rust) {
        let input = cli
            .input
            .as_ref()
            .unwrap_or_else(|| {
                eprintln!("Missing input file");
                std::process::exit(1)
            })
            .clone();

        let lang = if cli.output_cpp {
            Lang::Cpp
        } else {
            Lang::Rust
        };
        let r#gen = GenCmd {
            lang,
            input,
            include: Vec::new(),
            out: cli.output,
            out_dir: None, // Legacy mode doesn't use --out-dir
            namespace_cpp: cli.namespace_cpp,
            example: false,                // Legacy mode doesn't use --example
            build_system: None,            // Legacy mode doesn't use --build-system
            hdds_path: None,               // Legacy mode doesn't use --hdds-path
            c_standard: CStandardArg::C99, // Legacy mode uses C99
            fastdds_compat: false,         // Legacy mode doesn't use FastDDS compat
            serde: false,                  // Legacy mode doesn't use serde
            serde_rename: None,            // Legacy mode doesn't use serde rename
            separate: false,               // Legacy mode doesn't use --separate
            default_extensibility: None,   // Legacy mode: APPENDABLE, with the info line
        };
        run_gen(&r#gen);
        return;
    }

    match cli.command {
        Some(Commands::Parse(cmd)) => run_parse(&cmd),
        Some(Commands::Gen(cmd)) => run_gen(&cmd),
        Some(Commands::Check(cmd)) => run_check(&cmd),
        Some(Commands::Fmt(cmd)) => run_fmt(&cmd),
        None => {
            eprintln!(
                "Use --help to see available subcommands. For back-compat, you can still use --output-cpp/--output-rust."
            );
            std::process::exit(2);
        }
    }
}
