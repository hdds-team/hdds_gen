// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Project generation for --example flag
//!
//! Generates complete project structures with publisher/subscriber examples
//! and build system configuration.
//!
//! Note: uninlined_format_args allowed here due to extensive format!() usage
//! in code generation that would require significant refactoring.

#![allow(clippy::uninlined_format_args)]
#![allow(clippy::format_push_string)]

use crate::ast::{Definition, IdlFile, Struct};
use crate::codegen::keywords::rust_ident;
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

/// Crate name for a generated project, derived from the IDL file name.
///
/// `to_snake_case` alone can yield a Rust keyword (`mod.idl`, `type.idl`), and a
/// crate whose library target is a keyword cannot be named in a `use` statement.
/// Such a name gets a `_types` suffix so the Cargo manifest and the generated
/// `use` agree.
fn crate_ident(base_name: &str) -> String {
    let snake = to_snake_case(base_name);
    if rust_ident(&snake) == snake {
        snake
    } else {
        format!("{snake}_types")
    }
}

/// Build system choice for project generation
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BuildSystem {
    /// Cargo.toml (Rust default)
    #[default]
    Cargo,
    /// CMakeLists.txt (C/C++ default)
    Cmake,
    /// Makefile
    Make,
}

/// Represents a generated project with multiple files
#[derive(Debug)]
pub struct GeneratedProject {
    /// Map of relative path -> file content
    pub files: HashMap<String, String>,
}

impl Default for GeneratedProject {
    fn default() -> Self {
        Self::new()
    }
}

impl GeneratedProject {
    #[must_use]
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
        }
    }

    pub fn add_file(&mut self, path: impl Into<String>, content: impl Into<String>) {
        self.files.insert(path.into(), content.into());
    }

    /// Write all files to the output directory
    ///
    /// # Errors
    /// Returns an error if directory creation or file writing fails.
    pub fn write_to(&self, out_dir: &Path) -> std::io::Result<Vec<String>> {
        use std::fs;
        let mut created = Vec::new();

        for (rel_path, content) in &self.files {
            let full_path = out_dir.join(rel_path);
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&full_path, content)?;
            created.push(rel_path.clone());
        }

        Ok(created)
    }
}

/// Find the first struct in the AST (used for example generation)
fn find_first_struct_with_path(
    defs: &[Definition],
    path: Vec<String>,
) -> Option<(&Struct, Vec<String>)> {
    for def in defs {
        match def {
            Definition::Struct(s) => return Some((s, path)),
            Definition::Module(m) => {
                let mut new_path = path.clone();
                new_path.push(m.name.clone());
                if let Some(result) = find_first_struct_with_path(&m.definitions, new_path) {
                    return Some(result);
                }
            }
            _ => {}
        }
    }
    None
}

/// Extract base name from IDL filename (e.g., `HelloWorld.idl` -> `HelloWorld`)
fn idl_basename(idl_path: &str) -> String {
    let path = Path::new(idl_path);
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Generated")
        .to_string()
}

/// Convert `CamelCase` to `snake_case`
fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

/// Convert to C identifier (just lowercase, no underscores - matches C backend)
fn to_c_ident(s: &str) -> String {
    s.to_ascii_lowercase()
}

// ============================================================================
// C++ Project Generation
// ============================================================================

/// Generate a complete C++ project
#[must_use]
pub fn generate_cpp_project(
    ast: &IdlFile,
    types_code: &str,
    idl_path: &str,
    build_system: BuildSystem,
) -> GeneratedProject {
    let mut project = GeneratedProject::new();
    let base_name = idl_basename(idl_path);

    // Find first struct for examples
    let (first_struct, namespace_path) = find_first_struct_with_path(&ast.definitions, Vec::new())
        .unwrap_or_else(|| {
            // Create a dummy struct if none found
            static DUMMY: LazyLock<Struct> = LazyLock::new(|| Struct {
                name: "Message".to_string(),
                fields: vec![],
                annotations: vec![],
                base_struct: None,
                extensibility: None,
            });
            (&*DUMMY, vec![])
        });

    let struct_name = &first_struct.name;
    let full_type = if namespace_path.is_empty() {
        struct_name.clone()
    } else {
        format!("{}::{}", namespace_path.join("::"), struct_name)
    };

    // 1. Types header (main generated code)
    project.add_file(format!("{base_name}.hpp"), types_code.to_string());

    // 2. Publisher source
    project.add_file(
        format!("{base_name}Publisher.cxx"),
        generate_cpp_publisher(&base_name, &full_type, struct_name, first_struct),
    );

    // 3. Subscriber source
    project.add_file(
        format!("{base_name}Subscriber.cxx"),
        generate_cpp_subscriber(&base_name, &full_type, struct_name, first_struct),
    );

    // 4. Build system file
    match build_system {
        BuildSystem::Cmake => {
            project.add_file("CMakeLists.txt", generate_cpp_cmake(&base_name));
        }
        BuildSystem::Make => {
            project.add_file("Makefile", generate_cpp_makefile(&base_name));
        }
        BuildSystem::Cargo => {
            // Cargo doesn't make sense for C++, fall back to CMake
            project.add_file("CMakeLists.txt", generate_cpp_cmake(&base_name));
        }
    }

    project
}

fn generate_cpp_publisher(
    base_name: &str,
    full_type: &str,
    struct_name: &str,
    s: &Struct,
) -> String {
    let field_inits = generate_cpp_field_inits(s);
    let field_print = generate_cpp_field_print(s);
    format!(
        r#"// Generated by hddsgen --example
// DDS Publisher using HDDS C++ API

#include "{base_name}.hpp"
#include <hdds.hpp>

#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <iostream>
#include <thread>

// How long to wait for a subscriber to match before giving up.
constexpr std::chrono::seconds kDiscoveryTimeout{{30}};

int main(int argc, char* argv[]) {{
    try {{
        hdds::logging::init(hdds::LogLevel::Warn);

        // The SDK applies the domain rule: HDDS_DOMAIN_ID when set and valid
        // (0..232), else 0. A malformed value makes the constructor throw.
        hdds::Participant participant("{struct_name}Example", hdds::TransportMode::UdpMulticast);
        const std::uint32_t domain_id = participant.domain_id();
        std::cout << "[HDDS] Domain " << domain_id << " | Ready" << std::endl;

        auto writer = participant.create_writer<{full_type}>("{struct_name}Topic");

        // A sample written before discovery completes is never delivered.
        auto deadline = std::chrono::steady_clock::now() + kDiscoveryTimeout;
        while (writer.get_publication_matched_status().current_count == 0) {{
            if (std::chrono::steady_clock::now() >= deadline) {{
                std::cerr << "[PUB] No subscriber matched on domain " << domain_id
                          << " within " << kDiscoveryTimeout.count()
                          << " s; start the subscriber first" << std::endl;
                return 1;
            }}
            std::this_thread::sleep_for(std::chrono::milliseconds(50));
        }}
        std::cout << "[PUB] Subscriber matched" << std::endl;

        int sample_count = 10;
        if (argc > 1) {{
            sample_count = std::atoi(argv[1]);
        }}

        for (int i = 0; i < sample_count; ++i) {{
            {full_type} msg;
{field_inits}
            writer.write(msg);

            std::cout << "[PUB] {struct_name} {{ "{field_print} << " }}" << std::endl;

            std::this_thread::sleep_for(std::chrono::seconds(1));
        }}

    }} catch (const hdds::Error& e) {{
        std::cerr << "HDDS Error: " << e.what() << std::endl;
        return 1;
    }}

    return 0;
}}
"#,
        base_name = base_name,
        struct_name = struct_name,
        full_type = full_type,
        field_inits = field_inits,
        field_print = field_print,
    )
}

fn generate_cpp_subscriber(
    base_name: &str,
    full_type: &str,
    struct_name: &str,
    s: &Struct,
) -> String {
    format!(
        r#"// Generated by hddsgen --example
// DDS Subscriber using HDDS C++ API

#include "{base_name}.hpp"
#include <hdds.hpp>

#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <iostream>

using namespace std::chrono_literals;

// Exit when no sample has arrived for this long.
constexpr std::chrono::seconds kIdleTimeout{{30}};

int main(int argc, char* argv[]) {{
    try {{
        hdds::logging::init(hdds::LogLevel::Warn);

        // The SDK applies the domain rule: HDDS_DOMAIN_ID when set and valid
        // (0..232), else 0. A malformed value makes the constructor throw.
        hdds::Participant participant("{struct_name}Example", hdds::TransportMode::UdpMulticast);
        const std::uint32_t domain_id = participant.domain_id();

        std::cout << "[HDDS] Domain " << domain_id << " | UDP multicast | Ready" << std::endl;

        auto reader = participant.create_reader<{full_type}>("{struct_name}Topic");

        hdds::WaitSet waitset;
        waitset.attach(reader.get_status_condition());

        int sample_count = 10;
        if (argc > 1) {{
            sample_count = std::atoi(argv[1]);
        }}

        int received = 0;
        auto last_sample = std::chrono::steady_clock::now();
        while (received < sample_count) {{
            if (waitset.wait(1s)) {{
                while (auto msg = reader.take()) {{
                    ++received;
                    last_sample = std::chrono::steady_clock::now();
                    std::cout << "[SUB] Received: {struct_name} {{ "{field_print_sub} << " }}" << std::endl;
                }}
            }}
            if (std::chrono::steady_clock::now() - last_sample >= kIdleTimeout) {{
                std::cout << "[SUB] No sample for " << kIdleTimeout.count() << " s, giving up" << std::endl;
                break;
            }}
        }}

        std::cout << "[SUB] Received " << received << "/" << sample_count << " samples" << std::endl;

    }} catch (const hdds::Error& e) {{
        std::cerr << "HDDS Error: " << e.what() << std::endl;
        return 1;
    }}

    return 0;
}}
"#,
        base_name = base_name,
        struct_name = struct_name,
        full_type = full_type,
        field_print_sub = generate_cpp_field_print_with_prefix(s, "msg->"),
    )
}

