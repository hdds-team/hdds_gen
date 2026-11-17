<p align="center">
  <img src="hdds-logo.png" alt="HDDS" width="160">
</p>
# hdds_gen

[![CI](https://git.hdds.io/hdds/hdds_gen/actions/workflows/ci.yml/badge.svg)](https://git.hdds.io/hdds/hdds_gen/actions)
[![License](https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.87%2B-orange.svg)](https://www.rust-lang.org/)
[![IDL](https://img.shields.io/badge/OMG%20IDL-4.2-green.svg)](https://www.omg.org/spec/IDL/)
[![Backends](https://img.shields.io/badge/backends-7-blue.svg)](#code-generation-backends)

High-assurance OMG IDL 4.2 parser and multi-language code generator for DDS (Data Distribution Service) applications.

## Overview

`hdds_gen` is a Rust-based toolchain that parses OMG IDL 4.2 files and generates serialization code for multiple target languages. It provides CDR2 (Common Data Representation version 2) encode/decode implementations suitable for DDS middleware interoperability.

**Key capabilities:**

- Full OMG IDL 4.2 parser with preprocessor support
- Code generation for 7 target backends (cpp, rust, python, c, micro, c-micro, typescript)
- Semantic validation with detailed diagnostics
- Pretty-printer for IDL formatting
- CLI tool with subcommands for parsing, generation, validation, and formatting

## Supported IDL Types

### Primitive Types

| IDL Type | Description |
|----------|-------------|
| `boolean` | Boolean value |
| `char`, `wchar` | 8-bit and wide characters |
| `octet` | 8-bit unsigned |
| `short`, `unsigned short` | 16-bit signed/unsigned |
| `long`, `unsigned long` | 32-bit signed/unsigned |
| `long long`, `unsigned long long` | 64-bit signed/unsigned |
| `float`, `double`, `long double` | Floating-point types |
| `string`, `wstring` | Unbounded strings |
| `string<N>`, `wstring<N>` | Bounded strings |
| `int8`, `int16`, `int32`, `int64` | Fixed-width signed integers |
| `uint8`, `uint16`, `uint32`, `uint64` | Fixed-width unsigned integers |
| `fixed<D,S>` | Fixed-point decimal (D digits, S scale) |
| `void` | Void type (for operations) |

### Constructed Types

| Type | Description |
|------|-------------|
| `struct` | Aggregated data structure with optional inheritance |
| `enum` | Enumeration with optional explicit values |
| `union` | Discriminated union with case labels |
| `typedef` | Type alias with annotation support |
| `bitset` | Packed bit fields with explicit widths |
| `bitmask` | Named flag constants |
| `const` | Constant definitions |
| `module` | Namespace scoping |
| `forward declaration` | Forward struct/union declarations |
| `@annotation` | Custom annotation declarations |

### Container Types

| Type | Description |
|------|-------------|
| `sequence<T>` | Unbounded sequence |
| `sequence<T, N>` | Bounded sequence (max N elements) |
| `T[N]` | Fixed-size array |
| `map<K, V>` | Unbounded map |
| `map<K, V, N>` | Bounded map (max N entries) |

### Interfaces (Feature-Gated)

By default (no `--features interfaces`), hddsgen does not understand the
IDL `interface` and `exception` keywords: a file using them fails to parse,
with a diagnostic pointing at the offending keyword and naming the feature
flag.

With `--features interfaces`, the parser accepts `interface` and
`exception` definitions, validates them (duplicate operations/attributes,
unknown `raises` targets, `oneway` return/params rules), and the pretty-printer
can round-trip them back to IDL text. No backend generates code for them
either way: `interface` and `exception` definitions are parsed and
validated but silently skipped by every code generator (C, C++, Python,
Rust, TypeScript, micro). This feature exists for parsing/tooling use
cases (e.g. validating or reformatting IDL that contains interfaces), not
for code generation.

| Type | Description |
|------|-------------|
| `interface` | Interface with operations and attributes |
| `exception` | Exception type declarations |
| `oneway` | One-way operations |
| `in/out/inout` | Parameter direction qualifiers |
| `raises` | Exception specifications |

## Supported Annotations

### DDS/XTYPES Standard Annotations

| Annotation | Target | Description |
|------------|--------|-------------|
| `@key` | Field | Marks field as part of topic key |
| `@optional` | Field | Field may be absent |
| `@id(N)` | Field | Explicit member ID |
| `@autoid(SEQUENTIAL\|HASH)` | Type | Auto-generate member IDs |
| `@extensibility(FINAL\|APPENDABLE\|MUTABLE)` | Type | Type evolution policy |
| `@final` | Type | Shorthand for FINAL extensibility |
| `@appendable` | Type | Shorthand for APPENDABLE extensibility |
| `@mutable` | Type | Shorthand for MUTABLE extensibility |
| `@must_understand` | Field | Reader must understand this field |
| `@nested` | Type | Nested type (no topic) |
| `@external` | Field | Member stored outside the enclosing object (OMG DDS-XTypes 1.3 7.3.1.2.1.4); a `Box` in Rust |
| `@default(value)` | Field | Default VALUE of the member (OMG IDL 4.2 8.3.3.1); honoured by the Rust builder |
| `@position(N)` | Bitset/Bitmask | Explicit bit position |
| `@bit_bound(N)` | Enum/Bitmask | Maximum bit width |
| `@data_representation(XCDR1\|XCDR2)` | Type | Wire format selection |
| `@non_serialized` | Field | Exclude from serialization |

A struct, union, enum or bitmask without an extensibility annotation is
generated as APPENDABLE, the default DDS-XTypes 1.3 specifies (7.3.1.2.1.8:
such a type "shall be considered appendable"), and `hddsgen gen` prints an
info line naming those types. hddsgen 1.x generated them as
FINAL (Cyclone DDS idlc's default too): pass `--default-extensibility final`
for that layout, or `mutable` (an enum or a bitmask then stays
APPENDABLE); passing the option also silences the info line. See
`CHANGELOG.md` for what this changes on the wire.

Not accepted: `@value(N)` (on a member or an enumerator) and `@default_literal` on an
enumerator (OMG IDL 4.2 8.3.2.3). `hddsgen check` refuses them. Give an enumerator an
explicit value with `enum E { A = 0, B = 1 }`.

### Documentation Annotations

| Annotation | Description |
|------------|-------------|
| `@unit("...")` | Unit of measurement |
| `@min(N)` | Minimum value constraint |
| `@max(N)` | Maximum value constraint |
| `@range(min=N, max=M)` | Value range constraint |
| `@verbatim(...)` | Language-specific code injection |

### Interface Annotations

Parsed only in a build with `--features interfaces` (see above).

| Annotation | Description |
|------------|-------------|
| `@service` | Mark interface as service |
| `@ami` | Asynchronous method invocation |

### Custom Annotations

User-defined annotations via `@annotation` declarations with typed members and default values.

## Code Generation Backends

### Rust (`rust`)

- Idiomatic Rust structs with `#[derive(Debug, Clone, PartialEq)]`
- XCDR1 and XCDR2 (`encode_xcdr1_le` / `encode_xcdr2_le` and the
  `Cdr2Encode` / `Cdr2Decode` traits) as DDS-XTypes 1.3 7.4.3 specifies:
  FINAL, APPENDABLE (DHEADER) and MUTABLE (EMHEADER / parameter list)
  types, base members first, enumerations at their `@bit_bound` width;
  checked byte for byte against Cyclone DDS 0.11, `@mutable` unions
  against vectors built from the specification text
  (`tests/fixtures/xtypes_datapath/spec_vectors.py`) and maps against Fast
  CDR 2.3.3 (`tests/xtypes_datapath_rust.rs`)
- Type evolution (7.4.1.2.3): a reader reads the data of a writer whose
  MUTABLE or APPENDABLE type has fewer members, the missing ones at their
  default (`__hdds_default()`: zero, empty, the first enumerator, `None`
  when optional)
- Decoding refuses a sequence or map count before reserving storage when
  the storage of all collections of the decode would exceed
  `MAX_ALLOC_RATIO` (64) bytes of memory per byte of the body, or, for
  elements that take no byte on the wire (an empty structure), when the
  decode would read more than `MAX_ZERO_WIRE_ELEMENTS` (1024) such values
  in all; both are constants of the generated `__hdds_cdr` module. An
  element can take far more memory than wire bytes: an absent `@optional`
  member, a union with a large branch, an APPENDABLE element from an older
  writer (its missing members take their default)
- `try_xcdr1_size` / `try_xcdr2_size` give the encoded size or the error
  that makes a value unencodable; `max_xcdrN_size` returns the size up to
  that error
- Complete `TypeObject`s (`get_type_object`, `nested_type_objects`),
  hashing as Cyclone DDS idlc's (union case ids aside: Cyclone numbers
  them from 0, the specification from 1): typedefs are alias
  objects, enumerations and bitmasks carry their extensibility flags,
  `@try_construct` is in the member flags. The generated code needs an
  `hdds` whose `TypeObject` model has those fields
- `Option<T>` for `@optional` fields
- `Vec<T>` for sequences (`Vec<char>` for `sequence<char>`; `string<N>` is
  a `String`), `HashMap<K,V>` for maps
- Module namespacing preserved
- An un-annotated type is APPENDABLE unless `--default-extensibility` says
  otherwise (see the annotations above)

### C++ (`cpp`)

- C++17 compatible headers
- STL containers: `std::vector` for sequences (bounded ones too, the bound
  enforced on encode and decode; `sequence<char>` / `sequence<wchar>` are
  sequences, an element count without NUL), `std::array` for arrays and
  for `string<N>` / `wstring<N>` (N characters, NUL-terminated when
  shorter), `std::map`, `std::string` / `std::wstring`, `std::optional`
  for `@optional` members (`populate_from_json` fills them, `null` resets
  them)
- Inline XCDR2 little-endian encode/decode (`encode_cdr2_le[_at]`,
  `decode_cdr2_le[_at]`) following DDS-XTypes 1.3 7.4.3.5.3: FINAL,
  APPENDABLE (DHEADER) and MUTABLE (DHEADER, EMHEADER1) structures,
  inheritance, unions, `@bit_bound` enumerations and bitmasks, wchar /
  wstring as UTF-16. Every sample of the Cyclone DDS 0.11 corpus
  (`tests/fixtures/xtypes_datapath/`) encodes to Cyclone's bytes and
  Cyclone's bytes decode to the sample (`tests/xtypes_datapath_cpp.rs`).
  `@mutable` unions (which Cyclone does not support) encode to the
  specification's form and decode it and another form a writer may use,
  both built from the specification text.
  Type evolution (7.4.1.2.3): a reader reads the data of a writer whose
  MUTABLE or APPENDABLE type has fewer members, the missing ones at their
  default (zero, empty, the first enumerator, no value when optional).
  A decoded union keeps its discriminator (several labels, `default`).
  No XCDR1
- Decoder bounds (`tests/xtypes_datapath_cpp_hostile.rs`, under
  AddressSanitizer): the storage of every sequence and map of a decode is
  charged to one budget, the larger of `CDR_MAX_ZERO_WIRE_ELEMENTS` bytes
  and `CDR_MAX_ALLOC_RATIO` (default 64) times the bytes of the body,
  however the collections nest (nested structures and unions decode
  through the `decode_cdr2_le_at` overload taking the `cdr2::Budget` of
  the enclosing decode); beyond what is left the decode returns -1 before
  anything is reserved. Elements are counted at `sizeof` (a map entry at
  `sizeof(K) + sizeof(V)` and four pointers of node links). Valid data can
  need far more storage than wire bytes: absent `@optional` members (one
  byte on the wire, the whole `std::optional` in memory), unions with a
  large branch, APPENDABLE elements from an older writer (a DHEADER on the
  wire, the defaulted trailing members in memory), empty containers: raise
  it for such types. Elements that take no byte on the wire (an empty
  FINAL structure: one byte each) are also charged, at `sizeof`, to the
  `CDR_MAX_ZERO_WIRE_ELEMENTS` bytes (default 1024) of the whole decode. Both are `#ifndef` defines of the
  generated header (the same as the C backend's): define them before
  including it
- `--fastdds-compat`: FastDDS-style accessors (`x()` / `x(v)`), members
  `m_`-prefixed and `protected`
- Namespace wrapping via `--namespace-cpp`

### C (`c`)

- Header-only output for C99 (default), C11 or C89 (`--c-standard c89`:
  compiles with `-std=c89 -pedantic-errors`; still includes `<stdint.h>`)
- XCDR2 little-endian encode/decode (DDS-XTypes 1.3 7.4.3): FINAL,
  APPENDABLE (DHEADER) and MUTABLE (DHEADER + EMHEADER1) types, optional
  members, enums and bitmasks at their `@bit_bound` width, checked byte for
  byte against Cyclone DDS 0.11 (`tests/xtypes_datapath_c.rs`);
  `@mutable` unions encode to the specification's form and decode it and
  another form a writer may use, both built from the specification text
- Type evolution (DDS-XTypes 1.3 7.4.1.2.3): a member of a MUTABLE type
  absent from the data, or of an APPENDABLE type past the DHEADER's end,
  takes its default (Table 9: zero, `""`, an empty sequence, the first
  enumeration literal, nested defaults; an optional member is absent)
- `string<N>` is `char[N+1]`, `wstring<N>` `wchar_t[N+1]`; an unbounded
  `string` / `wstring` is a `char*` / `wchar_t*`; every sequence
  (`sequence<char>` included: a sequence on the wire, not a string) is
  `{ T* data; uint32_t len; }`, a map `{ K* keys; V* values; uint32_t len; }`
- Decoding pointer holders: the decoder never writes through a pointer the
  caller set. `T_decode_cdr2_le_alloc(out, src, len, alloc, ctx)` takes an
  allocator (`void* (*)(void* ctx, size_t size)`, memory aligned as by
  `malloc`) from which the decoder obtains the storage of every unbounded
  string, sequence and map, sized from bytes it has already bounded by the
  buffer (a count can never exceed the bytes left), so a hostile length is
  refused (`-CDR_UNEXPECTED_EOF`, `-CDR_INVALID_DATA`) or the allocation
  fails (`-CDR_OUT_OF_MEMORY`), never an overflow
  (`tests/xtypes_datapath_c_hostile.rs`, under AddressSanitizer). The
  header provides a bump allocator over a caller buffer
  (`hdds_x2c_arena_init` / `hdds_x2c_arena_alloc`: no free, reset it to
  reuse the buffer; the natural choice, since nothing has to be freed one
  by one). With `malloc`, free each pointer holder yourself; after a failed
  decode, the value is partially filled. `T_decode_cdr2_le` (no allocator)
  refuses a value holding such a member, an empty sequence or map excepted.
  An allocator rather than a capacity next to each pointer: a capacity
  cannot describe the elements of a `sequence<string>` or the strings of an
  array before the count is known, an allocator sized by the decoder can.
  `CDR_MAX_STRING` (default 1 MiB, overridable) still bounds an unbounded
  string. Every storage request of a decode (the elements of a sequence
  or map, an unbounded string) is charged to one budget: the larger of
  `CDR_MAX_ZERO_WIRE_ELEMENTS` bytes and `CDR_MAX_ALLOC_RATIO` (default 64)
  times the bytes of the body, however the collections nest; a request
  beyond what is left is refused (`-CDR_OUT_OF_MEMORY`, before the
  allocator is called). Valid data can need far more storage than wire
  bytes: absent `@optional` members (one byte on the wire, the whole member
  in C), unions with a large branch, APPENDABLE elements from an older
  writer (a DHEADER on the wire, the defaulted trailing members in C):
  raise it for such types. The storage of elements that take no byte on
  the wire (an empty FINAL structure: one byte each) is also charged to
  the `CDR_MAX_ZERO_WIRE_ELEMENTS` bytes (default 1024) of the whole decode
  and refused (`-CDR_INVALID_DATA`) beyond them. Both are `#ifndef` defines of the generated header.
- Modules: a type declared in a module is named with its full module path
  (`module a { module inner { struct T; }; };` is `a_inner_T`, its codec
  `a_inner_t_encode_cdr2_le`, its enumerators `A_INNER_E_X`, its
  descriptor functions `a_inner_t_*`, reporting the type name
  `a::inner::T`); references are resolved by the IDL scoping rules
  (innermost enclosing module outwards), so two modules declaring the same
  type name give two types. Two IDL names with the same C name
  (`a::b_T`, `a_b::T`) are refused (`tests/xtypes_datapath_c_modules.rs`)
- Type descriptors for runtime introspection

### Python (`python`)

- Python 3.7+ with `@dataclass` decorators
- Type hints via `typing` module
- `IntEnum` for enumerations
- XCDR2 `encode_cdr2_le()` / `decode_cdr2_le()` methods (DDS-XTypes 1.3
  7.4.3.5.3: DHEADER for `@appendable` and `@mutable`, EMHEADER1 member
  headers with spec member ids, `@bit_bound` enum and bitmask widths),
  checked byte for byte against Cyclone DDS 0.11
  (`tests/xtypes_datapath_python.rs`), `@mutable` unions against vectors
  built from the specification text; no XCDR1 codec
- Modules: a type `a::b::T` is the class `a_b_T`, also reachable as
  `a.b.T` (one class per module); its unqualified name `T` is bound too
  when no other declaration of the file has that name. `type_name()` is
  the scoped name `a::b::T`. References follow the IDL scoping rules, so
  two modules may declare types of the same name.
- `string<N>` is a `str`; `sequence<char>` / `sequence<wchar>` are lists
  of character codes, serialized as sequences
- Type evolution: a `@mutable` member absent from the data, or an
  `@appendable` member after the DHEADER's end, takes its default; the
  constructors give the defaults of DDS-XTypes 1.3 Table 9 (zero, empty,
  first enumerator, nested defaults)
- Decoder bounds, one budget per decode however its collections nest,
  checked on the count of a sequence or map before any element is built
  (`ValueError`): the values its elements can build beyond the bytes they
  take are charged to the larger of `CDR_MAX_ZERO_WIRE_ELEMENTS` and
  `CDR_MAX_ALLOC_RATIO` (64) times the bytes of the body; the values of
  elements that take no byte on the wire (an empty FINAL structure: one
  each) also to the `CDR_MAX_ZERO_WIRE_ELEMENTS` (1024) of the decode.
  Valid data can build far more values than it has bytes: absent
  `@optional` members and APPENDABLE elements from an older writer (their
  defaults, a whole array), unions with a large branch: raise
  `CDR_MAX_ALLOC_RATIO` for such types. Module globals, the C and C++
  defines' names and defaults (`tests/xtypes_datapath_python.rs`)
- `compute_key()` for `@key` field hashing

### TypeScript (`typescript`)

- Structs become TypeScript `interface` types (a derived structure
  `extends` its base); enums, union types. This is unrelated to the IDL
  `interface`/`exception` keywords described above under "Interfaces
  (Feature-Gated)" -- no backend, including this one, generates code for
  those
- XCDR2 `encode<Type>(obj, buf)` / `decode<Type>(buf)` through the
  `Cdr2Buffer` cursor emitted in each file (DDS-XTypes 1.3 7.4.3.5.3, same
  rules as Python), checked byte for byte against Cyclone DDS 0.11
  (`tests/xtypes_datapath_typescript.rs`, run with
  `node --experimental-transform-types`), `@mutable` unions against
  vectors built from the specification text; no XCDR1 codec
- Modules: every declaration is at the top level under its flat name
  (`a::b::T` is `a_b_T`, `encodea_b_T`, ...); one `namespace` per module,
  nested, re-exports them under their own names (`a.b.T`, `a.b.encodeT`)
- `default<Type>()` gives the DDS-XTypes 1.3 Table 9 default; a reader
  accepts a writer type with fewer members (`@mutable` absent members,
  `@appendable` members after the DHEADER's end take their default)
- Decoder bounds, one budget per decode (per `Cdr2Buffer`) however its
  collections nest, checked on the count of a sequence or map before any
  element is built (an `Error`): the values its elements can build beyond
  the bytes they take are charged to the larger of
  `CDR_MAX_ZERO_WIRE_ELEMENTS` and `CDR_MAX_ALLOC_RATIO` (64) times the
  bytes of the buffer; the values of elements that take no byte on the
  wire (an empty FINAL structure: one each) also to the
  `CDR_MAX_ZERO_WIRE_ELEMENTS` (1024) of the decode. Valid data can build
  far more values than it has bytes: absent `@optional` members and
  APPENDABLE elements from an older writer (their defaults, a whole
  array), unions with a large branch. Exported constants of each
  generated file, the C and C++ defines' names and defaults
  (`tests/xtypes_datapath_typescript.rs`)
- 64-bit integers and bitmasks of 33 to 64 bits are `bigint`;
  `sequence<char>` is a `string[]` serialized as a sequence;
  `computeKey<Type>()` for `@key` field hashing
- The generated code type-checks under `tsc --strict`
  (`generated_typescript_passes_tsc_strict`; it uses `$HDDS_TSC`, else
  `<target dir>/ts/node_modules/.bin/tsc`, else `tsc` on the PATH, and is
  skipped without one: `npm install --prefix target/ts typescript`)

### Micro (`micro`) - no_std Rust

- `#![no_std]` compatible for embedded targets
- Uses `heapless::Vec` and `heapless::String` (`string<N>` is a
  `heapless::String<N>`, `sequence<char>` a `heapless::Vec<u8, _>`)
- Inline CDR encode/decode (no trait dispatch) through `hdds-micro`'s
  cursors: XCDR1 (`PLAIN_CDR`) body, the encapsulation header is the
  caller's; MUTABLE types and `@optional` members as parameter lists;
  checked against Cyclone DDS 0.11, `@mutable` unions against vectors
  built from the specification text (`tests/xtypes_datapath_micro.rs`)
- Type evolution: an absent MUTABLE member, or an APPENDABLE member past
  the end of the data, takes its default (`__hdds_default()`)
- Refuses maps, bitmasks, bitsets and `fixed`
- Configurable buffer sizes
- Target: bare-metal Rust with `hdds-micro` crate

### C-Micro (`c-micro`) - Header-Only C for MCUs

- C99 (the SDK runtime header `sdk/c-micro/include/hdds_micro_cdr.h` uses
  `inline`), no dynamic allocation
- Fixed-size buffers with compile-time bounds (`<TYPE>_ENCODED_SIZE_MAX`)
- XCDR2 data path shared with the C backend: FINAL, APPENDABLE and MUTABLE
  types, checked against Cyclone DDS 0.11 (`tests/xtypes_datapath_c_micro.rs`),
  with the same type evolution rules (missing members at their default)
- Refuses (generation error naming the type) what its holders cannot
  represent: maps, bitsets, `fixed`
- Modules: the full module path prefixes a type's name (`a_inner_T`), as
  in the C backend
- Target: STM32, AVR, PIC, ESP32, any MCU with C compiler

## CLI Usage

```bash
# Install
cargo install --path .

# Parse and validate
hddsgen parse input.idl
hddsgen parse input.idl --pretty      # Pretty-print parsed IDL
hddsgen parse input.idl --json        # JSON diagnostics

# Check (validation only, CI-friendly exit codes)
hddsgen check input.idl
hddsgen check input.idl --json

# Generate code
hddsgen gen rust input.idl -o output.rs
hddsgen gen cpp input.idl -o output.hpp
hddsgen gen c input.idl -o output.h
hddsgen gen python input.idl -o output.py
hddsgen gen micro input.idl -o output.rs
hddsgen gen c-micro input.idl -o output.h

# Generate with namespace (C++)
hddsgen gen cpp input.idl --namespace-cpp MyApp::Types -o output.hpp

# Generate full project with examples
hddsgen gen rust input.idl --example --out-dir ./my_project
hddsgen gen cpp input.idl --example --out-dir ./my_project --build-system cmake

# Format IDL
hddsgen fmt input.idl -o formatted.idl

# Include directories for #include resolution
hddsgen parse main.idl -I ./includes -I /usr/share/idl
```

### Subcommands

| Command | Description |
|---------|-------------|
| `parse` | Parse and validate IDL, optionally pretty-print |
| `gen` | Generate code for target language |
| `check` | Validate only (returns non-zero on error) |
| `fmt` | Reformat IDL via pretty-printer |

### Generation Options

| Option | Description |
|--------|-------------|
| `-o, --out <FILE>` | Output file (stdout if omitted) |
| `--out-dir <DIR>` | Output directory for module files |
| `--namespace-cpp <NS>` | C++ namespace (e.g., `A::B::C`) |
| `--example` | Generate a project with publisher/subscriber examples for `rust`, `cpp`, `c`, `python`, `micro` and `c-micro`; `typescript` generates `types.ts` only |
| `--build-system <TYPE>` | Build system: `cargo`, `cmake`, `make` |
| `--hdds-path <PATH>` | Local crate for `rust` (`hdds`) or `micro` (`hdds-micro`) `--example` projects: the crate directory, or the repository root, which resolves to that crate |
| `-I, --include <DIR>` | Include directory for `#include` resolution |

## Preprocessor

Full C-style preprocessor with:

- `#include "file.idl"` and `#include <file.idl>`
- `#define NAME value` and `#define MACRO(args) body`
- `#ifdef`, `#ifndef`, `#if`, `#elif`, `#else`, `#endif`
- `#undef`
- Cycle detection for include guards
- Macro expansion with function-like macros
- Token pasting (`##`) and stringification (`#`)

## Validation Rules

The validator enforces semantic correctness:

### Struct Rules
- No duplicate field names
- Valid type references
- `@key` only on serializable fields
- Extensibility annotation conflicts

### Enum Rules
- No duplicate enumerator names
- Explicit values within `@bit_bound` limits

### Union Rules
- No duplicate case labels
- At most one `default` case
- Valid discriminator type
- `@default` annotation consistency

### Bitset Rules
- Bit positions must not overlap
- Total width within bounds
- Valid `@position` annotations

### Interface Rules (with feature)
- No duplicate operation names
- No operation/attribute name collisions
- Valid parameter types
- `raises` references existing exceptions
- `oneway` operations must return `void`

### Custom Annotations
- Parameters match declared annotation members
- Required parameters provided

## Project Architecture

```
src/
  lib.rs              # Public API exports
  ast.rs              # Abstract Syntax Tree types
  types.rs            # IDL type system (primitives, annotations)
  token.rs            # Lexer token definitions
  error.rs            # Error types and handling
  preprocessor.rs     # C-style preprocessor (#include, #define, conditionals)

  lexer/              # Lexical analysis
    mod.rs            # Lexer entry point
    scanner.rs        # Character scanning
    numbers.rs        # Numeric literal parsing
    state.rs          # Lexer state machine

  parser/             # Syntax analysis
    mod.rs            # Parser entry point
    annotations.rs    # Annotation parsing
    const_expr.rs     # Constant expression evaluation
    interfaces.rs     # Interface parsing (feature-gated)
    types.rs          # Type parsing
    definitions/      # Definition parsers
      structs.rs
      enums.rs
      unions.rs
      bitsets.rs
      bitmasks.rs
      typedefs.rs
      consts.rs
      module.rs
      forwards.rs

  validate/           # Semantic validation
    mod.rs            # Validation entry point
    engine.rs         # Validation orchestration
    rules/            # Validation rules
      structs.rs
      enums.rs
      unions.rs
      bitsets.rs
      interfaces.rs
    diagnostics.rs    # Diagnostic types
    references.rs     # Reference resolution

  codegen/            # Code generation
    mod.rs            # Backend trait and registry
    rust_backend/     # Rust code generator
    cpp/              # C++ code generator
    c/                # C code generator
    python.rs         # Python code generator
    micro/            # no_std Rust generator
    c_micro/          # Header-only C for MCUs
    examples.rs       # Example code generation
    examples_project.rs  # Full project scaffolding

  pretty/             # Pretty-printer
    mod.rs            # Formatter entry point
    formatter.rs      # Core formatting logic
    structs.rs        # Struct formatting
    enums.rs          # Enum formatting
    unions.rs         # Union formatting
    bitsets.rs        # Bitset formatting
    modules.rs        # Module formatting
    interfaces.rs     # Interface formatting (feature-gated)

  bin/
    hddsgen.rs        # CLI entry point
    hddsgen/
      commands.rs     # Subcommand implementations

tests/                # Integration tests
examples/             # Example IDL files
  canonical/          # Reference test cases
  invalid/            # Expected-failure cases
  include/            # Include resolution tests
  macros/             # Preprocessor tests
  interfaces/         # Interface feature tests
```

## Build and Test

The release build succeeds with Rust 1.87 and fails with Rust 1.86 (`const fn`
use of `Vec::len` and `String::as_str`). `Cargo.toml` declares no `rust-version`.

```bash
# Build
make build              # Debug build
make release            # Release build

# Test
make test               # Unit tests
make validate-ci        # Full CI validation suite

# Code quality
make fmt                # Format code
make clippy             # Run linter
make doc                # Generate documentation

# Install
make install            # Install to ~/.cargo/bin
```

### Feature Flags

| Feature | Description |
|---------|-------------|
| `interfaces` | Enable interface/exception parsing and pretty-printing |

```bash
# Build with interfaces support
cargo build --features interfaces
```

## Examples

### Basic IDL

```idl
@extensibility(APPENDABLE)
struct HelloWorld {
    unsigned long index;
    string message;
};
```

### Advanced IDL

```idl
module Comp {
    @appendable
    struct Msg {
        @key int32_t id;
        @optional string content;
        string<16> name;
        sequence<int32_t, 10> values;
    };

    enum Color { Red = 0, Green = 1, Blue = 2 };

    typedef map<string, int32_t, 100> ConfigMap;

    bitset Flags {
        bitfield<3> mode;
        bitfield<5> value, @position(4);
    };

    bitmask Permissions { Read, Write, Execute };

    union Data switch(int32_t) {
        case 1: int32_t integer;
        default: octet raw;
    };

    const int32_t MAGIC = 42;
};
```

### Generated Rust Usage

```rust
use hdds::{Cdr2Encode, Cdr2Decode};

let msg = Comp::Msg {
    id: 1,
    content: Some("Hello".to_string()),
    name: "test".to_string(),
    values: vec![1, 2, 3],
};

let mut buffer = [0u8; 256];
let size = msg.encode_cdr2_le(&mut buffer)?;
let (decoded, _) = Comp::Msg::decode_cdr2_le(&buffer)?;
assert_eq!(msg, decoded);
```

## Statistics

- 7 code generation backends
- 60+ example IDL files
- Comprehensive validation suite

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

Copyright (c) 2025-2026 naskel.com

## Repository

https://git.hdds.io/hdds/hdds_gen.git
