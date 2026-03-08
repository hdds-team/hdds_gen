#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
# Copyright (c) 2025-2026 naskel.com
#
# Cross-language CDR2 roundtrip test for hdds_gen.
#
# Tests Python, C, and C++ backends:
#   1. Generate types from interop_test.idl
#   2. Encode/decode CDR2 roundtrip in each language
#   3. Cross-check CDR2 bytes between languages
#
# Usage:
#   ./scripts/test-codegen-roundtrip.sh
#
# Exit 0 = all tests passed.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
IDL="$ROOT/tests/fixtures/interop_test.idl"

# --- Colors ---
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

PASS=0
FAIL=0

pass() { echo -e "[${GREEN}PASS${NC}] $1"; PASS=$((PASS + 1)); }
fail() { echo -e "[${RED}FAIL${NC}] $1"; FAIL=$((FAIL + 1)); }
info() { echo -e "[${YELLOW}INFO${NC}] $1"; }

# ---------------------------------------------------------------
# Phase 0: Setup
# ---------------------------------------------------------------
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

info "Temp dir: $TMP"

# Build or locate hddsgen
HDDSGEN="${HDDSGEN:-$ROOT/target/release/hddsgen}"
if [ ! -x "$HDDSGEN" ]; then
    info "Building hddsgen (release)..."
    cargo build --release --bin hddsgen --manifest-path="$ROOT/Cargo.toml" --quiet
fi

if [ ! -x "$HDDSGEN" ]; then
    echo "ERROR: hddsgen not found at $HDDSGEN" >&2
    exit 1
fi

info "Using hddsgen: $HDDSGEN"

# ---------------------------------------------------------------
# Phase 1: Code generation
# ---------------------------------------------------------------
info "Generating code from interop_test.idl..."

"$HDDSGEN" gen python "$IDL" -o "$TMP/interop_types.py"
"$HDDSGEN" gen c      "$IDL" -o "$TMP/interop_types.h"
"$HDDSGEN" gen cpp    "$IDL" -o "$TMP/interop_types.hpp"

pass "Code generation (Python, C, C++)"

# ---------------------------------------------------------------
# Phase 2: Python roundtrip
# ---------------------------------------------------------------
cat > "$TMP/test_roundtrip.py" << 'PYEOF'
import sys, struct
sys.path.insert(0, sys.argv[1])
from interop_types import SensorReading, SensorKind, GeoPoint

msg = SensorReading(
    sensor_id=42,
    kind=SensorKind.PRESSURE,
    value=3.14,
    label="test-sensor",
    timestamp_ns=1700000000000000000,
    history=[1.0, 2.0, 3.0],
    error_code=7,
    location=GeoPoint(latitude=48.8566, longitude=2.3522)
)

buf = msg.encode_cdr2_le()
decoded, nbytes = SensorReading.decode_cdr2_le(buf)

# Validate all fields
errs = []
if decoded.sensor_id != 42:
    errs.append(f"sensor_id: got {decoded.sensor_id}, want 42")
if decoded.kind != SensorKind.PRESSURE:
    errs.append(f"kind: got {decoded.kind}, want PRESSURE")
if struct.pack('<f', decoded.value) != struct.pack('<f', 3.14):
    errs.append(f"value: got {decoded.value}, want 3.14f")
if decoded.label != "test-sensor":
    errs.append(f"label: got {decoded.label!r}, want 'test-sensor'")
if decoded.timestamp_ns != 1700000000000000000:
    errs.append(f"timestamp_ns: got {decoded.timestamp_ns}")
if len(decoded.history) != 3:
    errs.append(f"history len: got {len(decoded.history)}, want 3")
else:
    for i, (got, want) in enumerate(zip(decoded.history, [1.0, 2.0, 3.0])):
        if struct.pack('<f', got) != struct.pack('<f', want):
            errs.append(f"history[{i}]: got {got}, want {want}")
