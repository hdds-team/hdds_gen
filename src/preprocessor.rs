// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! C preprocessor for IDL files.
//!
//! Handles `#include`, `#define`, and conditional compilation directives.
//! The `hddsgen` binary runs every input through it; other tools (an IDE,
//! a linter) use the same implementation through this module:
//! [`read_and_preprocess`] for a file with its includes inlined,
//! [`preprocess_no_inline`] for one file alone, [`preprocess_str`] for a
//! source held in memory.

use encoding_rs::WINDOWS_1252;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
/// Representation of a function-style macro encountered during preprocessing.
pub struct MacroFunc {
    pub params: Vec<String>,
    pub body: String,
}

/// Result of preprocessing an IDL file.
#[derive(Clone, Debug, Default)]
pub struct PreprocessResult {
    /// The fully preprocessed content (with includes inlined).
    pub content: String,
    /// List of included IDL file paths (in order of inclusion).
    pub includes: Vec<PathBuf>,
}

fn read_file_with_encoding(path: &str) -> io::Result<String> {
    let bytes = fs::read(path)?;
    String::from_utf8(bytes.clone()).map_or_else(
        |_| {
            let (decoded, _, _) = WINDOWS_1252.decode(&bytes);
            Ok(decoded.into_owned())
        },
        Ok,
    )
}

fn read_input(input: &Path) -> io::Result<String> {
    if input == Path::new("-") {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else {
        read_file_with_encoding(&input.to_string_lossy())
    }
}

/// Preprocess an IDL file, resolving includes and macros.
///
/// # Errors
///
/// Returns an error when the file or an included file cannot be read.
#[allow(clippy::implicit_hasher)]
pub fn preprocess_from_file(
    path: &Path,
    include_dirs: &[PathBuf],
    visited: &mut HashSet<PathBuf>,
    defines: &mut HashMap<String, String>,
    func_macros: &mut HashMap<String, MacroFunc>,
    includes: &mut Vec<PathBuf>,
) -> io::Result<String> {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical.clone()) {
        return Ok(String::new());
    }
    let content = read_file_with_encoding(&canonical.to_string_lossy())?;
    preprocess_content(
        &content,
        canonical.parent(),
        include_dirs,
        visited,
        defines,
        func_macros,
        includes,
    )
}

