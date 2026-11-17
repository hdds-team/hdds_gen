// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! CLI command implementations.
//!
//! Handlers for parse, gen, check, and fmt subcommands.

use hddsgen::preprocessor::{preprocess_no_inline, read_and_preprocess};

use super::{
    BuildSystem, CStandardArg, CheckCmd, DefaultExtArg, FmtCmd, GenCmd, Lang, ParseCmd,
    SerdeRenameStyle,
};
use hddsgen::{
    codegen::examples_project,
    codegen::{
        c::CGenerator,
        cpp::CppGenerator,
        rust_backend::{RustGenerator, SerdeRename},
        Backend, CStandard,
    },
    idl_pretty, validate, Parser,
};
use serde::Serialize;
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize)]
/// JSON diagnostic payload returned when `--json` is used.
pub struct Diag {
    pub ok: bool,
    pub message: Option<String>,
}

/// Execute the `parse` subcommand.
pub fn run_parse(cmd: &ParseCmd) {
    let result = match read_and_preprocess(&cmd.input, &cmd.include) {
        Ok(r) => r,
        Err(e) => exit_err(cmd.json, format!("Error reading input: {e}")),
    };
    let mut parser = match Parser::try_new(&result.content) {
        Ok(p) => p,
        Err(e) => exit_err(cmd.json, format!("Lexer error: {e}")),
    };
    match parser.parse() {
        Ok(ast) => {
            if cmd.json {
                println!(
                    "{}",
                    diag_to_json(&Diag {
                        ok: true,
                        message: None
                    })
                );
            } else if cmd.pretty {
                let formatted = idl_pretty(&ast);
                println!("{formatted}");
            } else {
                eprintln!("OK");
            }
        }
        Err(e) => exit_err(cmd.json, format!("Parse error: {e}")),
    }
}

/// Applies `--default-extensibility` to the structures, unions,
/// enumerations and bitmasks of `ast` that carry no extensibility
/// annotation. Without the option they are APPENDABLE, the DDS-XTypes 1.3
/// default (7.3.1.2.1.8), and, when `info` is set, a note naming them and
/// the option that restores the FINAL layout of hddsgen 1.x goes to stderr.
fn apply_default_extensibility(
    cmd: &GenCmd,
    ast: &mut hddsgen::IdlFile,
    origin: &std::path::Path,
    info: bool,
) {
    use hddsgen::types::ExtensibilityKind;
    let kind = match cmd.default_extensibility {
        Some(DefaultExtArg::Final) => ExtensibilityKind::Final,
        None | Some(DefaultExtArg::Appendable) => ExtensibilityKind::Appendable,
        Some(DefaultExtArg::Mutable) => ExtensibilityKind::Mutable,
    };
    let names = hddsgen::codegen::data_model::apply_default_extensibility(ast, kind);
    if info && cmd.default_extensibility.is_none() && !names.is_empty() {
        let shown: Vec<&str> = names.iter().take(5).map(String::as_str).collect();
        let more = if names.len() > shown.len() {
            ", ..."
        } else {
            ""
        };
        eprintln!(
            "info: {}: {} type(s) without an extensibility annotation ({}{more}) \
             generated as APPENDABLE, the DDS-XTypes 1.3 default (7.3.1.2.1.8). \
             hddsgen 1.x generated them as FINAL: pass \
             --default-extensibility final for that layout, or annotate them.",
            origin.display(),
            names.len(),
            shown.join(", ")
        );
    }
}

