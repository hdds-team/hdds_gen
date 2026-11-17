// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Publisher and subscriber of the generated `--example` projects.
//!
//! Three rules hold in every language that generates a live pub/sub pair:
//!
//! - the domain is the one the SDK reports for the participant (the SDK reads
//!   `HDDS_DOMAIN_ID`, else 0; the example does not parse it), and it is printed;
//! - the publisher waits for a matched reader before its first write, and
//!   gives up with a message and a non-zero status after a timeout;
//! - the subscriber exits after an idle timeout, reporting its sample count.
//!
//! The Python project follows the same three rules through the HDDS Python SDK.

#![allow(clippy::expect_used, clippy::panic)]

use std::collections::HashMap;

use hddsgen::codegen::examples_project::{
    generate_c_project, generate_cpp_project, generate_micro_project, generate_python_project,
    generate_rust_project, BuildSystem,
};
use hddsgen::Parser;

const IDL: &str = "module hdds_samples { struct HelloWorld { long id; string message; }; };\n";
const IDL_PATH: &str = "HelloWorld.idl";

fn ast() -> hddsgen::ast::IdlFile {
    let mut p = Parser::try_new(IDL).expect("lexer");
    p.parse().expect("parse")
}

fn file<'a>(files: &'a HashMap<String, String>, name: &str) -> &'a str {
    files
        .get(name)
        .unwrap_or_else(|| panic!("missing {name}; have {:?}", files.keys()))
}

/// Byte offset of `needle` in `hay`; panics if absent.
fn at(hay: &str, needle: &str) -> usize {
    hay.find(needle)
        .unwrap_or_else(|| panic!("`{needle}` not found"))
}

#[test]
fn rust_domain_comes_from_the_sdk_and_is_printed() {
    let p = generate_rust_project(&ast(), "types", IDL_PATH, None);
    for name in ["src/bin/publisher.rs", "src/bin/subscriber.rs"] {
        let src = file(&p.files, name);
        // The SDK applies the domain rule; the example never parses the variable.
        assert!(!src.contains("std::env::var"), "{name}");
        assert!(!src.contains(".domain_id(domain_id)"), "{name}");
        assert!(!src.contains(".domain_id(0)"), "{name}");
        assert!(
            src.contains("let domain_id = participant.domain_id();"),
            "{name}"
        );
        assert!(
            src.contains("[HDDS] Domain {} | UDP multicast | Ready"),
            "{name}"
        );
    }
}

#[test]
fn rust_publisher_waits_for_a_match_before_the_first_write() {
    let p = generate_rust_project(&ast(), "types", IDL_PATH, None);
    let src = file(&p.files, "src/bin/publisher.rs");
    let wait = at(src, "while writer.matched_reader_count() == 0 {");
    let timeout = at(src, "std::process::exit(1);");
    let write = at(src, "writer.write(&msg)?;");
    assert!(wait < write, "wait must precede the first write");
    assert!(
        wait < timeout && timeout < write,
        "timeout exits inside the wait"
    );
}

#[test]
fn rust_subscriber_exits_on_idle_timeout_with_count() {
    let p = generate_rust_project(&ast(), "types", IDL_PATH, None);
    let src = file(&p.files, "src/bin/subscriber.rs");
    assert!(src.contains("const IDLE_TIMEOUT: Duration = Duration::from_secs(30);"));
    assert!(src.contains("else if last_sample.elapsed() >= IDLE_TIMEOUT {"));
    assert!(src.contains("println!(\"[SUB] Received {}/{} samples\""));
}