#[allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    clippy::implicit_hasher
)]
/// Core preprocessing routine applied to raw IDL source.
///
/// # Errors
///
/// Returns an error when an included file cannot be found or read.
pub fn preprocess_content(
    content: &str,
    current_dir: Option<&Path>,
    include_dirs: &[PathBuf],
    visited: &mut HashSet<PathBuf>,
    defines: &mut HashMap<String, String>,
    func_macros: &mut HashMap<String, MacroFunc>,
    includes: &mut Vec<PathBuf>,
) -> io::Result<String> {
    #[derive(Clone, Debug)]
    struct IfFrame {
        any_true: bool,
        taking: bool,
    }

    fn stack_active(stack: &[IfFrame]) -> bool {
        stack.iter().all(|f| f.taking)
    }

    fn expand_macros(
        line: &str,
        defines: &HashMap<String, String>,
        func_macros: &HashMap<String, MacroFunc>,
    ) -> String {
        let mut out = String::with_capacity(line.len());
        let bytes: Vec<char> = line.chars().collect();
        let mut i = 0usize;
        let is_ident_start = |c: char| c == '_' || c.is_ascii_alphabetic();
        let is_ident_char = |c: char| c == '_' || c.is_ascii_alphanumeric();

        while i < bytes.len() {
            let c = bytes[i];
            if is_ident_start(c) {
                let start = i;
                i += 1;
                while i < bytes.len() && is_ident_char(bytes[i]) {
                    i += 1;
                }
                let name: String = bytes[start..i].iter().collect();

                let mut j = i;
                while j < bytes.len() && bytes[j].is_whitespace() {
                    j += 1;
                }

                if j < bytes.len() && bytes[j] == '(' {
                    if let Some(func) = func_macros.get(name.trim()) {
                        let mut depth = 1usize;
                        let mut k = j + 1;
                        let mut current_arg = String::new();
                        let mut args = Vec::new();
                        while k < bytes.len() {
                            let ch = bytes[k];
                            match ch {
                                '(' => {
                                    depth += 1;
                                    current_arg.push(ch);
                                }
                                ')' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        args.push(current_arg.trim().to_string());
                                        break;
                                    }
                                    current_arg.push(ch);
                                }
                                ',' if depth == 1 => {
                                    args.push(current_arg.trim().to_string());
                                    current_arg.clear();
                                }
                                _ => current_arg.push(ch),
                            }
                            k += 1;
                        }

                        out.push_str(&substitute(func, &args));
                        i = k + 1;
                        continue;
                    }
                }

                if let Some(value) = defines.get(name.trim()) {
                    out.push_str(value);
                } else {
                    out.push_str(&name);
                }
            } else {
                out.push(c);
                i += 1;
            }
        }

        out
    }

    let mut out = String::new();
    let mut if_stack: Vec<IfFrame> = Vec::new();

    for raw_line in content.lines() {
        let line = raw_line.trim_start();
        if let Some(stripped) = line.strip_prefix('#') {
            let mut parts = stripped.splitn(2, char::is_whitespace);
            let directive = parts.next().unwrap_or("").trim();
            let arg = parts.next().unwrap_or("").trim();

            match directive {
                "define" => {
                    if !arg.is_empty() && stack_active(&if_stack) {
                        match parse_define(arg) {
                            Define::Object(name, value) => {
                                defines.insert(name, value);
                            }
                            Define::Function(name, func) => {
                                func_macros.insert(name, func);
                            }
                        }
                    }
                    continue;
                }
                "undef" => {
                    if stack_active(&if_stack) {
                        defines.remove(arg.trim());
                    }
                    continue;
                }
                "ifdef" => {
                    let cond = defines.contains_key(arg.trim());
                    let parent = stack_active(&if_stack);
                    if_stack.push(IfFrame {
                        any_true: cond,
                        taking: cond && parent,
                    });
                    continue;
                }
                "ifndef" => {
                    let cond = !defines.contains_key(arg.trim());
                    let parent = stack_active(&if_stack);
                    if_stack.push(IfFrame {
                        any_true: cond,
                        taking: cond && parent,
                    });
                    continue;
                }
                "if" => {
                    let expr = arg.trim();
                    let cond = if expr == "1" {
                        true
                    } else if expr == "0" {
                        false
                    } else if let Some(inner) = expr.strip_prefix("defined(") {
                        defines.contains_key(inner.trim_end_matches(')').trim())
                    } else if let Some(rest) = expr.strip_prefix("defined ") {
                        defines.contains_key(rest.trim())
                    } else if let Ok(n) = expr.parse::<i64>() {
                        n != 0
                    } else {
                        false
                    };
                    let parent = stack_active(&if_stack);
                    if_stack.push(IfFrame {
                        any_true: cond,
                        taking: cond && parent,
                    });
                    continue;
                }
                "elif" => {
                    let len = if_stack.len();
                    if len > 0 {
                        let (prefix, last) = if_stack.split_at_mut(len - 1);
                        if let Some(top) = last.last_mut() {
                            let expr = arg.trim();
                            let cond = if expr == "1" {
                                true
                            } else if expr == "0" {
                                false
                            } else if let Some(inner) = expr.strip_prefix("defined(") {
                                defines.contains_key(inner.trim_end_matches(')').trim())
                            } else if let Some(rest) = expr.strip_prefix("defined ") {
                                defines.contains_key(rest.trim())
                            } else if let Ok(n) = expr.parse::<i64>() {
                                n != 0
                            } else {
                                false
                            };
                            if top.any_true {
                                top.taking = false;
                            } else {
                                let parent = prefix.iter().all(|f| f.taking);
                                top.taking = cond && parent;
                                if cond {
                                    top.any_true = true;
                                }
                            }
                        }
                    }
                    continue;
                }
                "else" => {
                    let len = if_stack.len();
                    if len > 0 {
                        let (prefix, last) = if_stack.split_at_mut(len - 1);
                        if let Some(top) = last.last_mut() {
                            if top.any_true {
                                top.taking = false;
                            } else {
                                let parent = prefix.iter().all(|f| f.taking);
                                top.taking = parent;
                                top.any_true = true;
                            }
                        }
                    }
                    continue;
                }
                "endif" => {
                    if_stack.pop();
                    continue;
                }
                "include" => {
                    if !stack_active(&if_stack) {
                        continue;
                    }
                    let include_path = arg.trim_matches(['"', '<', '>']);
                    let mut resolved = None;

                    if let Some(dir) = current_dir {
                        let candidate = dir.join(include_path);
                        if candidate.exists() {
                            resolved = Some(candidate);
                        }
                    }

                    if resolved.is_none() {
                        for dir in include_dirs {
                            let candidate = dir.join(include_path);
                            if candidate.exists() {
                                resolved = Some(candidate);
                                break;
                            }
                        }
                    }

                    if let Some(path) = resolved {
                        // Track the include path (store the original path as specified)
                        includes.push(PathBuf::from(include_path));
                        let preprocessed = preprocess_from_file(
                            &path,
                            include_dirs,
                            visited,
                            defines,
                            func_macros,
                            includes,
                        )?;
                        out.push_str(&preprocessed);
                    } else {
                        // A missing file is an error, as for a C
                        // preprocessor: skipping it would drop every
                        // declaration it holds without a word.
                        return Err(io::Error::new(
                            io::ErrorKind::NotFound,
                            format!(
                                "#include {arg}: file not found (searched the including file's \
                                 directory and {} include director{})",
                                include_dirs.len(),
                                if include_dirs.len() == 1 { "y" } else { "ies" }
                            ),
                        ));
                    }
                    continue;
                }
                _ => {}
            }
        }

        if stack_active(&if_stack) {
            out.push_str(&expand_macros(raw_line, defines, func_macros));
        }
        out.push('\n');
    }
    Ok(out)
}