/// Execute the `gen` subcommand.
#[allow(clippy::too_many_lines)]
pub fn run_gen(cmd: &GenCmd) {
    // Handle --separate mode: generate each included file separately
    if cmd.separate {
        run_gen_separate(cmd);
        return;
    }

    let preprocess_result = match read_and_preprocess(&cmd.input, &cmd.include) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error reading input: {e}");
            std::process::exit(1);
        }
    };
    let mut parser = Parser::try_new(&preprocess_result.content).unwrap_or_else(|e| {
        eprintln!("Lexer error: {e}");
        std::process::exit(1);
    });
    let mut ast = match parser.parse() {
        Ok(ast) => ast,
        Err(e) => {
            eprintln!("Parse error: {e}");
            std::process::exit(1);
        }
    };
    apply_default_extensibility(cmd, &mut ast, &cmd.input, true);
    refuse_invalid(&ast);

    // Convert CLI C standard to library type
    let c_standard = match cmd.c_standard {
        CStandardArg::C89 => CStandard::C89,
        CStandardArg::C99 => CStandard::C99,
        CStandardArg::C11 => CStandard::C11,
    };

    // For C/CMicro, use custom generator with standard; otherwise use backend default
    let generator: Box<dyn hddsgen::codegen::CodeGenerator> = match cmd.lang {
        Lang::C => Box::new(CGenerator::with_standard(c_standard)),
        Lang::CMicro => {
            // CMicro uses its own fixed C standard for no_std targets
            Box::new(hddsgen::codegen::c_micro::CMicroGenerator::new())
        }
        Lang::Cpp => {
            let generator = if cmd.fastdds_compat {
                CppGenerator::with_fastdds_compat()
            } else {
                CppGenerator::new()
            };
            Box::new(match &cmd.namespace_cpp {
                Some(ns) => generator.with_namespace(ns),
                None => generator,
            })
        }
        Lang::Rust => {
            if cmd.serde {
                let rename = match cmd.serde_rename {
                    Some(SerdeRenameStyle::Camel) => SerdeRename::Camel,
                    Some(SerdeRenameStyle::Pascal) => SerdeRename::Pascal,
                    Some(SerdeRenameStyle::Kebab) => SerdeRename::Kebab,
                    None => SerdeRename::None,
                };
                Box::new(RustGenerator::with_serde(rename))
            } else {
                Backend::Rust.generator()
            }
        }
        Lang::Python => Backend::Python.generator(),
        Lang::Micro => Backend::Micro.generator(),
        Lang::TypeScript => Backend::TypeScript.generator(),
    };

    let mut output = match generator.generate(&ast) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Code generation error: {e}");
            std::process::exit(1);
        }
    };

    // For C++, insert #include directives for included IDL files
    if matches!(cmd.lang, Lang::Cpp) && !preprocess_result.includes.is_empty() {
        output = insert_cpp_includes(&output, &preprocess_result.includes);
    }

    // --example: Generate full project structure
    if cmd.example {
        let out_dir = cmd.out_dir.clone().unwrap_or_else(|| {
            // Default to current directory if no --out-dir specified
            std::path::PathBuf::from(".")
        });

        let idl_path = cmd.input.to_string_lossy().to_string();

        // Determine build system: use specified or default for language
        let build_system = cmd.build_system.unwrap_or(match cmd.lang {
            Lang::Rust | Lang::Micro => BuildSystem::Cargo,
            Lang::Cpp => BuildSystem::Cmake, // CMake is default for C++
            // Make is default for C/C-micro and script languages (Python/TypeScript don't really use a build system)
            Lang::C | Lang::CMicro | Lang::Python | Lang::TypeScript => BuildSystem::Make,
        });

        // Convert CLI BuildSystem to library BuildSystem
        let lib_build_system = match build_system {
            BuildSystem::Cargo => examples_project::BuildSystem::Cargo,
            BuildSystem::Cmake => examples_project::BuildSystem::Cmake,
            BuildSystem::Make => examples_project::BuildSystem::Make,
        };

        let project = match cmd.lang {
            Lang::Cpp => {
                examples_project::generate_cpp_project(&ast, &output, &idl_path, lib_build_system)
            }
            Lang::Rust => examples_project::generate_rust_project(
                &ast,
                &output,
                &idl_path,
                cmd.hdds_path.as_deref(),
            ),
            Lang::C => {
                examples_project::generate_c_project(&ast, &output, &idl_path, lib_build_system)
            }
            Lang::Python => examples_project::generate_python_project(&ast, &output, &idl_path),
            Lang::Micro => examples_project::generate_micro_project(
                &ast,
                &output,
                &idl_path,
                cmd.hdds_path.as_deref(),
            ),
            Lang::CMicro => examples_project::generate_c_micro_project(&ast, &output, &idl_path),
            Lang::TypeScript => {
                // TypeScript example project (generates types.ts only for now)
                eprintln!(
                    "Note: TypeScript --example generates types.ts only (no project scaffolding yet)"
                );
                let mut project = examples_project::GeneratedProject::new();
                project.add_file("types.ts", &output);
                project
            }
        };

        match project.write_to(&out_dir) {
            Ok(created) => {
                for f in &created {
                    eprintln!("Generated {f}");
                }
            }
            Err(e) => {
                eprintln!("Error writing project: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    // Handle output destination (non-example mode)
    match (&cmd.out, &cmd.out_dir) {
        (Some(_), Some(_)) => {
            eprintln!("Options --out and --out-dir are mutually exclusive");
            std::process::exit(2);
        }
        (Some(out), None) => {
            if out.is_dir() {
                eprintln!(
                    "Error: '{}' is a directory. -o expects a file path, e.g.: -o {}/MyType.hpp",
                    out.display(),
                    out.display()
                );
                eprintln!("Hint: use --out-dir for directory output.");
                std::process::exit(2);
            }
            if let Err(e) = fs::write(out, &output) {
                eprintln!("Error writing '{}': {e}", out.display());
                std::process::exit(1);
            }
            eprintln!("Generated: {}", out.display());
        }
        (None, Some(dir)) => {
            if let Err(e) = fs::create_dir_all(dir) {
                eprintln!("Error creating directory '{}': {e}", dir.display());
                std::process::exit(1);
            }
            let mod_path = dir.join(out_dir_file_name(cmd.lang, &cmd.input));
            if let Err(e) = fs::write(&mod_path, &output) {
                eprintln!("Error writing '{}': {e}", mod_path.display());
                std::process::exit(1);
            }
            eprintln!("Generated: {}", mod_path.display());
        }
        (None, None) => {
            println!("{output}");
        }
    }
}

/// Execute the `check` subcommand.
pub fn run_check(cmd: &CheckCmd) {
    let result = match read_and_preprocess(&cmd.input, &cmd.include) {
        Ok(r) => r,
        Err(e) => exit_err(cmd.json, format!("Error reading input: {e}")),
    };
    let mut parser = match Parser::try_new(&result.content) {
        Ok(p) => p,
        Err(e) => exit_err(cmd.json, format!("Lexer error: {e}")),
    };
    match parser.parse() {
        Ok(ast) => {
            let diags = validate(&ast);
            if diags.is_empty() {
                if cmd.json {
                    println!(
                        "{}",
                        diag_to_json(&Diag {
                            ok: true,
                            message: None
                        })
                    );
                } else {
                    eprintln!("OK");
                }
            } else {
                let first = diags[0].message.clone();
                let more = diags.len().saturating_sub(1);
                let msg = if more > 0 {
                    format!("{first} (+{more} more)")
                } else {
                    first
                };
                exit_err(cmd.json, msg);
            }
        }
        Err(e) => exit_err(cmd.json, format!("Parse error: {e}")),
    }
}

/// Execute the `fmt` subcommand.
pub fn run_fmt(cmd: &FmtCmd) {
    let result = match read_and_preprocess(&cmd.input, &cmd.include) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error reading input: {e}");
            std::process::exit(1);
        }
    };
    let mut parser = Parser::try_new(&result.content).unwrap_or_else(|e| {
        eprintln!("Lexer error: {e}");
        std::process::exit(1);
    });
    let ast = match parser.parse() {
        Ok(ast) => ast,
        Err(e) => {
            eprintln!("Parse error: {e}");
            std::process::exit(1);
        }
    };
    let formatted = idl_pretty(&ast);
    if let Some(out) = &cmd.out {
        if out.is_dir() {
            eprintln!(
                "Error: '{}' is a directory. -o expects a file path.",
                out.display()
            );
            std::process::exit(2);
        }
        if let Err(e) = fs::write(out, formatted) {
            eprintln!("Error writing '{}': {e}", out.display());
            std::process::exit(1);
        }
        eprintln!("Formatted written to: {}", out.display());
    } else {
        println!("{formatted}");
    }
}

fn exit_err(json: bool, msg: impl Into<String>) -> ! {
    let msg = msg.into();
    if json {
        println!(
            "{}",
            diag_to_json(&Diag {
                ok: false,
                message: Some(msg),
            })
        );
        std::process::exit(1);
    }
    eprintln!("{msg}");
    std::process::exit(1);
}

/// Convert an IDL include path to a C++ header path.
/// Examples: "Foo.idl" -> "Foo.hpp", "types/Bar.idl" -> "types/Bar.hpp"
fn idl_to_hpp(idl_path: &std::path::Path) -> String {
    let mut hpp = idl_path.to_string_lossy().to_string();
    if idl_path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("idl"))
    {
        hpp.truncate(hpp.len() - 4);
    }
    hpp.push_str(".hpp");
    hpp
}