if decoded.error_code != 7:
    errs.append(f"error_code: got {decoded.error_code}, want 7")
if abs(decoded.location.latitude - 48.8566) > 1e-10:
    errs.append(f"latitude: got {decoded.location.latitude}")
if abs(decoded.location.longitude - 2.3522) > 1e-10:
    errs.append(f"longitude: got {decoded.location.longitude}")
if nbytes != len(buf):
    errs.append(f"bytes_read ({nbytes}) != bytes_written ({len(buf)})")

if errs:
    for e in errs:
        print(f"FAIL: {e}", file=sys.stderr)
    sys.exit(1)

# Output hex on stdout
print(buf.hex())
PYEOF

if python3 "$TMP/test_roundtrip.py" "$TMP" > "$TMP/python.hex" 2>"$TMP/python.err"; then
    pass "Python roundtrip"
else
    fail "Python roundtrip"
    cat "$TMP/python.err" >&2
fi

# ---------------------------------------------------------------
# Phase 3: C roundtrip
# ---------------------------------------------------------------

# Find C compiler
CC=""
for cc_candidate in clang gcc; do
    if command -v "$cc_candidate" > /dev/null 2>&1; then
        CC="$cc_candidate"
        break
    fi
done

if [ -z "$CC" ]; then
    info "SKIP: No C compiler found"
else
    cat > "$TMP/test_roundtrip.c" << 'CEOF'
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <math.h>
#include "interop_types.h"

int main(void) {
    /* Build SensorReading with deterministic values */
    float hist[] = {1.0f, 2.0f, 3.0f};

    SensorReading msg;
    memset(&msg, 0, sizeof(msg));
    msg.sensor_id = 42;
    msg.kind = SENSORKIND_PRESSURE;
    msg.value = 3.14f;
    msg.label = (char*)"test-sensor";
    msg.timestamp_ns = 1700000000000000000LL;
    msg.history.data = hist;
    msg.history.len = 3;
    msg.has_error_code = 1;
    msg.error_code = 7;
    msg.location.latitude = 48.8566;
    msg.location.longitude = 2.3522;

    /* Encode */
    uint8_t buf[4096];
    int enc = sensorreading_encode_cdr2_le(&msg, buf, sizeof(buf));
    if (enc < 0) {
        fprintf(stderr, "FAIL: encode returned %d\n", enc);
        return 1;
    }

    /* Decode */
    SensorReading out;
    memset(&out, 0, sizeof(out));
    char label_buf[256];
    out.label = label_buf;
    float hist_out[64];
    out.history.data = hist_out;

    int dec = sensorreading_decode_cdr2_le(&out, buf, (size_t)enc);
    if (dec != enc) {
        fprintf(stderr, "FAIL: decode returned %d, encode was %d\n", dec, enc);
        return 2;
    }

    /* Validate fields */
    int errs = 0;
    if (out.sensor_id != 42) {
        fprintf(stderr, "FAIL: sensor_id = %u, want 42\n", out.sensor_id);
        errs++;
    }
    if (out.kind != SENSORKIND_PRESSURE) {
        fprintf(stderr, "FAIL: kind = %d, want PRESSURE(1)\n", out.kind);
        errs++;
    }
    if (out.value != 3.14f) {
        fprintf(stderr, "FAIL: value = %f, want 3.14\n", (double)out.value);
        errs++;
    }
    if (strcmp(out.label, "test-sensor") != 0) {
        fprintf(stderr, "FAIL: label = '%s', want 'test-sensor'\n", out.label);
        errs++;
    }
    if (out.timestamp_ns != 1700000000000000000LL) {
        fprintf(stderr, "FAIL: timestamp_ns mismatch\n");
        errs++;
    }
    if (out.history.len != 3) {
        fprintf(stderr, "FAIL: history.len = %u, want 3\n", out.history.len);
        errs++;
    } else {
        if (out.history.data[0] != 1.0f) { fprintf(stderr, "FAIL: history[0]\n"); errs++; }
        if (out.history.data[1] != 2.0f) { fprintf(stderr, "FAIL: history[1]\n"); errs++; }
        if (out.history.data[2] != 3.0f) { fprintf(stderr, "FAIL: history[2]\n"); errs++; }
    }
    if (!out.has_error_code) {
        fprintf(stderr, "FAIL: has_error_code = 0, want 1\n");
        errs++;
    }
    if (out.error_code != 7) {
        fprintf(stderr, "FAIL: error_code = %d, want 7\n", out.error_code);
        errs++;
    }
    if (fabs(out.location.latitude - 48.8566) > 1e-10) {
        fprintf(stderr, "FAIL: latitude = %f\n", out.location.latitude);
        errs++;
    }
    if (fabs(out.location.longitude - 2.3522) > 1e-10) {
        fprintf(stderr, "FAIL: longitude = %f\n", out.location.longitude);
        errs++;
    }

    if (errs > 0) return 1;

    /* Print hex */
    for (int i = 0; i < enc; i++) {
        printf("%02x", buf[i]);
    }
    printf("\n");
    return 0;
}
CEOF

    if "$CC" -std=c11 -Wall -Wextra -Wno-unused-function \
        -I"$TMP" "$TMP/test_roundtrip.c" -o "$TMP/test_c" -lm 2>"$TMP/c_compile.err"; then
        if "$TMP/test_c" > "$TMP/c.hex" 2>"$TMP/c.err"; then
            pass "C roundtrip"
        else
            fail "C roundtrip"
            cat "$TMP/c.err" >&2
        fi
    else
        fail "C compilation"
        cat "$TMP/c_compile.err" >&2
    fi
