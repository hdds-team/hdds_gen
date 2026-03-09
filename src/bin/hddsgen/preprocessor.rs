// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! C preprocessor for IDL files.
//!
//! Handles `#include`, `#define`, and conditional compilation directives.

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

#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
/// Core preprocessing routine applied to raw IDL source.
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

                        let mut expanded = func.body.clone();
                        for (param, arg) in func.params.iter().zip(args.iter()) {
                            expanded = expanded.replace(param, arg);
                        }
                        out.push_str(&expanded);
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
                        let mut parts = arg.splitn(2, char::is_whitespace);
                        let name = parts.next().unwrap_or("").trim().to_string();
                        let value = parts.next().unwrap_or("").trim().to_string();
                        if arg.contains('(') && arg.contains(')') && arg.contains(',') {
                            let params_start = name.find('(').unwrap_or(name.len());
                            let params_end = name.find(')').unwrap_or(name.len());
                            if params_end > params_start {
                                let macro_name = name[..params_start].trim().to_string();
                                let params = name[params_start + 1..params_end]
                                    .split(',')
                                    .map(|s| s.trim().to_string())
                                    .collect();
                                func_macros.insert(
                                    macro_name,
                                    MacroFunc {
                                        params,
                                        body: value,
                                    },
                                );
                            } else {
                                defines.insert(name, value);
                            }
                        } else {
                            defines.insert(name, value);
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
/// Returns the preprocessed content of just this file (macros expanded, conditionals resolved)
/// and a list of included file paths. Does NOT inline included content.
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

    // Filter out remaining preprocessor directives
    let filtered = processed
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
        .join("\n");

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

                        let mut expanded = func.body.clone();
                        for (param, arg) in func.params.iter().zip(args.iter()) {
                            expanded = expanded.replace(param, arg);
                        }
                        out.push_str(&expanded);
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
                        let mut parts = arg.splitn(2, char::is_whitespace);
                        let name = parts.next().unwrap_or("").trim().to_string();
                        let value = parts.next().unwrap_or("").trim().to_string();
                        if arg.contains('(') && arg.contains(')') && arg.contains(',') {
                            let params_start = name.find('(').unwrap_or(name.len());
                            let params_end = name.find(')').unwrap_or(name.len());
                            if params_end > params_start {
                                let macro_name = name[..params_start].trim().to_string();
                                let params = name[params_start + 1..params_end]
                                    .split(',')
                                    .map(|s| s.trim().to_string())
                                    .collect();
                                func_macros.insert(
                                    macro_name,
                                    MacroFunc {
                                        params,
                                        body: value,
                                    },
                                );
                            } else {
                                defines.insert(name, value);
                            }
                        } else {
                            defines.insert(name, value);
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

/// Read an IDL source (file or stdin) and return the fully preprocessed content with include info.
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

    // Filter out any remaining preprocessor directives after processing.
    // The lexer/parser may still see residual #include/#define lines that weren't
    // fully consumed, causing it to skip subsequent content. Remove them here.
    let content = raw_content
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            // Keep the line unless it's a preprocessor directive
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
        .join("\n");

    Ok(PreprocessResult { content, includes })
}