/// Insert C++ #include directives for included IDL files.
/// Inserts after the last standard library #include.
fn insert_cpp_includes(code: &str, includes: &[std::path::PathBuf]) -> String {
    if includes.is_empty() {
        return code.to_string();
    }

    // Build the include block (use filename only, not full path)
    let mut include_block = String::new();
    include_block.push_str("// Included IDL dependencies\n");
    for inc in includes {
        // Use only the filename, not the full path
        let filename = inc.file_name().unwrap_or(inc.as_os_str());
        let hpp_path = idl_to_hpp(std::path::Path::new(filename));
        let _ = writeln!(&mut include_block, "#include \"{hpp_path}\"");
    }
    include_block.push('\n');

    // Find position after last standard library #include
    let mut insert_pos = 0usize;
    let mut offset = 0usize;
    for line in code.lines() {
        let line_len = line.len() + 1;
        let trimmed = line.trim_start();
        // Match standard library includes (#include <...>)
        if trimmed.starts_with("#include <") {
            insert_pos = offset + line_len;
        }
        offset += line_len;
    }

    // If no standard includes found, insert after #pragma once
    if insert_pos == 0 {
        if let Some(pragma_pos) = code.find("#pragma once") {
            if let Some(newline) = code[pragma_pos..].find('\n') {
                insert_pos = pragma_pos + newline + 1;
            }
        }
    }

    // Insert the include block
    if insert_pos > 0 && insert_pos < code.len() {
        format!(
            "{head}\n{includes}{tail}",
            head = &code[..insert_pos],
            includes = include_block,
            tail = &code[insert_pos..]
        )
    } else {
        format!("{include_block}{code}")
    }
}