fn generate_cpp_cmake(base_name: &str) -> String {
    format!(
        r"# Generated by hddsgen --example
cmake_minimum_required(VERSION 3.16)
project({base_name}_example CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Find HDDS C++ SDK
# Pass -DCMAKE_PREFIX_PATH=/path/to/hdds/sdk/cmake when configuring
find_package(hdds REQUIRED)

# Publisher executable
add_executable({base_name}_publisher
    {base_name}Publisher.cxx
)
target_include_directories({base_name}_publisher PRIVATE ${{CMAKE_CURRENT_SOURCE_DIR}})
target_link_libraries({base_name}_publisher PRIVATE hdds::hdds)

# Subscriber executable
add_executable({base_name}_subscriber
    {base_name}Subscriber.cxx
)
target_include_directories({base_name}_subscriber PRIVATE ${{CMAKE_CURRENT_SOURCE_DIR}})
target_link_libraries({base_name}_subscriber PRIVATE hdds::hdds)
",
        base_name = base_name,
    )
}

fn generate_cpp_makefile(base_name: &str) -> String {
    format!(
        r"# Generated by hddsgen --example --build-system=make
# Set HDDS_ROOT to the root of your HDDS installation
HDDS_ROOT ?= $(error Set HDDS_ROOT to HDDS install path, e.g. make HDDS_ROOT=/path/to/hdds)

CXX ?= g++
CXXFLAGS ?= -Wall -Wextra -O2 -std=c++17 \
	-I. \
	-I$(HDDS_ROOT)/sdk/c/include \
	-I$(HDDS_ROOT)/sdk/cxx/include
LDFLAGS ?= $(HDDS_ROOT)/sdk/cxx/build/libhdds_cxx.a \
	$(HDDS_ROOT)/target/release/libhdds_c.a \
	-lpthread -ldl -lm

.PHONY: all clean

all: {base_name}_publisher {base_name}_subscriber

{base_name}_publisher: {base_name}Publisher.cxx {base_name}.hpp
	$(CXX) $(CXXFLAGS) -o $@ {base_name}Publisher.cxx $(LDFLAGS)

{base_name}_subscriber: {base_name}Subscriber.cxx {base_name}.hpp
	$(CXX) $(CXXFLAGS) -o $@ {base_name}Subscriber.cxx $(LDFLAGS)

clean:
	rm -f {base_name}_publisher {base_name}_subscriber
",
        base_name = base_name,
    )
}

fn generate_cpp_field_inits(s: &Struct) -> String {
    use crate::types::{IdlType, PrimitiveType};

    let mut inits = String::new();
    for f in &s.fields {
        let init = match &f.field_type {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Boolean => "false".to_string(),
                PrimitiveType::Char => "'A'".to_string(),
                PrimitiveType::WChar => "L'A'".to_string(),
                PrimitiveType::Long | PrimitiveType::Int32 => format!("i + {}", 42),
                PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => {
                    format!("static_cast<uint32_t>(i + {})", 42)
                }
                PrimitiveType::Float => "3.14f".to_string(),
                PrimitiveType::Double | PrimitiveType::LongDouble => "3.14159".to_string(),
                PrimitiveType::String => "\"hello\"".to_string(),
                PrimitiveType::WString => "L\"hello\"".to_string(),
                _ => "0".to_string(),
            },
            IdlType::Named(n) => format!("{}()", n),
            _ => "{}".to_string(),
        };
        inits.push_str(&format!("            msg.{} = {};\n", f.name, init));
    }
    inits
}

/// Generate `<< "field: " << msg.field` chain for C++ `std::cout` (publisher uses `msg.`)
fn generate_cpp_field_print(s: &Struct) -> String {
    generate_cpp_field_print_with_prefix(s, "msg.")
}

/// Generate `<< "field: " << prefix.field` chain with configurable prefix
fn generate_cpp_field_print_with_prefix(s: &Struct, prefix: &str) -> String {
    let mut parts = Vec::new();
    for f in &s.fields {
        parts.push(format!(
            "<< \"{name}: \" << {prefix}{name}",
            name = f.name,
            prefix = prefix,
        ));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("{} ", parts.join(" << \", \" "))
    }
}

// ============================================================================
// Rust Project Generation
// ============================================================================

/// Generate a complete Rust project
#[must_use]
pub fn generate_rust_project(
    ast: &IdlFile,
    types_code: &str,
    idl_path: &str,
    hdds_path: Option<&std::path::Path>,
) -> GeneratedProject {
    let mut project = GeneratedProject::new();
    let base_name = idl_basename(idl_path);
    let crate_name = crate_ident(&base_name);

    // Find first struct for examples
    let (first_struct, module_path) = find_first_struct_with_path(&ast.definitions, Vec::new())
        .unwrap_or_else(|| {
            static DUMMY: LazyLock<Struct> = LazyLock::new(|| Struct {
                name: "Message".to_string(),
                fields: vec![],
                annotations: vec![],
                base_struct: None,
                extensibility: None,
            });
            (&*DUMMY, vec![])
        });

    let struct_name = &first_struct.name;
    let full_type = if module_path.is_empty() {
        struct_name.clone()
    } else {
        // The Rust backend emits `pub mod <IdlName>` verbatim (under
        // allow(non_snake_case)), so the path a generated example imports must use
        // the IDL spelling too. Snake-casing it here produced `use crate::demo::T`
        // against a `pub mod Demo` and the project did not compile.
        format!(
            "{}::{}",
            module_path
                .iter()
                .map(|s| rust_ident(s).into_owned())
                .collect::<Vec<_>>()
                .join("::"),
            struct_name
        )
    };

    // 1. Cargo.toml
    project.add_file(
        "Cargo.toml",
        generate_rust_cargo_toml(&crate_name, &to_snake_case(&base_name), hdds_path),
    );

    // 2. src/<basename>.rs (types -- named after IDL, not generic lib.rs)
    project.add_file(
        format!("src/{}.rs", to_snake_case(&base_name)),
        types_code.to_string(),
    );

    // 3. src/bin/publisher.rs
    project.add_file(
        "src/bin/publisher.rs",
        generate_rust_publisher(&crate_name, &full_type, struct_name, first_struct),
    );

    // 4. src/bin/subscriber.rs
    project.add_file(
        "src/bin/subscriber.rs",
        generate_rust_subscriber(&crate_name, &full_type, struct_name),
    );

    project
}