fi

# ---------------------------------------------------------------
# Phase 4: C++ roundtrip
# ---------------------------------------------------------------

CXX=""
for cxx_candidate in clang++ g++; do
    if command -v "$cxx_candidate" > /dev/null 2>&1; then
        CXX="$cxx_candidate"
        break
    fi
done

if [ -z "$CXX" ]; then
    info "SKIP: No C++ compiler found"
else
    cat > "$TMP/test_roundtrip.cpp" << 'CPPEOF'
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <cstdlib>
#include <cmath>
#include <optional>
#include <vector>
#include <string>
#include "interop_types.hpp"

int main() {
    /* Build SensorReading with deterministic values */
    SensorReading msg;
    msg.sensor_id = 42;
    msg.kind = SensorKind::PRESSURE;
    msg.value = 3.14f;
    msg.label = "test-sensor";
    msg.timestamp_ns = 1700000000000000000LL;
    msg.history = {1.0f, 2.0f, 3.0f};
    msg.error_code = 7;
    msg.location.latitude = 48.8566;
    msg.location.longitude = 2.3522;

    /* Encode */
    uint8_t buf[4096];
    int enc = msg.encode_cdr2_le(buf, sizeof(buf));
    if (enc < 0) {
        std::fprintf(stderr, "FAIL: encode returned %d\n", enc);
        return 1;
    }

    /* Decode */
    SensorReading out;
    int dec = out.decode_cdr2_le(buf, static_cast<size_t>(enc));
    if (dec != enc) {
        std::fprintf(stderr, "FAIL: decode returned %d, encode was %d\n", dec, enc);
        return 2;
    }

    /* Validate fields */
    int errs = 0;
    if (out.sensor_id != 42) {
        std::fprintf(stderr, "FAIL: sensor_id = %u, want 42\n", out.sensor_id);
        errs++;
    }
    if (out.kind != SensorKind::PRESSURE) {
        std::fprintf(stderr, "FAIL: kind mismatch\n");
        errs++;
    }
    if (out.value != 3.14f) {
        std::fprintf(stderr, "FAIL: value = %f, want 3.14\n", static_cast<double>(out.value));
        errs++;
    }
    if (out.label != "test-sensor") {
        std::fprintf(stderr, "FAIL: label = '%s', want 'test-sensor'\n", out.label.c_str());
        errs++;
    }
    if (out.timestamp_ns != 1700000000000000000LL) {
        std::fprintf(stderr, "FAIL: timestamp_ns mismatch\n");
        errs++;
    }
    if (out.history.size() != 3) {
        std::fprintf(stderr, "FAIL: history.size = %zu, want 3\n", out.history.size());
        errs++;
    } else {
        if (out.history[0] != 1.0f) { std::fprintf(stderr, "FAIL: history[0]\n"); errs++; }
        if (out.history[1] != 2.0f) { std::fprintf(stderr, "FAIL: history[1]\n"); errs++; }
        if (out.history[2] != 3.0f) { std::fprintf(stderr, "FAIL: history[2]\n"); errs++; }
    }
    if (!out.error_code.has_value() || *out.error_code != 7) {
        std::fprintf(stderr, "FAIL: error_code mismatch\n");
        errs++;
    }
    if (std::fabs(out.location.latitude - 48.8566) > 1e-10) {
        std::fprintf(stderr, "FAIL: latitude = %f\n", out.location.latitude);
        errs++;
    }
    if (std::fabs(out.location.longitude - 2.3522) > 1e-10) {
        std::fprintf(stderr, "FAIL: longitude = %f\n", out.location.longitude);
        errs++;
    }

    if (errs > 0) return 1;

    /* Print hex */
    for (int i = 0; i < enc; i++) {
        std::printf("%02x", buf[i]);
    }
    std::printf("\n");
    return 0;
}
CPPEOF

    if "$CXX" -std=c++17 -Wall -Wextra -Wno-unused-function \
        -I"$TMP" "$TMP/test_roundtrip.cpp" -o "$TMP/test_cpp" -lm 2>"$TMP/cpp_compile.err"; then
        if "$TMP/test_cpp" > "$TMP/cpp.hex" 2>"$TMP/cpp.err"; then
            pass "C++ roundtrip"
        else
            fail "C++ roundtrip"
            cat "$TMP/cpp.err" >&2
        fi
    else
        fail "C++ compilation"
        cat "$TMP/cpp_compile.err" >&2
    fi