/// Execute `gen` in separate mode: generate each included IDL as its own file.
/// Each .idl becomes a separate .hpp/.rs.
fn run_gen_separate(cmd: &GenCmd) {
    let Some(out_dir) = cmd.out_dir.as_ref() else {
        eprintln!("--separate requires --out-dir");
        std::process::exit(1);
    };
    // Validated once as a whole (each file alone names types its includes
    // declare).
    match read_and_preprocess(&cmd.input, &cmd.include)
        .map_err(|e| e.to_string())
        .and_then(|r| {
            Parser::try_new(&r.content)
                .and_then(|mut p| p.parse())
                .map_err(|e| e.to_string())
        }) {
        Ok(ast) => refuse_invalid(&ast),
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
    fs::create_dir_all(out_dir).unwrap_or_else(|e| {
        eprintln!("Failed to create output directory: {e}");
        std::process::exit(1);
    });

    let mut generated = HashSet::new();
    generate_file_recursive(cmd, &cmd.input, out_dir, &mut generated);
}

/// Recursively generate output for an IDL file and its dependencies.
// codegen function - line count from template output
#[allow(clippy::too_many_lines)]
fn generate_file_recursive(
    cmd: &GenCmd,
    idl_path: &std::path::Path,
    out_dir: &std::path::Path,
    generated: &mut HashSet<PathBuf>,
) {
    // Canonicalize to avoid duplicates
    let canonical = fs::canonicalize(idl_path).unwrap_or_else(|_| idl_path.to_path_buf());
    if generated.contains(&canonical) {
        return;
    }
    generated.insert(canonical);

    // Preprocess without inlining
    let preprocess_result = match preprocess_no_inline(idl_path, &cmd.include) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error reading {}: {e}", idl_path.display());
            std::process::exit(1);
        }
    };

    // First, recursively generate all dependencies
    for dep in &preprocess_result.includes {
        generate_file_recursive(cmd, dep, out_dir, generated);
    }

    // Parse just this file's content
    let mut parser = Parser::try_new(&preprocess_result.content).unwrap_or_else(|e| {
        eprintln!("Lexer error in {}: {e}", idl_path.display());
        std::process::exit(1);
    });
    let mut ast = match parser.parse() {
        Ok(ast) => ast,
        Err(e) => {
            eprintln!("Parse error in {}: {e}", idl_path.display());
            std::process::exit(1);
        }
    };
    apply_default_extensibility(cmd, &mut ast, idl_path, true);

    // Skip if no definitions (e.g., file with only typedefs that resolved to nothing)
    if ast.definitions.is_empty() {
        return;
    }

    // Create generator
    let c_standard = match cmd.c_standard {
        CStandardArg::C89 => CStandard::C89,
        CStandardArg::C99 => CStandard::C99,
        CStandardArg::C11 => CStandard::C11,
    };

    // Parse dependency files for cross-module type resolution (C++ codec needs this)
    let dep_asts: Vec<hddsgen::IdlFile> = preprocess_result
        .includes
        .iter()
        .filter_map(|dep_path| {
            let dep_pp = preprocess_no_inline(dep_path, &cmd.include).ok()?;
            let mut dep_parser = Parser::try_new(&dep_pp.content).ok()?;
            let mut dep_ast = dep_parser.parse().ok()?;
            // Its own generation already warned.
            apply_default_extensibility(cmd, &mut dep_ast, dep_path, false);
            Some(dep_ast)
        })
        .collect();

    let generator: Box<dyn hddsgen::codegen::CodeGenerator> = match cmd.lang {
        Lang::C => Box::new(CGenerator::with_standard(c_standard)),
        Lang::CMicro => Box::new(hddsgen::codegen::c_micro::CMicroGenerator::new()),
        Lang::Cpp => {
            let mut cpp_gen = if cmd.fastdds_compat {
                CppGenerator::with_fastdds_compat()
            } else {
                CppGenerator::new()
            };
            if !dep_asts.is_empty() {
                cpp_gen.set_deps(dep_asts);
            }
            if let Some(ns) = &cmd.namespace_cpp {
                cpp_gen = cpp_gen.with_namespace(ns);
            }
            Box::new(cpp_gen)
        }
        Lang::Rust => {
            if cmd.serde {
                let rename = match cmd.serde_rename {
                    Some(SerdeRenameStyle::Camel) => {
                        hddsgen::codegen::rust_backend::SerdeRename::Camel
                    }
                    Some(SerdeRenameStyle::Pascal) => {
                        hddsgen::codegen::rust_backend::SerdeRename::Pascal
                    }
                    Some(SerdeRenameStyle::Kebab) => {
                        hddsgen::codegen::rust_backend::SerdeRename::Kebab
                    }
                    None => hddsgen::codegen::rust_backend::SerdeRename::None,
                };
                Box::new(hddsgen::codegen::rust_backend::RustGenerator::with_serde(
                    rename,
                ))
            } else {
                Backend::Rust.generator()
            }
        }
        Lang::Python => Backend::Python.generator(),
        Lang::Micro => Backend::Micro.generator(),
        Lang::TypeScript => Backend::TypeScript.generator(),
    };

    // Generate code
    let mut output = match generator.generate(&ast) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Code generation error for {}: {e}", idl_path.display());
            std::process::exit(1);
        }
    };

    // Insert #include directives for dependencies (C++ only for now)
    if matches!(cmd.lang, Lang::Cpp) && !preprocess_result.includes.is_empty() {
        output = insert_cpp_includes(&output, &preprocess_result.includes);
    }

    // Determine output filename
    let stem = idl_path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = out_extension(cmd.lang);
    let out_file = out_dir.join(format!("{stem}.{ext}"));

    // Write output
    fs::write(&out_file, &output).unwrap_or_else(|e| {
        eprintln!("Failed to write {}: {e}", out_file.display());
        std::process::exit(1);
    });

    eprintln!("Generated: {}", out_file.display());
}

