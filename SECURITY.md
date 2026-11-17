# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 2.0.x   | :white_check_mark: |

## Reporting a Vulnerability

We take security vulnerabilities seriously. If you discover a security issue, please report it responsibly.

### How to Report

**Please do NOT report security vulnerabilities through public GitHub issues.**

Instead, please send an email to: **contact@hdds.io**

Include the following information:

- Type of vulnerability (e.g., code injection via IDL, path traversal in output)
- Full path of the affected source file(s)
- Location of the affected code (tag/branch/commit or direct URL)
- Step-by-step instructions to reproduce the issue
- Proof-of-concept IDL file (if applicable)
- Impact assessment

### What to Expect

- **Acknowledgment**: Within 48 hours of your report
- **Initial Assessment**: Within 7 days
- **Resolution Timeline**: Depends on severity (critical: 7 days, high: 30 days, medium: 90 days)
- **Public Disclosure**: Coordinated with you after fix is available

### Safe Harbor

We consider security research conducted in good faith to be authorized. We will not pursue legal action against researchers who:

- Make a good faith effort to avoid privacy violations and data destruction
- Only interact with accounts they own or with explicit permission
- Do not exploit vulnerabilities beyond what is necessary to demonstrate the issue
- Report vulnerabilities promptly and do not publicly disclose before resolution

## Security Considerations

### IDL Parsing

- hdds_gen parses untrusted IDL input files
- The parser is designed to handle malformed input gracefully
- Recursive structures are bounded to prevent stack overflow

### Code Generation

- Generated file paths are validated to prevent path traversal
- Output directories are created with appropriate permissions
- No arbitrary code execution from IDL content

### Dependencies

- We regularly update dependencies to patch known vulnerabilities
- Use `cargo audit` to check for known issues

## Acknowledgments

We thank all security researchers who help keep hdds_gen secure. Contributors who report valid security issues will be acknowledged here (with permission).