/// Read an IDL source without inlining includes.
///
/// Returns the preprocessed content of just this file (macros expanded,
/// conditionals resolved) and a list of included file paths. Does NOT
/// inline included content.
///
/// # Errors
///
/// Returns an error when the file cannot be read.
pub fn preprocess_no_inline(
    input: &Path,
    include_dirs: &[PathBuf],
) -> io::Result<PreprocessResult> {
    let content = read_file_with_encoding(&input.to_string_lossy())?;
    let mut includes = Vec::new();
    let mut defines = HashMap::new();
    let mut func_macros = HashMap::new();

    // Process content but skip the actual file inclusion
    let processed = preprocess_content_no_inline(
        &content,
        input.parent(),
        include_dirs,
        &mut defines,
        &mut func_macros,
        &mut includes,
    )?;

    let filtered = strip_directives(&processed);

    Ok(PreprocessResult {
        content: filtered,
        includes,
    })
}

/// Preprocess content without inlining includes - just collect include paths.
// codegen function - line count from template output
#[allow(clippy::too_many_lines)]
// Result wrapper kept for API consistency with preprocess_content
#[allow(clippy::unnecessary_wraps)]
fn preprocess_content_no_inline(
    content: &str,
    current_dir: Option<&Path>,
    include_dirs: &[PathBuf],
    defines: &mut HashMap<String, String>,
    func_macros: &mut HashMap<String, MacroFunc>,
    includes: &mut Vec<PathBuf>,
) -> io::Result<String> {
    #[derive(Clone, Debug)]
    struct IfFrame {
        any_true: bool,
        taking: bool,
    }

    fn stack_active(stack: &[IfFrame]) -> bool {
        stack.iter().all(|f| f.taking)
    }

    fn expand_macros(
        line: &str,
        defines: &HashMap<String, String>,
        func_macros: &HashMap<String, MacroFunc>,
    ) -> String {
        let mut out = String::with_capacity(line.len());
        let bytes: Vec<char> = line.chars().collect();
        let mut i = 0usize;
        let is_ident_start = |c: char| c == '_' || c.is_ascii_alphabetic();
        let is_ident_char = |c: char| c == '_' || c.is_ascii_alphanumeric();

        while i < bytes.len() {
            let c = bytes[i];
            if is_ident_start(c) {
                let start = i;
                i += 1;
                while i < bytes.len() && is_ident_char(bytes[i]) {
                    i += 1;
                }
                let name: String = bytes[start..i].iter().collect();

                let mut j = i;
                while j < bytes.len() && bytes[j].is_whitespace() {
                    j += 1;
                }

                if j < bytes.len() && bytes[j] == '(' {
                    if let Some(func) = func_macros.get(name.trim()) {
                        let mut depth = 1usize;
                        let mut k = j + 1;
                        let mut current_arg = String::new();
                        let mut args = Vec::new();
                        while k < bytes.len() {
                            let ch = bytes[k];
                            match ch {
                                '(' => {
                                    depth += 1;
                                    current_arg.push(ch);
                                }
                                ')' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        args.push(current_arg.trim().to_string());
                                        break;
                                    }
                                    current_arg.push(ch);
                                }
                                ',' if depth == 1 => {
                                    args.push(current_arg.trim().to_string());
                                    current_arg.clear();
                                }
                                _ => current_arg.push(ch),
                            }
                            k += 1;
                        }

                        out.push_str(&substitute(func, &args));
                        i = k + 1;
                        continue;
                    }
                }

                if let Some(value) = defines.get(name.trim()) {
                    out.push_str(value);
                } else {
                    out.push_str(&name);
                }
            } else {
                out.push(c);
                i += 1;
            }
        }

        out
    }

    let mut out = String::new();
    let mut if_stack: Vec<IfFrame> = Vec::new();

    for raw_line in content.lines() {
        let line = raw_line.trim_start();
        if let Some(stripped) = line.strip_prefix('#') {
            let mut parts = stripped.splitn(2, char::is_whitespace);
            let directive = parts.next().unwrap_or("").trim();
            let arg = parts.next().unwrap_or("").trim();

            match directive {
                "define" => {
                    if !arg.is_empty() && stack_active(&if_stack) {
                        match parse_define(arg) {
                            Define::Object(name, value) => {
                                defines.insert(name, value);
                            }
                            Define::Function(name, func) => {
                                func_macros.insert(name, func);
                            }
                        }
                    }
                    continue;
                }
                "undef" => {
                    if stack_active(&if_stack) {
                        defines.remove(arg.trim());
                    }
                    continue;
                }
                "ifdef" => {
                    let cond = defines.contains_key(arg.trim());
                    let parent = stack_active(&if_stack);
                    if_stack.push(IfFrame {
                        any_true: cond,
                        taking: cond && parent,
                    });
                    continue;
                }
                "ifndef" => {
                    let cond = !defines.contains_key(arg.trim());
                    let parent = stack_active(&if_stack);
                    if_stack.push(IfFrame {
                        any_true: cond,
                        taking: cond && parent,
                    });
                    continue;
                }
                "if" => {
                    let expr = arg.trim();
                    let cond = if expr == "1" {
                        true
                    } else if expr == "0" {
                        false
                    } else if let Some(inner) = expr.strip_prefix("defined(") {
                        defines.contains_key(inner.trim_end_matches(')').trim())
                    } else if let Some(rest) = expr.strip_prefix("defined ") {
                        defines.contains_key(rest.trim())
                    } else if let Ok(n) = expr.parse::<i64>() {
                        n != 0
                    } else {
                        false
                    };
                    let parent = stack_active(&if_stack);
                    if_stack.push(IfFrame {
                        any_true: cond,
                        taking: cond && parent,
                    });
                    continue;
                }
                "elif" => {
                    let len = if_stack.len();
                    if len > 0 {
                        let (prefix, last) = if_stack.split_at_mut(len - 1);
                        if let Some(top) = last.last_mut() {
                            let expr = arg.trim();
                            let cond = if expr == "1" {
                                true
                            } else if expr == "0" {
                                false
                            } else if let Some(inner) = expr.strip_prefix("defined(") {
                                defines.contains_key(inner.trim_end_matches(')').trim())
                            } else if let Some(rest) = expr.strip_prefix("defined ") {
                                defines.contains_key(rest.trim())
                            } else if let Ok(n) = expr.parse::<i64>() {
                                n != 0
                            } else {
                                false
                            };
                            if top.any_true {
                                top.taking = false;
                            } else {
                                let parent = prefix.iter().all(|f| f.taking);
                                top.taking = cond && parent;
                                if cond {
                                    top.any_true = true;
                                }
                            }
                        }
                    }
                    continue;
                }
                "else" => {
                    let len = if_stack.len();
                    if len > 0 {
                        let (prefix, last) = if_stack.split_at_mut(len - 1);
                        if let Some(top) = last.last_mut() {
                            if top.any_true {
                                top.taking = false;
                            } else {
                                let parent = prefix.iter().all(|f| f.taking);
                                top.taking = parent;
                                top.any_true = true;
                            }
                        }
                    }
                    continue;
                }
                "endif" => {
                    if_stack.pop();
                    continue;
                }
                "include" => {
                    if !stack_active(&if_stack) {
                        continue;
                    }
                    // Just record the include, don't inline
                    let include_path = arg.trim_matches(['"', '<', '>']);
                    let mut resolved = None;

                    if let Some(dir) = current_dir {
                        let candidate = dir.join(include_path);
                        if candidate.exists() {
                            resolved = Some(candidate);
                        }
                    }

                    if resolved.is_none() {
                        for dir in include_dirs {
                            let candidate = dir.join(include_path);
                            if candidate.exists() {
                                resolved = Some(candidate);
                                break;
                            }
                        }
                    }

                    if let Some(path) = resolved {
                        includes.push(path);
                    } else {
                        // Keep as-is if not found
                        includes.push(PathBuf::from(include_path));
                    }
                    continue;
                }
                _ => {}
            }
        }

        if stack_active(&if_stack) {
            out.push_str(&expand_macros(raw_line, defines, func_macros));
        }
        out.push('\n');
    }
    Ok(out)
}