fi

# ---------------------------------------------------------------
# Phase 5: Cross-check CDR2 bytes
# ---------------------------------------------------------------
info "Cross-checking CDR2 bytes..."

# Python vs C++ (both handle @optional correctly)
if [ -s "$TMP/python.hex" ] && [ -s "$TMP/cpp.hex" ]; then
    if diff -q "$TMP/python.hex" "$TMP/cpp.hex" > /dev/null 2>&1; then
        pass "CDR2 cross-check: Python == C++"
    else
        fail "CDR2 cross-check: Python != C++"
        echo "  Python: $(cat "$TMP/python.hex")" >&2
        echo "  C++:    $(cat "$TMP/cpp.hex")" >&2
    fi
else
    info "SKIP: Python vs C++ cross-check (missing hex output)"
fi

# C vs Python -- C backend now supports @optional, bytes should match
if [ -s "$TMP/python.hex" ] && [ -s "$TMP/c.hex" ]; then
    if diff -q "$TMP/python.hex" "$TMP/c.hex" > /dev/null 2>&1; then
        pass "CDR2 cross-check: Python == C"
    else
        fail "CDR2 cross-check: Python != C"
        echo "  Python: $(cat "$TMP/python.hex")" >&2
        echo "  C:      $(cat "$TMP/c.hex")" >&2
    fi
else
    info "SKIP: Python vs C cross-check (missing hex output)"
fi

# ---------------------------------------------------------------
# Phase 6: Summary
# ---------------------------------------------------------------
echo ""
echo "========================================="
echo "  Codegen roundtrip test summary"
echo "========================================="
echo -e "  ${GREEN}PASS${NC}: $PASS"
echo -e "  ${RED}FAIL${NC}: $FAIL"
echo "========================================="

exit "$FAIL"