#[test]
fn cpp_domain_comes_from_the_sdk_and_is_printed() {
    let p = generate_cpp_project(&ast(), "types", IDL_PATH, BuildSystem::Cmake);
    for name in ["HelloWorldPublisher.cxx", "HelloWorldSubscriber.cxx"] {
        let src = file(&p.files, name);
        assert!(!src.contains("std::getenv"), "{name}");
        assert!(!src.contains("kDefaultDomainId"), "{name}");
        assert!(
            src.contains(
                r#"hdds::Participant participant("HelloWorldExample", hdds::TransportMode::UdpMulticast);"#
            ),
            "{name}"
        );
        assert!(
            src.contains("const std::uint32_t domain_id = participant.domain_id();"),
            "{name}"
        );
        assert!(
            src.contains(r#"<< "[HDDS] Domain " << domain_id"#),
            "{name}"
        );
    }
}

#[test]
fn cpp_publisher_waits_for_a_match_before_the_first_write() {
    let p = generate_cpp_project(&ast(), "types", IDL_PATH, BuildSystem::Cmake);
    let src = file(&p.files, "HelloWorldPublisher.cxx");
    let wait = at(
        src,
        "while (writer.get_publication_matched_status().current_count == 0) {",
    );
    let timeout = at(src, "return 1;");
    let write = at(src, "writer.write(msg);");
    assert!(
        wait < timeout && timeout < write,
        "wait, bounded exit, then the first write"
    );
}

#[test]
fn cpp_subscriber_exits_on_idle_timeout_with_count() {
    let p = generate_cpp_project(&ast(), "types", IDL_PATH, BuildSystem::Cmake);
    let src = file(&p.files, "HelloWorldSubscriber.cxx");
    assert!(src.contains("constexpr std::chrono::seconds kIdleTimeout{30};"));
    assert!(src.contains("now() - last_sample >= kIdleTimeout"));
    assert!(src.contains(r#"<< "[SUB] Received " << received << "/" << sample_count"#));
}

#[test]
fn c_domain_comes_from_the_sdk_and_is_printed() {
    let p = generate_c_project(&ast(), "types", IDL_PATH, BuildSystem::Cmake);
    for name in ["publisher.c", "subscriber.c"] {
        let src = file(&p.files, name);
        assert!(!src.contains("getenv"), "{name}");
        assert!(!src.contains("strtoul"), "{name}");
        assert!(
            src.contains("unsigned domain_id = hdds_participant_domain_id(participant);"),
            "{name}"
        );
        assert!(src.contains("(domain %u)"), "{name}");
    }
}

#[test]
fn c_publisher_waits_for_a_match_before_the_first_write() {
    let p = generate_c_project(&ast(), "types", IDL_PATH, BuildSystem::Cmake);
    let src = file(&p.files, "publisher.c");
    let wait = at(
        src,
        "hdds_writer_wait_for_matched_readers(writer, 1, EXAMPLE_DISCOVERY_TIMEOUT_MS) != HDDS_OK",
    );
    let timeout = at(
        src,
        "        return 1;\n    }\n    printf(\"Subscriber matched.",
    );
    let write = at(src, "hdds_writer_write(writer, buffer");
    assert!(
        wait < timeout && timeout < write,
        "wait, bounded exit, then the first write"
    );
}

#[test]
fn c_subscriber_exits_on_idle_timeout_with_count() {
    let p = generate_c_project(&ast(), "types", IDL_PATH, BuildSystem::Cmake);
    let src = file(&p.files, "subscriber.c");
    assert!(src.contains("#define EXAMPLE_IDLE_TIMEOUT_MS 30000u"));
    assert!(src.contains("while (received < sample_count && idle_ms < EXAMPLE_IDLE_TIMEOUT_MS) {"));
    assert!(src.contains(
        r#"printf("Subscriber finished. Received %d/%d samples.\n", received, sample_count);"#
    ));
    // Attach and wait with the calls declared in the SDK header.
    assert!(src.contains("hdds_waitset_attach_status_condition(waitset, cond);"));
    assert!(
        src.contains("hdds_waitset_wait(waitset, 1000000000LL, triggered, 1, &triggered_count)")
    );
}

#[test]
fn python_domain_comes_from_the_sdk_and_is_printed() {
    let p = generate_python_project(&ast(), "types", IDL_PATH);
    for name in ["publisher.py", "subscriber.py"] {
        let src = file(&p.files, name);
        assert!(!src.contains("os.environ"), "{name}");
        assert!(!src.contains("DEFAULT_DOMAIN_ID"), "{name}");
        assert!(!src.contains("domain_id=domain_id"), "{name}");
        assert!(!src.contains("domain_id=0"), "{name}");
        assert!(
            src.contains("hdds.Participant(\"HelloWorldExample\") as participant:"),
            "{name}"
        );
        assert!(
            src.contains("domain_id = participant.domain_id\n"),
            "{name}"
        );
        // A dotted from-import is a syntax error: the first segment is imported.
        assert!(
            src.contains("from hello_world import hdds_samples\n"),
            "{name}"
        );
        assert!(!src.contains("import hdds_samples.HelloWorld"), "{name}");
        assert!(
            src.contains("[HDDS] Domain {domain_id} | UDP multicast | Ready"),
            "{name}"
        );
    }
}

#[test]
fn micro_cargo_requests_the_hdds_micro_workspace_version() {
    let p = generate_micro_project(&ast(), "types", IDL_PATH, None);
    let toml = file(&p.files, "Cargo.toml");
    assert!(
        toml.contains(r#"hdds-micro = { version = "1.5.0", features = ["std", "heapless"] }"#),
        "{toml}"
    );
    assert!(!toml.contains(r#"version = "0.8""#), "{toml}");
}

#[test]
fn python_publisher_waits_for_a_match_before_the_first_write() {
    let p = generate_python_project(&ast(), "types", IDL_PATH);
    let src = file(&p.files, "publisher.py");
    let wait = at(
        src,
        "if not writer.wait_for_readers(1, timeout=DISCOVERY_TIMEOUT_S):",
    );
    let timeout = at(src, "sys.exit(1)");
    let write = at(src, "writer.write_typed(msg)");
    assert!(
        wait < timeout && timeout < write,
        "wait, bounded exit, then the first write"
    );
}

#[test]
fn python_subscriber_exits_on_idle_timeout_with_count() {
    let p = generate_python_project(&ast(), "types", IDL_PATH);
    let src = file(&p.files, "subscriber.py");
    assert!(src.contains("IDLE_TIMEOUT_S = 30.0"));
    assert!(src.contains("elif time.monotonic() - last_sample >= IDLE_TIMEOUT_S:"));
    assert!(src.contains(r#"print(f"[SUB] Received {received}/{sample_count} samples")"#));
}

#[test]
fn python_examples_use_the_sdk_not_commented_out_dds_lines() {
    let p = generate_python_project(&ast(), "types", IDL_PATH);
    for name in ["publisher.py", "subscriber.py"] {
        let src = file(&p.files, name);
        assert!(src.contains("\nimport hdds\n"), "{name}");
        assert!(
            src.contains("hdds.Participant(\"HelloWorldExample\""),
            "{name}"
        );
        assert!(
            src.contains("type_name=hdds_samples.HelloWorld.type_name()"),
            "{name}"
        );
        assert!(!src.contains("# writer.write"), "{name}");
        assert!(!src.contains("# participant ="), "{name}");
    }
}

/// Runs `python3 -c <script>` with `src` on stdin. `None` when python3 cannot be spawned.
fn python_with_stdin(script: &str, src: &str) -> Option<std::process::Output> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("python3")
        .args(["-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(src.as_bytes())
        .expect("write stdin");
    child.wait_with_output().ok()
}

#[test]
fn python_examples_compile() {
    let p = generate_python_project(&ast(), "types", IDL_PATH);
    for name in ["publisher.py", "subscriber.py"] {
        let Some(out) = python_with_stdin(
            "import sys; compile(sys.stdin.read(), 'example.py', 'exec')",
            file(&p.files, name),
        ) else {
            eprintln!("python3 not found; Python example compile test not run");
            return;
        };
        assert!(
            out.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// Flow-insensitive undefined-name check (stdlib `ast` only): every name the
/// example loads must be bound somewhere in the file or be a builtin. It does
/// not model scopes or order, so it catches a dropped import or a typo, not
/// use-before-definition; ruff F821 is the stricter gate.
const UNDEFINED_NAMES: &str = r#"
import ast, builtins, sys
tree = ast.parse(sys.stdin.read(), "example.py")
bound = set(dir(builtins))
for n in ast.walk(tree):
    if isinstance(n, (ast.Import, ast.ImportFrom)):
        bound.update((a.asname or a.name).split(".")[0] for a in n.names)
    elif isinstance(n, (ast.FunctionDef, ast.ClassDef)):
        bound.add(n.name)
    elif isinstance(n, ast.arg):
        bound.add(n.arg)
    elif isinstance(n, ast.Name) and not isinstance(n.ctx, ast.Load):
        bound.add(n.id)
used = {n.id for n in ast.walk(tree) if isinstance(n, ast.Name)}
print(",".join(sorted(used - bound)))
"#;

#[test]
fn python_examples_load_no_undefined_names() {
    let p = generate_python_project(&ast(), "types", IDL_PATH);
    for name in ["publisher.py", "subscriber.py"] {
        let Some(out) = python_with_stdin(UNDEFINED_NAMES, file(&p.files, name)) else {
            eprintln!("python3 not found; Python undefined-name test not run");
            return;
        };
        assert!(out.status.success(), "{name}: checker failed");
        let missing = String::from_utf8_lossy(&out.stdout);
        assert_eq!(missing.trim(), "", "{name}: undefined names");
    }
}