fn diag_to_json(diag: &Diag) -> String {
    serde_json::to_string(diag).unwrap_or_else(|e| {
        eprintln!("Failed to encode diagnostic as JSON: {e}");
        std::process::exit(1);
    })
}

/// The file extension of a generated file in `lang`.
const fn out_extension(lang: Lang) -> &'static str {
    match lang {
        Lang::Cpp => "hpp",
        Lang::C | Lang::CMicro => "h",
        Lang::Rust | Lang::Micro => "rs",
        Lang::Python => "py",
        Lang::TypeScript => "ts",
    }
}

/// The file `--out-dir` writes in single-file mode: `mod.rs` for the Rust
/// backends (the directory is the module), else the input's stem with the
/// language's extension, as `--separate` names each file (`Shapes.hpp`,
/// `Shapes.py`); `generated.<ext>` for standard input.
fn out_dir_file_name(lang: Lang, input: &std::path::Path) -> String {
    if matches!(lang, Lang::Rust | Lang::Micro) {
        return "mod.rs".to_string();
    }
    let stem = if input == std::path::Path::new("-") {
        "generated".into()
    } else {
        input.file_stem().unwrap_or_default().to_string_lossy()
    };
    format!("{stem}.{}", out_extension(lang))
}

/// Run the validator before generating: every diagnostic is printed, and
/// an error (an unresolved name, a cyclic typedef, a member declared twice)
/// stops the generation, which would otherwise emit code that does not
/// compile or means something else. Warnings do not stop it.
fn refuse_invalid(ast: &hddsgen::IdlFile) {
    let diags = validate(ast);
    let mut errors = 0usize;
    for d in &diags {
        match d.level {
            hddsgen::validate::Level::Error => {
                errors += 1;
                eprintln!("error: {}", d.message);
            }
            hddsgen::validate::Level::Warning => eprintln!("warning: {}", d.message),
        }
    }
    if errors > 0 {
        eprintln!("hddsgen: {errors} validation error(s); nothing generated");
        std::process::exit(1);
    }
}