fn generate_rust_cargo_toml(
    crate_name: &str,
    types_module: &str,
    hdds_path: Option<&std::path::Path>,
) -> String {
    let hdds_dep = hdds_path.map_or_else(
        || {
            format!(
                r#"hdds = "{}""#,
                crate::codegen::rust_backend::compat_guard::REQUIRED_HDDS_VERSION
            )
        },
        |path| {
            let resolved = resolve_local_crate_path(path, "hdds");
            let display_path = resolved.display();
            format!(r#"hdds = {{ path = "{display_path}" }}"#)
        },
    );
    format!(
        r#"# Generated by hddsgen --example
[workspace]

[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/{types_module}.rs"

[[bin]]
name = "publisher"
path = "src/bin/publisher.rs"

[[bin]]
name = "subscriber"
path = "src/bin/subscriber.rs"

[dependencies]
{hdds_dep}
"#,
        crate_name = crate_name,
        types_module = types_module,
        hdds_dep = hdds_dep,
    )
}

fn generate_rust_publisher(
    crate_name: &str,
    full_type: &str,
    struct_name: &str,
    s: &Struct,
) -> String {
    let field_inits = generate_rust_field_inits(s);
    format!(
        r#"// Generated by hddsgen --example
// DDS Publisher using HDDS

use {crate_name}::{full_type};
use hdds::{{Participant, QoS, TransportMode}};
use std::{{thread, time::{{Duration, Instant}}}};

/// How long to wait for a subscriber to match before giving up.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);

fn main() -> hdds::Result<()> {{
    // The SDK applies the domain rule: HDDS_DOMAIN_ID when set and valid
    // (0..=232), else 0. A malformed value makes build() return an error.
    let participant = Participant::builder("{struct_name}Example")
        .with_transport(TransportMode::UdpMulticast)
        .build()?;
    let domain_id = participant.domain_id();
    println!("[HDDS] Domain {{}} | UDP multicast | Ready", domain_id);

    let qos = QoS::reliable().keep_last(10);
    let topic = participant.topic::<{struct_name}>("{struct_name}Topic")?;
    let writer = topic.writer().qos(qos).build()?;

    // A sample written before discovery completes is never delivered.
    let deadline = Instant::now() + DISCOVERY_TIMEOUT;
    while writer.matched_reader_count() == 0 {{
        if Instant::now() >= deadline {{
            eprintln!(
                "[PUB] No subscriber matched on domain {{}} within {{:?}}; start the subscriber first",
                domain_id, DISCOVERY_TIMEOUT
            );
            std::process::exit(1);
        }}
        thread::sleep(Duration::from_millis(50));
    }}
    println!("[PUB] Subscriber matched");

    let sample_count: i32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    for i in 0..sample_count {{
        let msg = {struct_name} {{
{field_inits}        }};

        writer.write(&msg)?;
        println!("[PUB] {{:?}}", msg);

        thread::sleep(Duration::from_secs(1));
    }}

    Ok(())
}}
"#,
        crate_name = crate_name,
        full_type = full_type,
        struct_name = struct_name,
        field_inits = field_inits,
    )
}

fn generate_rust_subscriber(crate_name: &str, full_type: &str, struct_name: &str) -> String {
    format!(
        r#"// Generated by hddsgen --example
// DDS Subscriber using HDDS

use {crate_name}::{full_type};
use hdds::{{Participant, QoS, TransportMode}};
use std::{{thread, time::{{Duration, Instant}}}};

/// Exit when no sample has arrived for this long.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

fn main() -> hdds::Result<()> {{
    // The SDK applies the domain rule: HDDS_DOMAIN_ID when set and valid
    // (0..=232), else 0. A malformed value makes build() return an error.
    let participant = Participant::builder("{struct_name}Example")
        .with_transport(TransportMode::UdpMulticast)
        .build()?;
    let domain_id = participant.domain_id();

    println!("[HDDS] Domain {{}} | UDP multicast | Ready", domain_id);

    let qos = QoS::reliable().keep_last(10);
    let topic = participant.topic::<{struct_name}>("{struct_name}Topic")?;
    let reader = topic.reader().qos(qos).build()?;

    let sample_count: i32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let mut received = 0;
    let mut last_sample = Instant::now();
    while received < sample_count {{
        if let Some(msg) = reader.take()? {{
            received += 1;
            last_sample = Instant::now();
            println!("[SUB] Received: {{:?}}", msg);
        }} else if last_sample.elapsed() >= IDLE_TIMEOUT {{
            println!("[SUB] No sample for {{:?}}, giving up", IDLE_TIMEOUT);
            break;
        }} else {{
            thread::sleep(Duration::from_millis(100));
        }}
    }}

    println!("[SUB] Received {{}}/{{}} samples", received, sample_count);
    Ok(())
}}
"#,
        crate_name = crate_name,
        full_type = full_type,
        struct_name = struct_name,
    )
}

#[allow(clippy::option_if_let_else)] // Complex else branch; map_or_else would hurt readability
fn generate_rust_field_inits(s: &Struct) -> String {
    use crate::types::{IdlType, PrimitiveType};

    let mut inits = String::new();
    for f in &s.fields {
        // Check for @default annotation first
        let init = if let Some(default_val) = f.get_default() {
            convert_default_to_rust(default_val, &f.field_type)
        } else {
            match &f.field_type {
                IdlType::BoundedString { .. } => "\"hello\".to_string()".to_string(),
                IdlType::Primitive(p) => match p {
                    PrimitiveType::Boolean => "false".to_string(),
                    PrimitiveType::Char | PrimitiveType::WChar => "'A'".to_string(),
                    PrimitiveType::Long | PrimitiveType::Int32 => "i + 42".to_string(),
                    PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => {
                        "(i + 42) as u32".to_string()
                    }
                    PrimitiveType::Float => "3.14".to_string(),
                    PrimitiveType::Double | PrimitiveType::LongDouble => "3.14159".to_string(),
                    PrimitiveType::String | PrimitiveType::WString => {
                        "\"hello\".to_string()".to_string()
                    }
                    _ => "Default::default()".to_string(),
                },
                IdlType::Named(_) | IdlType::Array { .. } | IdlType::Map { .. } => {
                    "Default::default()".to_string()
                }
                IdlType::Sequence { .. } => "vec![]".to_string(),
            }
        };
        inits.push_str(&format!("            {}: {},\n", f.name, init));
    }
    inits
}

/// Convert IDL @default value to Rust syntax
fn convert_default_to_rust(value: &str, ty: &crate::types::IdlType) -> String {
    use crate::types::{IdlType, PrimitiveType};
    match ty {
        IdlType::Primitive(p) => match p {
            PrimitiveType::Boolean => match value.to_ascii_lowercase().as_str() {
                "true" => "true".to_string(),
                "false" => "false".to_string(),
                _ => value.to_string(),
            },
            PrimitiveType::Char | PrimitiveType::WChar => {
                if value.starts_with('\'') {
                    value.to_string()
                } else if value.len() == 1 {
                    format!("'{}'", value)
                } else {
                    format!("'{}'", value.chars().next().unwrap_or('?'))
                }
            }
            PrimitiveType::String | PrimitiveType::WString => {
                if value.starts_with('"') && value.ends_with('"') {
                    format!("{}.to_string()", value)
                } else {
                    format!("\"{}\".to_string()", value)
                }
            }
            PrimitiveType::Float => format!("{}f32", value.trim_end_matches('f')),
            PrimitiveType::Double | PrimitiveType::LongDouble => {
                format!("{}f64", value.trim_end_matches('d'))
            }
            PrimitiveType::Octet | PrimitiveType::UInt8 => format!("{}u8", value),
            PrimitiveType::Int8 => format!("{}i8", value),
            PrimitiveType::Short | PrimitiveType::Int16 => format!("{}i16", value),
            PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => format!("{}u16", value),
            PrimitiveType::Long | PrimitiveType::Int32 => format!("{}i32", value),
            PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => format!("{}u32", value),
            PrimitiveType::LongLong | PrimitiveType::Int64 => format!("{}i64", value),
            PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => format!("{}u64", value),
            _ => value.to_string(),
        },
        // All other types (Named, Sequence, Array, Map) pass through as-is
        _ => value.to_string(),
    }
}

// ============================================================================
// C Project Generation
// ============================================================================

/// Generate a complete C project
#[must_use]
pub fn generate_c_project(
    ast: &IdlFile,
    types_code: &str,
    idl_path: &str,
    build_system: BuildSystem,
) -> GeneratedProject {
    let mut project = GeneratedProject::new();
    let base_name = idl_basename(idl_path);
    let base_lower = to_snake_case(&base_name);

    // Find first struct for examples
    let (first_struct, module_path) = find_first_struct_with_path(&ast.definitions, Vec::new())
        .unwrap_or_else(|| {
            static DUMMY: LazyLock<Struct> = LazyLock::new(|| Struct {
                name: "Message".to_string(),
                fields: vec![],
                annotations: vec![],
                base_struct: None,
                extensibility: None,
            });
            (&*DUMMY, vec![])
        });

    // The C backend names a type with its module path (`m_T`) and its
    // functions with that name lowercased (`m_t_encode_cdr2_le`).
    let c_name = if module_path.is_empty() {
        first_struct.name.clone()
    } else {
        format!("{}_{}", module_path.join("_"), first_struct.name)
    };
    let struct_name = &c_name;
    let func_prefix = to_c_ident(struct_name);

    // 1. Header file (types)
    project.add_file(format!("{base_lower}.h"), types_code.to_string());

    // 2. Publisher source
    project.add_file(
        "publisher.c",
        generate_c_publisher(&base_lower, struct_name, &func_prefix, first_struct),
    );

    // 3. Subscriber source
    project.add_file(
        "subscriber.c",
        generate_c_subscriber(&base_lower, struct_name, &func_prefix),
    );

    // 4. Build system file
    match build_system {
        BuildSystem::Cmake => {
            project.add_file("CMakeLists.txt", generate_c_cmake(&base_lower));
        }
        BuildSystem::Make | BuildSystem::Cargo => {
            // Make is default for C, Cargo falls back to Make
            project.add_file("Makefile", generate_c_makefile(&base_lower));
        }
    }

    project
}

fn generate_c_publisher(
    header_name: &str,
    struct_name: &str,
    func_prefix: &str,
    s: &Struct,
) -> String {
    let field_inits = generate_c_field_inits(s);
    format!(
        r#"/* Generated by hddsgen --example */
/* DDS Publisher using HDDS C API */

/* For usleep() and strdup() on POSIX systems */
#define _DEFAULT_SOURCE

#include "{header_name}.h"
#include <hdds.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#include <windows.h>
#define sleep_ms(ms) Sleep(ms)
#else
#include <unistd.h>
#define sleep_ms(ms) usleep((ms) * 1000)
#endif

/* How long to wait for a subscriber to match before giving up. */
#define EXAMPLE_DISCOVERY_TIMEOUT_MS 30000u

int main(int argc, char* argv[]) {{
    /* The SDK applies the domain rule: HDDS_DOMAIN_ID when set and valid (0..232), else 0. */
    struct HddsParticipant* participant = hdds_participant_create("{struct_name}Example");
    if (!participant) {{
        fprintf(stderr, "Failed to create participant (HDDS_DOMAIN_ID: unset or empty is 0, else decimal 0..232)\n");
        return 1;
    }}
    unsigned domain_id = hdds_participant_domain_id(participant);
    printf("Starting {struct_name} Publisher (domain %u)...\n", domain_id);

    struct HddsDataWriter* writer = hdds_writer_create(participant, "{struct_name}Topic");
    if (!writer) {{
        fprintf(stderr, "Failed to create writer\n");
        hdds_participant_destroy(participant);
        return 1;
    }}

    /* A sample written before discovery completes is never delivered. */
    if (hdds_writer_wait_for_matched_readers(writer, 1, EXAMPLE_DISCOVERY_TIMEOUT_MS) != HDDS_OK) {{
        fprintf(stderr, "No subscriber matched on domain %u within %u s; start the subscriber first\n",
                domain_id, EXAMPLE_DISCOVERY_TIMEOUT_MS / 1000u);
        hdds_writer_destroy(writer);
        hdds_participant_destroy(participant);
        return 1;
    }}
    printf("Subscriber matched.\n");

    int sample_count = 10;
    if (argc > 1) {{
        sample_count = atoi(argv[1]);
    }}

    for (int i = 0; i < sample_count; ++i) {{
        /* Create message */
        {struct_name} msg;
        memset(&msg, 0, sizeof(msg));
{field_inits}
        /* Serialize to CDR2 and publish */
        uint8_t buffer[4096];
        int len = {func_prefix}_encode_cdr2_le(&msg, buffer, sizeof(buffer));
        if (len < 0) {{
            fprintf(stderr, "Encode failed!\n");
            continue;
        }}

        hdds_writer_write(writer, buffer, (size_t)len);
        printf("[SEND] Sample %d/%d (%d bytes)\n", i + 1, sample_count, len);

        sleep_ms(1000);
    }}

    hdds_writer_destroy(writer);
    hdds_participant_destroy(participant);
    printf("Publisher finished.\n");
    return 0;
}}
"#,
        header_name = header_name,
        struct_name = struct_name,
        func_prefix = func_prefix,
        field_inits = field_inits,
    )
}

fn generate_c_subscriber(header_name: &str, struct_name: &str, func_prefix: &str) -> String {
    format!(
        r#"/* Generated by hddsgen --example */
/* DDS Subscriber using HDDS C API */

/* For usleep() on POSIX systems */
#define _DEFAULT_SOURCE

#include "{header_name}.h"
#include <hdds.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#include <windows.h>
#define sleep_ms(ms) Sleep(ms)
#else
#include <unistd.h>
#define sleep_ms(ms) usleep((ms) * 1000)
#endif

/* Exit when no sample has arrived for this long. */
#define EXAMPLE_IDLE_TIMEOUT_MS 30000u

int main(int argc, char* argv[]) {{
    /* The SDK applies the domain rule: HDDS_DOMAIN_ID when set and valid (0..232), else 0. */
    struct HddsParticipant* participant = hdds_participant_create("{struct_name}Example");
    if (!participant) {{
        fprintf(stderr, "Failed to create participant (HDDS_DOMAIN_ID: unset or empty is 0, else decimal 0..232)\n");
        return 1;
    }}
    unsigned domain_id = hdds_participant_domain_id(participant);
    printf("Starting {struct_name} Subscriber (domain %u)...\n", domain_id);
    printf("(Start this BEFORE the publisher)\n");

    struct HddsDataReader* reader = hdds_reader_create(participant, "{struct_name}Topic");
    if (!reader) {{
        fprintf(stderr, "Failed to create reader\n");
        hdds_participant_destroy(participant);
        return 1;
    }}

    struct HddsWaitSet* waitset = hdds_waitset_create();
    const struct HddsStatusCondition* cond = hdds_reader_get_status_condition(reader);
    hdds_waitset_attach_status_condition(waitset, cond);

    int sample_count = 10;
    if (argc > 1) {{
        sample_count = atoi(argv[1]);
    }}

    printf("Waiting for %d samples...\n", sample_count);

    int received = 0;
    unsigned idle_ms = 0;
    while (received < sample_count && idle_ms < EXAMPLE_IDLE_TIMEOUT_MS) {{
        const void* triggered[1];
        size_t triggered_count = 0;
        if (hdds_waitset_wait(waitset, 1000000000LL, triggered, 1, &triggered_count) == HDDS_OK
            && triggered_count > 0) {{
            uint8_t buffer[4096];
            size_t len;
            idle_ms = 0;
            while (hdds_reader_take(reader, buffer, sizeof(buffer), &len) == HDDS_OK) {{
                {struct_name} msg;
                memset(&msg, 0, sizeof(msg));
                /* Strings and sequences are placed in this arena. */
                uint8_t storage[4096];
                hdds_x2c_arena arena;
                hdds_x2c_arena_init(&arena, storage, sizeof(storage));
                if ({func_prefix}_decode_cdr2_le_alloc(&msg, buffer, len, hdds_x2c_arena_alloc, &arena) >= 0) {{
                    printf("[RECV] Sample %d/%d\n", ++received, sample_count);
                }}
            }}
        }} else {{
            idle_ms += 1000;
        }}
    }}

    if (received < sample_count) {{
        printf("No sample for %u s, giving up.\n", EXAMPLE_IDLE_TIMEOUT_MS / 1000u);
    }}

    hdds_waitset_destroy(waitset);
    hdds_reader_destroy(reader);
    hdds_participant_destroy(participant);
    printf("Subscriber finished. Received %d/%d samples.\n", received, sample_count);
    return 0;
}}
"#,
        header_name = header_name,
        struct_name = struct_name,
        func_prefix = func_prefix,
    )
}

fn generate_c_makefile(base_name: &str) -> String {
    // The generated project must BUILD as generated. An earlier version gave neither an
    // include path for hdds.h nor a library (its hint named `-lhdds`, while the C library
    // is `hdds_c`), so `make` stopped on "hdds.h: No such file or directory". This mirrors
    // the Makefile of the documented C tutorial, so the generated project and the
    // documented one agree.
    format!(
        r"# Generated by hddsgen --example
# Path to your HDDS checkout: override it if this project does not sit next to it,
#   make HDDS=/path/to/hdds
# and build the C library there first:
#   cargo build --release -p hdds-c
HDDS ?= ../hdds

CC ?= gcc
CFLAGS ?= -Wall -Wextra -O2 -std=c11 -I$(HDDS)/sdk/c/include
LDLIBS ?= $(HDDS)/target/release/libhdds_c.a -lpthread -ldl -lm

.PHONY: all clean

all: publisher subscriber

publisher: publisher.c {base_name}.h
	$(CC) $(CFLAGS) -o $@ publisher.c $(LDLIBS)

subscriber: subscriber.c {base_name}.h
	$(CC) $(CFLAGS) -o $@ subscriber.c $(LDLIBS)

clean:
	rm -f publisher subscriber
",
        base_name = base_name,
    )
}

fn generate_c_cmake(base_name: &str) -> String {
    format!(
        r"# Generated by hddsgen --example --build-system=cmake
cmake_minimum_required(VERSION 3.16)
project({base_name}_example C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)

# Find HDDS C SDK
# Pass -DCMAKE_PREFIX_PATH=/path/to/hdds/sdk/cmake when configuring
find_package(hdds REQUIRED)

# Publisher executable
add_executable({base_name}_publisher publisher.c)
target_include_directories({base_name}_publisher PRIVATE ${{CMAKE_CURRENT_SOURCE_DIR}})
target_link_libraries({base_name}_publisher PRIVATE hdds::hdds)

# Subscriber executable
add_executable({base_name}_subscriber subscriber.c)
target_include_directories({base_name}_subscriber PRIVATE ${{CMAKE_CURRENT_SOURCE_DIR}})
target_link_libraries({base_name}_subscriber PRIVATE hdds::hdds)
",
        base_name = base_name,
    )
}

fn generate_c_field_inits(s: &Struct) -> String {
    use crate::types::{IdlType, PrimitiveType};

    let mut inits = String::new();
    for f in &s.fields {
        let init = match &f.field_type {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Char | PrimitiveType::WChar => {
                    format!("        msg.{} = 'A';\n", f.name)
                }
                PrimitiveType::Long | PrimitiveType::Int32 => {
                    format!("        msg.{} = i + 42;\n", f.name)
                }
                PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => {
                    format!("        msg.{} = (uint32_t)(i + 42);\n", f.name)
                }
                PrimitiveType::Float => format!("        msg.{} = 3.14f;\n", f.name),
                PrimitiveType::Double | PrimitiveType::LongDouble => {
                    format!("        msg.{} = 3.14159;\n", f.name)
                }
                PrimitiveType::String => {
                    format!("        msg.{} = strdup(\"hello\");\n", f.name)
                }
                // Boolean and all other numeric types default to 0
                _ => format!("        msg.{} = 0;\n", f.name),
            },
            _ => String::new(),
        };
        inits.push_str(&init);
    }
    inits
}

// ============================================================================
// Python Project Generation
// ============================================================================

/// Generate a complete Python project
#[must_use]
pub fn generate_python_project(
    ast: &IdlFile,
    types_code: &str,
    idl_path: &str,
) -> GeneratedProject {
    let mut project = GeneratedProject::new();
    let base_name = idl_basename(idl_path);
    let module_name = to_snake_case(&base_name);

    // Find first struct for examples
    let (first_struct, path) = find_first_struct_with_path(&ast.definitions, Vec::new())
        .unwrap_or_else(|| {
            static DUMMY: LazyLock<Struct> = LazyLock::new(|| Struct {
                name: "Message".to_string(),
                fields: vec![],
                annotations: vec![],
                base_struct: None,
                extensibility: None,
            });
            (&*DUMMY, vec![])
        });

    let struct_name = &first_struct.name;
    let import_path = if path.is_empty() {
        struct_name.clone()
    } else {
        format!(
            "{}.{}",
            path.iter()
                .map(|s| to_snake_case(s))
                .collect::<Vec<_>>()
                .join("."),
            struct_name
        )
    };

    // 1. Types module
    project.add_file(format!("{module_name}.py"), types_code.to_string());

    // 2. Publisher script
    project.add_file(
        "publisher.py",
        generate_python_publisher(&module_name, &import_path, struct_name, first_struct),
    );

    // 3. Subscriber script
    project.add_file(
        "subscriber.py",
        generate_python_subscriber(&module_name, &import_path, struct_name),
    );

    // 4. requirements.txt
    project.add_file("requirements.txt", generate_python_requirements());

    project
}

fn generate_python_publisher(
    module_name: &str,
    import_path: &str,
    struct_name: &str,
    s: &Struct,
) -> String {
    let field_kwargs = generate_python_field_kwargs(s);
    // `from m import A.B` is not Python: import the first segment, use the full path.
    let import_head = import_path.split('.').next().unwrap_or(import_path);
    format!(
        r#"#!/usr/bin/env python3
# Generated by hddsgen --example
# DDS Publisher using the HDDS Python SDK

import sys
import time

import hdds
from {module_name} import {import_head}

# How long to wait for a subscriber to match before giving up.
DISCOVERY_TIMEOUT_S = 30.0

def main():
    # The SDK applies the domain rule: HDDS_DOMAIN_ID when set and valid
    # (0..232), else 0. A malformed value makes the constructor raise.
    with hdds.Participant("{struct_name}Example") as participant:
        domain_id = participant.domain_id
        print(f"[HDDS] Domain {{domain_id}} | UDP multicast | Ready")

        qos = hdds.QoS.reliable().history_depth(10)
        writer = participant.create_writer(
            "{struct_name}Topic",
            {import_path},
            qos=qos,
            type_name={import_path}.type_name(),
        )

        # A sample written before discovery completes is never delivered.
        if not writer.wait_for_readers(1, timeout=DISCOVERY_TIMEOUT_S):
            print(
                f"[PUB] No subscriber matched on domain {{domain_id}} within "
                f"{{DISCOVERY_TIMEOUT_S:.0f}} s; start the subscriber first",
                file=sys.stderr,
            )
            sys.exit(1)
        print("[PUB] Subscriber matched")

        sample_count = int(sys.argv[1]) if len(sys.argv) > 1 else 10

        for i in range(sample_count):
            msg = {import_path}({field_kwargs})
            writer.write_typed(msg)
            print(f"[PUB] {{msg}}")
            time.sleep(1)

if __name__ == "__main__":
    main()
"#,
        module_name = module_name,
        import_path = import_path,
        import_head = import_head,
        struct_name = struct_name,
        field_kwargs = field_kwargs,
    )
}

fn generate_python_subscriber(module_name: &str, import_path: &str, struct_name: &str) -> String {
    let import_head = import_path.split('.').next().unwrap_or(import_path);
    format!(
        r#"#!/usr/bin/env python3
# Generated by hddsgen --example
# DDS Subscriber using the HDDS Python SDK

import sys
import time

import hdds
from {module_name} import {import_head}

# Exit when no sample has arrived for this long.
IDLE_TIMEOUT_S = 30.0

def main():
    # The SDK applies the domain rule: HDDS_DOMAIN_ID when set and valid
    # (0..232), else 0. A malformed value makes the constructor raise.
    with hdds.Participant("{struct_name}Example") as participant:
        domain_id = participant.domain_id
        print(f"[HDDS] Domain {{domain_id}} | UDP multicast | Ready")

        qos = hdds.QoS.reliable().history_depth(10)
        reader = participant.create_reader(
            "{struct_name}Topic",
            qos=qos,
            type_name={import_path}.type_name(),
            msg_type={import_path},
        )

        sample_count = int(sys.argv[1]) if len(sys.argv) > 1 else 10

        received = 0
        last_sample = time.monotonic()
        while received < sample_count:
            msg = reader.take_typed({import_path})
            if msg is not None:
                received += 1
                last_sample = time.monotonic()
                print(f"[SUB] Received: {{msg}}")
            elif time.monotonic() - last_sample >= IDLE_TIMEOUT_S:
                print(f"[SUB] No sample for {{IDLE_TIMEOUT_S:.0f}} s, giving up")
                break
            else:
                time.sleep(0.01)

        print(f"[SUB] Received {{received}}/{{sample_count}} samples")

if __name__ == "__main__":
    main()
"#,
        module_name = module_name,
        import_path = import_path,
        import_head = import_head,
        struct_name = struct_name,
    )
}

fn generate_python_requirements() -> String {
    r"# Generated by hddsgen --example
# The HDDS Python SDK is not on PyPI: build its wheel from sdk/python, and set
# HDDS_LIB_PATH to a directory holding libhdds_c.so.
"
    .to_string()
}

fn generate_python_field_kwargs(s: &Struct) -> String {
    use crate::types::{IdlType, PrimitiveType};

    let mut kwargs = Vec::new();
    for f in &s.fields {
        let init = match &f.field_type {
            IdlType::BoundedString { .. } => "\"hello\"".to_string(),
            IdlType::Primitive(p) => match p {
                PrimitiveType::Boolean => "False".to_string(),
                PrimitiveType::Char | PrimitiveType::WChar => "\"A\"".to_string(),
                PrimitiveType::Long
                | PrimitiveType::Int32
                | PrimitiveType::UnsignedLong
                | PrimitiveType::UInt32 => "i + 42".to_string(),
                PrimitiveType::Float | PrimitiveType::Double | PrimitiveType::LongDouble => {
                    "3.14".to_string()
                }
                PrimitiveType::String | PrimitiveType::WString => "\"hello\"".to_string(),
                _ => "0".to_string(),
            },
            IdlType::Named(_) => "None".to_string(),
            IdlType::Sequence { .. } | IdlType::Array { .. } => "[]".to_string(),
            IdlType::Map { .. } => "{}".to_string(),
        };
        kwargs.push(format!("{}={}", f.name, init));
    }
    kwargs.join(", ")
}

// ============================================================================
// Micro (no_std) Project Generation
// ============================================================================

/// Generate a complete `no_std` Rust project for embedded targets
///
/// # Arguments
/// * `hdds_path` - Optional path to local `hdds-micro` crate. If `None`, uses crates.io version.
#[must_use]
pub fn generate_micro_project(
    ast: &IdlFile,
    types_code: &str,
    idl_path: &str,
    hdds_path: Option<&std::path::Path>,
) -> GeneratedProject {
    let mut project = GeneratedProject::new();
    let base_name = idl_basename(idl_path);
    let crate_name = crate_ident(&base_name);

    // Find first struct for examples
    let (first_struct, module_path) = find_first_struct_with_path(&ast.definitions, Vec::new())
        .unwrap_or_else(|| {
            static DUMMY: LazyLock<Struct> = LazyLock::new(|| Struct {
                name: "Message".to_string(),
                fields: vec![],
                annotations: vec![],
                base_struct: None,
                extensibility: None,
            });
            (&*DUMMY, vec![])
        });

    let struct_name = &first_struct.name;
    let full_type = if module_path.is_empty() {
        struct_name.clone()
    } else {
        // The Rust backend emits `pub mod <IdlName>` verbatim (under
        // allow(non_snake_case)), so the path a generated example imports must use
        // the IDL spelling too. Snake-casing it here produced `use crate::demo::T`
        // against a `pub mod Demo` and the project did not compile.
        format!(
            "{}::{}",
            module_path
                .iter()
                .map(|s| rust_ident(s).into_owned())
                .collect::<Vec<_>>()
                .join("::"),
            struct_name
        )
    };

    // 1. Cargo.toml (no_std compatible)
    project.add_file(
        "Cargo.toml",
        generate_micro_cargo_toml(&crate_name, hdds_path),
    );

    // 2. src/lib.rs (types with no_std)
    project.add_file("src/lib.rs", types_code.to_string());

    // 3. src/bin/sender.rs (UDP example that works on std for testing)
    project.add_file(
        "src/bin/sender.rs",
        generate_micro_sender(&crate_name, &full_type, struct_name, first_struct),
    );

    // 4. src/bin/receiver.rs
    project.add_file(
        "src/bin/receiver.rs",
        generate_micro_receiver(&crate_name, &full_type, struct_name),
    );

    // 5. .cargo/config.toml for cross-compilation
    project.add_file(".cargo/config.toml", generate_micro_cargo_config());

    // 6. README with build instructions
    project.add_file("README.md", generate_micro_readme(&crate_name, &base_name));

    project
}

/// Resolve the path a user gave for a local crate dependency.
///
/// `--hdds-path` accepts either the crate directory or the repository root. A
/// repository root holds a virtual workspace manifest, which cargo cannot use as
/// a dependency, so when `path` has no package manifest and `path/crates/<crate>`
/// exists, that crate directory is used. Otherwise `path` is used unchanged.
fn resolve_local_crate_path(path: &std::path::Path, crate_dir: &str) -> std::path::PathBuf {
    let direct = path.join("Cargo.toml");
    if direct.is_file() {
        let is_virtual = std::fs::read_to_string(&direct)
            .map(|s| !s.contains("[package]"))
            .unwrap_or(false);
        if !is_virtual {
            return path.to_path_buf();
        }
    }
    let nested = path.join("crates").join(crate_dir);
    if nested.join("Cargo.toml").is_file() {
        return nested;
    }
    path.to_path_buf()
}

fn generate_micro_cargo_toml(crate_name: &str, hdds_path: Option<&std::path::Path>) -> String {
    // Determine the hdds-micro dependency line
    let hdds_dep = hdds_path.map_or_else(
        || {
            format!(
                r#"hdds-micro = {{ version = "{}", features = ["std", "heapless"] }}"#,
                crate::codegen::rust_backend::compat_guard::REQUIRED_HDDS_MICRO_VERSION
            )
        },
        |path| {
            let resolved = resolve_local_crate_path(path, "hdds-micro");
            let display_path = resolved.display();
            format!(r#"hdds-micro = {{ path = "{display_path}", features = ["std", "heapless"] }}"#)
        },
    );

    format!(
        r#"# Generated by hddsgen gen micro --example
[workspace]

[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[[bin]]
name = "sender"
path = "src/bin/sender.rs"

[[bin]]
name = "receiver"
path = "src/bin/receiver.rs"

[dependencies]
# hdds-micro provides CDR encoder/decoder and RTPS support
{hdds_dep}
heapless = "0.8"

[profile.release]
opt-level = "s"
lto = true
strip = true
"#,
        crate_name = crate_name,
        hdds_dep = hdds_dep,
    )
}

fn generate_micro_sender(
    crate_name: &str,
    full_type: &str,
    struct_name: &str,
    s: &Struct,
) -> String {
    let field_inits = generate_micro_field_inits(s);
    format!(
        r#"// Generated by hddsgen gen micro --example
// RTPS Sender using hdds-micro

use std::net::UdpSocket;
use std::thread;
use std::time::Duration;

use hdds_micro::cdr::CdrEncoder;
use hdds_micro::rtps::{{RtpsHeader, GuidPrefix, ProtocolVersion, VendorId, EntityId, SequenceNumber}};
use hdds_micro::rtps::submessages::Data;

use {crate_name}::{full_type};

fn main() {{
    let socket = UdpSocket::bind("0.0.0.0:0").expect("Failed to bind");
    let target = std::env::args().nth(1).unwrap_or_else(|| "127.0.0.1:7400".to_string());

    println!("=== {struct_name} RTPS Sender ===");
    println!("Sending to {{}}", target);

    // Generate a unique GUID prefix (in production, derive from MAC address or similar)
    let guid_prefix = GuidPrefix::new([
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
        0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C,
    ]);
    let writer_id = EntityId::new([0x00, 0x00, 0x01, 0x02]); // User DataWriter

    let sample_count: u32 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    for i in 0..sample_count {{
        let msg = {struct_name} {{
{field_inits}        }};

        // Build RTPS packet
        let mut packet = [0u8; 512];
        let mut offset = 0;

        // 1. RTPS Header (20 bytes)
        let header = RtpsHeader::new(ProtocolVersion::RTPS_2_5, VendorId::HDDS, guid_prefix);
        offset += header.encode(&mut packet[offset..]).expect("Encode header failed");

        // 2. DATA submessage header (24 bytes)
        let seq = SequenceNumber::new((i + 1) as i64);
        let data = Data::new(EntityId::UNKNOWN, writer_id, seq);
        let data_header_len = data.encode_header(&mut packet[offset..]).expect("Encode DATA failed");

        // 3. CDR payload
        let payload_offset = offset + data_header_len;
        let mut cdr_buf = [0u8; 256];
        let cdr_bytes = {{
            let mut enc = CdrEncoder::new(&mut cdr_buf);
            msg.encode(&mut enc).expect("Encode CDR failed");
            enc.finish()
        }};
        let cdr_len = cdr_bytes.len();
        packet[payload_offset..payload_offset + cdr_len].copy_from_slice(cdr_bytes);

        // Update DATA octets_to_next (20 fixed + payload)
        // @audit-ok: safe casts - cdr_len bounded by MTU, masks ensure byte range
        let octets_to_next = (20 + cdr_len) as u16;
        packet[offset + 2] = (octets_to_next & 0xff) as u8;
        packet[offset + 3] = ((octets_to_next >> 8) & 0xff) as u8;

        let total_len = payload_offset + cdr_len;
        socket.send_to(&packet[..total_len], &target).expect("Send failed");
        println!("[TX] Sample {{}}/{{}}: {{}} bytes (RTPS)", i + 1, sample_count, total_len);

        thread::sleep(Duration::from_secs(1));
    }}

    println!("Sender finished.");
}}
"#,
        crate_name = crate_name,
        full_type = full_type,
        struct_name = struct_name,
        field_inits = field_inits,
    )
}

fn generate_micro_receiver(crate_name: &str, full_type: &str, struct_name: &str) -> String {
    format!(
        r#"// Generated by hddsgen gen micro --example
// RTPS Receiver using hdds-micro

use std::net::UdpSocket;

use hdds_micro::cdr::CdrDecoder;
use hdds_micro::rtps::RtpsHeader;
use hdds_micro::rtps::submessages::Data;

use {crate_name}::{full_type};

fn main() {{
    let bind_addr = std::env::args().nth(1).unwrap_or_else(|| "0.0.0.0:7400".to_string());

    println!("=== {struct_name} RTPS Receiver ===");
    println!("Listening on {{}}", bind_addr);

    let socket = UdpSocket::bind(&bind_addr).expect("Failed to bind");
    let mut buf = [0u8; 512];
    let mut received = 0u32;

    loop {{
        match socket.recv_from(&mut buf) {{
            Ok((len, src)) => {{
                if len < 20 {{
                    eprintln!("[RX] Packet too small: {{}} bytes", len);
                    continue;
                }}

                // 1. Parse RTPS header (20 bytes)
                let header = match RtpsHeader::decode(&buf[0..20]) {{
                    Ok(h) => h,
                    Err(e) => {{
                        eprintln!("[RX] Invalid RTPS header: {{:?}}", e);
                        continue;
                    }}
                }};

                // 2. Parse DATA submessage (starts at offset 20)
                if len < 44 {{
                    eprintln!("[RX] Packet too small for DATA: {{}} bytes", len);
                    continue;
                }}

                let (data, payload_offset) = match Data::decode(&buf[20..len]) {{
                    Ok(d) => d,
                    Err(e) => {{
                        eprintln!("[RX] Invalid DATA submessage: {{:?}}", e);
                        continue;
                    }}
                }};

                // 3. Decode CDR payload
                let payload_start = 20 + payload_offset;
                if payload_start >= len {{
                    eprintln!("[RX] No payload in packet");
                    continue;
                }}

                let mut dec = CdrDecoder::new(&buf[payload_start..len]);
                match {struct_name}::decode(&mut dec) {{
                    Ok(msg) => {{
                        received += 1;
                        println!("[RX] Sample {{}} from {{}}: {{}} bytes (RTPS seq={{}})",
                            received, src, len, data.writer_sn.value());
                        println!("     GUID: {{:?}}", header.guid_prefix);
                        println!("     Data: {{:?}}", msg);
                    }}
                    Err(e) => {{
                        eprintln!("[RX] CDR decode error: {{:?}} ({{}} bytes payload)", e, len - payload_start);
                    }}
                }}
            }}
            Err(e) => {{
                eprintln!("[RX] Recv error: {{}}", e);
            }}
        }}
    }}
}}
"#,
        crate_name = crate_name,
        full_type = full_type,
        struct_name = struct_name,
    )
}

fn generate_micro_cargo_config() -> String {
    r#"# Cross-compilation targets for embedded
# Uncomment the target you want to use

# Pi Zero 2 W (64-bit ARM)
[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"

# Pi Zero W v1 (32-bit ARM, static linking)
[target.arm-unknown-linux-musleabihf]
linker = "rust-lld"
rustflags = ["-C", "link-self-contained=yes", "-C", "target-feature=+crt-static"]

# ESP32 (Xtensa) - requires esp-rs toolchain
# [target.xtensa-esp32-none-elf]
# linker = "xtensa-esp32-elf-gcc"
"#
    .to_string()
}

fn generate_micro_readme(crate_name: &str, base_name: &str) -> String {
    format!(
        r"# {base_name} - no_std DDS Types

Generated by `hddsgen gen micro --example`

## Building

```bash
cargo build --release
```

## Running examples

Terminal 1 (receiver):
```bash
cargo run --release --bin receiver
```

Terminal 2 (sender):
```bash
cargo run --release --bin sender 127.0.0.1:5555
```

## Cross-compiling for Pi Zero 2 W (aarch64)

```bash
cargo build --release --target aarch64-unknown-linux-gnu --bin sender
scp target/aarch64-unknown-linux-gnu/release/sender pi@<pi-ip>:/tmp/
```

## Cross-compiling for Pi Zero W v1 (armv6, static)

```bash
cargo build --release --target arm-unknown-linux-musleabihf --bin receiver
scp target/arm-unknown-linux-musleabihf/release/receiver pi@<pi-ip>:/tmp/
```

## Project structure

```
{crate_name}/
|-- Cargo.toml
|-- .cargo/
|   `-- config.toml      # Cross-compilation settings
|-- src/
|   |-- lib.rs           # Generated types (no_std compatible)
|   `-- bin/
|       |-- sender.rs    # UDP sender example
|       `-- receiver.rs  # UDP receiver example
`-- README.md
```
",
        base_name = base_name,
        crate_name = crate_name,
    )
}

fn generate_micro_field_inits(s: &Struct) -> String {
    use crate::types::{IdlType, PrimitiveType};

    let mut inits = String::new();
    for f in &s.fields {
        let init = match &f.field_type {
            IdlType::BoundedString { wide: false, .. } => {
                "heapless::String::try_from(\"hello\").unwrap()".to_string()
            }
            IdlType::Primitive(p) => match p {
                PrimitiveType::Boolean => "false".to_string(),
                PrimitiveType::Char | PrimitiveType::WChar => "'A'".to_string(),
                PrimitiveType::Long | PrimitiveType::Int32 => "i as i32 + 42".to_string(),
                PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => "i + 42".to_string(),
                PrimitiveType::Float => "3.14".to_string(),
                PrimitiveType::Double | PrimitiveType::LongDouble => "3.14159".to_string(),
                PrimitiveType::String | PrimitiveType::WString => {
                    "heapless::String::try_from(\"hello\").unwrap()".to_string()
                }
                PrimitiveType::UInt8 | PrimitiveType::Octet => "42u8".to_string(),
                PrimitiveType::UInt16 | PrimitiveType::UnsignedShort => "42u16".to_string(),
                PrimitiveType::UInt64 | PrimitiveType::UnsignedLongLong => "42u64".to_string(),
                PrimitiveType::Int8 => "42i8".to_string(),
                PrimitiveType::Int16 | PrimitiveType::Short => "42i16".to_string(),
                PrimitiveType::Int64 | PrimitiveType::LongLong => "42i64".to_string(),
                PrimitiveType::Void | PrimitiveType::Fixed { .. } => "0".to_string(),
            },
            IdlType::Named(n) => format!("{}::default()", n),
            IdlType::BoundedString { wide: true, .. } | IdlType::Sequence { .. } => {
                "heapless::Vec::new()".to_string()
            }
            IdlType::Array { size, .. } => format!("[Default::default(); {}]", size),
            IdlType::Map { .. } => "/* Map not supported in no_std */".to_string(),
        };
        inits.push_str(&format!("            {}: {},\n", f.name, init));
    }
    inits
}

// ============================================================================
// C Micro (Header-Only) Project Generation
// ============================================================================

/// Generate a complete C-micro project for embedded MCUs
#[must_use]
pub fn generate_c_micro_project(
    ast: &IdlFile,
    types_code: &str,
    idl_path: &str,
) -> GeneratedProject {
    let mut project = GeneratedProject::new();
    let base_name = idl_basename(idl_path);

    // Find first struct for examples
    let (first_struct, _module_path) = find_first_struct_with_path(&ast.definitions, Vec::new())
        .unwrap_or_else(|| {
            static DUMMY: LazyLock<Struct> = LazyLock::new(|| Struct {
                name: "Message".to_string(),
                fields: vec![],
                annotations: vec![],
                base_struct: None,
                extensibility: None,
            });
            (&*DUMMY, vec![])
        });

    let struct_name = &first_struct.name;

    // 1. include/generated_types.h (generated types)
    project.add_file("include/generated_types.h", types_code.to_string());

    // 2. include/hdds_micro_cdr.h (runtime CDR encoder/decoder)
    project.add_file("include/hdds_micro_cdr.h", C_MICRO_CDR_HEADER.to_string());

    // 3. src/sender.c (example sender)
    project.add_file(
        "src/sender.c",
        generate_c_micro_sender(struct_name, first_struct),
    );

    // 4. src/receiver.c (example receiver)
    project.add_file("src/receiver.c", generate_c_micro_receiver(struct_name));

    // 5. Makefile
    project.add_file("Makefile", generate_c_micro_makefile(&base_name));

    // 6. README.md
    project.add_file(
        "README.md",
        generate_c_micro_readme(&base_name, struct_name),
    );

    project
}

/// Embedded CDR runtime header (bundled with generated code)
const C_MICRO_CDR_HEADER: &str = include_str!("../../sdk/c-micro/include/hdds_micro_cdr.h");

#[allow(clippy::too_many_lines)] // Complete C sender template generation
fn generate_c_micro_sender(struct_name: &str, s: &Struct) -> String {
    use crate::types::{IdlType, PrimitiveType};

    // Generate field initializers
    let mut field_inits = String::new();
    for f in &s.fields {
        // Check for bounded string first
        if matches!(f.field_type, IdlType::BoundedString { wide: false, .. }) {
            field_inits.push_str(&format!(
                "    strncpy(msg.{}, \"hello\", sizeof(msg.{})-1);\n",
                f.name, f.name
            ));
            continue;
        }

        let init = match &f.field_type {
            // `string<N>` is handled above; `wstring<N>` is left as is.
            IdlType::BoundedString { .. } => continue,
            IdlType::Primitive(p) => match p {
                PrimitiveType::Char => "'A'".to_string(),
                PrimitiveType::Long
                | PrimitiveType::Int32
                | PrimitiveType::UnsignedLong
                | PrimitiveType::UInt32
                | PrimitiveType::UInt8
                | PrimitiveType::Octet
                | PrimitiveType::UInt16
                | PrimitiveType::UnsignedShort
                | PrimitiveType::Int8
                | PrimitiveType::Int16
                | PrimitiveType::Short => "42".to_string(),
                PrimitiveType::Float => "3.14f".to_string(),
                PrimitiveType::Double | PrimitiveType::LongDouble => "3.14159".to_string(),
                PrimitiveType::UInt64 | PrimitiveType::UnsignedLongLong => "42ULL".to_string(),
                PrimitiveType::Int64 | PrimitiveType::LongLong => "42LL".to_string(),
                PrimitiveType::String | PrimitiveType::WString => {
                    format!(
                        "strncpy(msg.{}, \"hello\", sizeof(msg.{})-1)",
                        f.name, f.name
                    )
                }
                // Boolean and other types default to 0
                _ => "0".to_string(),
            },
            IdlType::Named(name) => format!("{{0}} /* {} */", name),
            IdlType::Sequence { .. } => "{{0}} /* seq */".to_string(),
            IdlType::Array { .. } => "{{0}} /* arr */".to_string(),
            IdlType::Map { .. } => "{{0}} /* map */".to_string(),
        };

        if matches!(
            f.field_type,
            IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString)
        ) {
            field_inits.push_str(&format!("    {};\n", init));
        } else {
            field_inits.push_str(&format!("    msg.{} = {};\n", f.name, init));
        }
    }

    let struct_lower = to_snake_case(struct_name);
    let struct_upper = struct_name.to_uppercase();

    format!(
        r#"/**
 * @file sender.c
 * @brief Example UDP sender for {struct_name}
 * @note Generated by hddsgen gen c-micro --example
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#include <winsock2.h>
#pragma comment(lib, "ws2_32.lib")
#else
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <unistd.h>
#endif

#include "generated_types.h"

int main(int argc, char** argv) {{
    const char* target_ip = argc > 1 ? argv[1] : "127.0.0.1";
    int target_port = argc > 2 ? atoi(argv[2]) : 5555;
    int count = argc > 3 ? atoi(argv[3]) : 10;

#ifdef _WIN32
    WSADATA wsa;
    WSAStartup(MAKEWORD(2, 2), &wsa);
#endif

    int sock = socket(AF_INET, SOCK_DGRAM, 0);
    if (sock < 0) {{
        perror("socket");
        return 1;
    }}

    struct sockaddr_in dest;
    memset(&dest, 0, sizeof(dest));
    dest.sin_family = AF_INET;
    dest.sin_port = htons(target_port);
    inet_pton(AF_INET, target_ip, &dest.sin_addr);

    printf("=== {struct_name} Sender ===\n");
    printf("Sending to %s:%d\n", target_ip, target_port);

    for (int i = 0; i < count; ++i) {{
        {struct_name} msg;
        memset(&msg, 0, sizeof(msg));

{field_inits}
        uint8_t buffer[{struct_upper}_ENCODED_SIZE_MAX];
        hdds_cdr_t cdr;
        hdds_cdr_init(&cdr, buffer, sizeof(buffer));

        int32_t rc = {struct_lower}_encode(&msg, &cdr);
        if (rc != HDDS_CDR_OK) {{
            fprintf(stderr, "Encode error: %d\n", rc);
            continue;
        }}

        sendto(sock, (const char*)buffer, cdr.pos, 0,
               (struct sockaddr*)&dest, sizeof(dest));

        printf("[TX] Sample %d/%d: %u bytes\n", i + 1, count, cdr.pos);

#ifdef _WIN32
        Sleep(1000);
#else
        sleep(1);
#endif
    }}

#ifdef _WIN32
    closesocket(sock);
    WSACleanup();
#else
    close(sock);
#endif

    printf("Sender finished.\n");
    return 0;
}}
"#,
        struct_name = struct_name,
        struct_upper = struct_upper,
        struct_lower = struct_lower,
        field_inits = field_inits,
    )
}

fn generate_c_micro_receiver(struct_name: &str) -> String {
    let struct_lower = to_snake_case(struct_name);

    format!(
        r#"/**
 * @file receiver.c
 * @brief Example UDP receiver for {struct_name}
 * @note Generated by hddsgen gen c-micro --example
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#include <winsock2.h>
#pragma comment(lib, "ws2_32.lib")
#else
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <unistd.h>
#endif

#include "generated_types.h"

int main(int argc, char** argv) {{
    int bind_port = argc > 1 ? atoi(argv[1]) : 5555;

#ifdef _WIN32
    WSADATA wsa;
    WSAStartup(MAKEWORD(2, 2), &wsa);
#endif

    int sock = socket(AF_INET, SOCK_DGRAM, 0);
    if (sock < 0) {{
        perror("socket");
        return 1;
    }}

    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port = htons(bind_port);
    addr.sin_addr.s_addr = INADDR_ANY;

    if (bind(sock, (struct sockaddr*)&addr, sizeof(addr)) < 0) {{
        perror("bind");
        return 1;
    }}

    printf("=== {struct_name} Receiver ===\n");
    printf("Listening on port %d\n", bind_port);

    uint8_t buffer[1024];
    uint32_t received = 0;

    while (1) {{
        struct sockaddr_in src;
        socklen_t src_len = sizeof(src);

        ssize_t len = recvfrom(sock, (char*)buffer, sizeof(buffer), 0,
                               (struct sockaddr*)&src, &src_len);
        if (len < 0) {{
            perror("recvfrom");
            continue;
        }}

        hdds_cdr_t cdr;
        hdds_cdr_init(&cdr, buffer, (uint32_t)len);

        {struct_name} msg;
        int32_t rc = {struct_lower}_decode(&msg, &cdr);

        if (rc == HDDS_CDR_OK) {{
            received++;
            char src_ip[INET_ADDRSTRLEN];
            inet_ntop(AF_INET, &src.sin_addr, src_ip, sizeof(src_ip));
            printf("[RX] Sample %u from %s:%d (%zd bytes)\n",
                   received, src_ip, ntohs(src.sin_port), len);
            /* Print fields here if needed */
        }} else {{
            fprintf(stderr, "[RX] Decode error: %d (%zd bytes)\n", rc, len);
        }}
    }}

#ifdef _WIN32
    closesocket(sock);
    WSACleanup();
#else
    close(sock);
#endif

    return 0;
}}
"#,
        struct_name = struct_name,
        struct_lower = struct_lower,
    )
}

fn generate_c_micro_makefile(base_name: &str) -> String {
    format!(
        r#"# Generated by hddsgen gen c-micro --example
# Makefile for {base_name}

CC ?= gcc
CFLAGS ?= -Wall -Wextra -O2 -I./include
LDFLAGS ?=

# For ARM embedded targets (uncomment as needed):
# CC = arm-none-eabi-gcc
# CFLAGS = -mcpu=cortex-m4 -mthumb -Os -I./include

SRCS = src/sender.c src/receiver.c
OBJS_SENDER = src/sender.o
OBJS_RECEIVER = src/receiver.o

.PHONY: all clean

all: sender receiver

sender: $(OBJS_SENDER)
	$(CC) -o $@ $^ $(LDFLAGS)

receiver: $(OBJS_RECEIVER)
	$(CC) -o $@ $^ $(LDFLAGS)

%.o: %.c
	$(CC) $(CFLAGS) -c -o $@ $<

clean:
	rm -f sender receiver src/*.o

# Cross-compile for STM32 (example)
stm32:
	$(MAKE) CC=arm-none-eabi-gcc CFLAGS="-mcpu=cortex-m4 -mthumb -Os -I./include -DSTM32"

# Cross-compile for AVR (example)
avr:
	$(MAKE) CC=avr-gcc CFLAGS="-mmcu=atmega328p -Os -I./include -DAVR"
"#,
        base_name = base_name,
    )
}

fn generate_c_micro_readme(base_name: &str, struct_name: &str) -> String {
    format!(
        r#"# {base_name} - C Micro DDS Types

Generated by `hddsgen gen c-micro --example`

## Overview

This project contains header-only C code for CDR serialization of DDS types.
Compatible with C89/C99, suitable for embedded MCUs (STM32, AVR, PIC, ESP32).

## Building

```bash
make
```

## Running

Terminal 1 (receiver):
```bash
./receiver 5555
```

Terminal 2 (sender):
```bash
./sender 127.0.0.1 5555
```

## Cross-compiling for ARM

```bash
# STM32 (Cortex-M4)
make stm32

# AVR (ATmega328P)
make avr

# Raspberry Pi
make CC=arm-linux-gnueabihf-gcc
```

## Project structure

```
{base_name}/
|-- include/
|   |-- generated_types.h     # Generated types ({struct_name}, etc.)
|   `-- hdds_micro_cdr.h      # CDR encoder/decoder runtime
|-- src/
|   |-- sender.c              # UDP sender example
|   `-- receiver.c            # UDP receiver example
|-- Makefile
`-- README.md
```

## Using in your embedded project

1. Copy `include/generated_types.h` and `include/hdds_micro_cdr.h` to your project
2. Include the header in your code:
   ```c
   #include "generated_types.h"

   void send_data(void) {{
       {struct_name} msg = {{...}};
       uint8_t buffer[{struct_name}_ENCODED_SIZE_MAX];
       hdds_cdr_t cdr;
       hdds_cdr_init(&cdr, buffer, sizeof(buffer));
       {struct_name_lower}_encode(&msg, &cdr);
       // Send buffer[0..cdr.pos] over your transport
   }}
   ```

## Interoperability

The generated CDR encoding follows DDS-XTypes 1.3 (XCDR1 / XCDR2) and is
read by:
- hdds (Rust DDS)
- hddsgen gen micro (no_std Rust)
- any DDS implementation that follows the same specification
"#,
        base_name = base_name,
        struct_name = struct_name,
        struct_name_lower = to_snake_case(struct_name),
    )
}