/// Preprocess an IDL source held in memory (an editor buffer, say):
/// includes are looked up in `base_dir` (the directory of the source, if
/// any) then `include_dirs`, and inlined.
///
/// # Errors
///
/// Returns an error when an included file cannot be read.
pub fn preprocess_str(
    content: &str,
    base_dir: Option<&Path>,
    include_dirs: &[PathBuf],
) -> io::Result<PreprocessResult> {
    let mut includes = Vec::new();
    let raw = preprocess_content(
        content,
        base_dir,
        include_dirs,
        &mut HashSet::new(),
        &mut HashMap::new(),
        &mut HashMap::new(),
        &mut includes,
    )?;
    Ok(PreprocessResult {
        content: strip_directives(&raw),
        includes,
    })
}

/// The lines of `content` that are not preprocessor directives: the lexer
/// would otherwise skip what follows a residual `#include` or `#define`.
fn strip_directives(content: &str) -> String {
    content
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("#include")
                && !trimmed.starts_with("#define")
                && !trimmed.starts_with("#undef")
                && !trimmed.starts_with("#ifdef")
                && !trimmed.starts_with("#ifndef")
                && !trimmed.starts_with("#if ")
                && !trimmed.starts_with("#elif")
                && !trimmed.starts_with("#else")
                && !trimmed.starts_with("#endif")
                && !trimmed.starts_with("#pragma")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Read an IDL source (file or stdin) and return the fully preprocessed content with include info.
///
/// # Errors
///
/// Returns an error when the source or an included file cannot be read.
pub fn read_and_preprocess(input: &Path, include_dirs: &[PathBuf]) -> io::Result<PreprocessResult> {
    let mut includes = Vec::new();
    let raw_content = if input == Path::new("-") {
        let raw = read_input(input)?;
        let mut visited = HashSet::new();
        let mut defines = HashMap::new();
        let mut func_macros = HashMap::new();
        preprocess_content(
            &raw,
            None,
            include_dirs,
            &mut visited,
            &mut defines,
            &mut func_macros,
            &mut includes,
        )?
    } else {
        let mut visited = HashSet::new();
        let mut defines = HashMap::new();
        let mut func_macros = HashMap::new();
        preprocess_from_file(
            input,
            include_dirs,
            &mut visited,
            &mut defines,
            &mut func_macros,
            &mut includes,
        )?
    };

    // Remove the directives left after processing: the lexer/parser would
    // skip what follows a residual #include/#define line.
    let content = strip_directives(&raw_content);

    Ok(PreprocessResult { content, includes })
}

enum Define {
    Object(String, String),
    Function(String, MacroFunc),
}

/// A `#define` argument: `NAME value`, or `NAME(a, b) body` when the
/// parenthesis follows the name with no space (C 6.10.3: then it is a
/// function-like macro, whatever spaces the parameter list holds).
fn parse_define(arg: &str) -> Define {
    let name_end = arg
        .find(|c: char| !(c == '_' || c.is_ascii_alphanumeric()))
        .unwrap_or(arg.len());
    let name = arg[..name_end].to_string();
    let rest = &arg[name_end..];
    if let Some(after) = rest.strip_prefix('(') {
        if let Some(close) = after.find(')') {
            let params = after[..close]
                .split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect();
            return Define::Function(
                name,
                MacroFunc {
                    params,
                    body: after[close + 1..].trim().to_string(),
                },
            );
        }
    }
    Define::Object(name, rest.trim().to_string())
}

/// The body of `func` with its parameters replaced by `args`, token by
/// token (C 6.10.3.1-3): `#p` is the argument as a string literal (quotes
/// and backslashes escaped, inner whitespace collapsed), `a ## b` pastes
/// the two tokens, any other occurrence of a parameter is the argument.
fn substitute(func: &MacroFunc, args: &[String]) -> String {
    let arg_of = |name: &str| {
        func.params
            .iter()
            .position(|p| p == name)
            .map(|i| args.get(i).cloned().unwrap_or_default())
    };
    let body: Vec<char> = func.body.chars().collect();
    let mut out = String::with_capacity(func.body.len());
    let mut i = 0;
    while i < body.len() {
        let c = body[i];
        if c == '#' && body.get(i + 1) == Some(&'#') {
            // Paste: drop the spaces around `##`.
            while out.ends_with(char::is_whitespace) {
                out.pop();
            }
            i += 2;
            while i < body.len() && body[i].is_whitespace() {
                i += 1;
            }
            continue;
        }
        if c == '#' {
            let mut j = i + 1;
            while j < body.len() && body[j].is_whitespace() {
                j += 1;
            }
            let start = j;
            while j < body.len() && (body[j] == '_' || body[j].is_ascii_alphanumeric()) {
                j += 1;
            }
            let word: String = body[start..j].iter().collect();
            if let Some(arg) = arg_of(&word) {
                let collapsed = arg.split_whitespace().collect::<Vec<_>>().join(" ");
                out.push('"');
                for ch in collapsed.chars() {
                    if ch == '"' || ch == '\\' {
                        out.push('\\');
                    }
                    out.push(ch);
                }
                out.push('"');
                i = j;
                continue;
            }
        }
        if c == '_' || c.is_ascii_alphabetic() {
            let start = i;
            while i < body.len() && (body[i] == '_' || body[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            let word: String = body[start..i].iter().collect();
            match arg_of(&word) {
                Some(arg) => out.push_str(&arg),
                None => out.push_str(&word),
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}
