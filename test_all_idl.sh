#!/bin/bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
# Copyright (c) 2025-2026 naskel.com
#
# Production IDL test suite: parse all valid IDL files, check invalid ones fail.

set -euo pipefail

HDDSGEN="./target/debug/hddsgen"
PASS=0
FAIL=0
SKIP=0

# Valid IDL files - must parse without error
valid_files=(
    examples/HelloWorld.idl
    examples/advanced.idl
    examples/flow.idl
    examples/big_arrays.idl
    examples/fqn_ok.idl
    examples/config.idl
    examples/sample.idl
    examples/micro/sensor.idl
    examples/c-micro/temperature.idl
    examples/comments/nested.idl
    # examples/interfaces/Simple.idl  # uses 'interface' (not yet supported)
    examples/canonical/HelloWorld.idl
    examples/canonical/AnnotationsMatrix.idl
    examples/canonical/UnionCases.idl
    examples/canonical/Advanced.idl
    examples/canonical/WideTypes.idl
    examples/canonical/ArraysAndSequences.idl
    examples/canonical/ConstExpressions.idl
    examples/canonical/BitsetEdgeCases.idl
    examples/canonical/NumericAndLiterals.idl
    examples/canonical/TypedefChains.idl
    examples/canonical/StringsAndEscapes.idl
    examples/canonical/FixedDecimal.idl
    examples/canonical/BitmaskOps.idl
    examples/canonical/MapsNested.idl
    examples/canonical/UnionsAdvanced.idl
    examples/canonical/BitFeatures.idl
    examples/canonical/EnumValues.idl
    # examples/macros/FuncLike.idl         # function-like macro expansion (not yet supported)
    # examples/macros/FuncLikeSpaces.idl   # function-like macro expansion (not yet supported)
    # examples/macros/FuncLikeZero.idl     # function-like macro expansion (not yet supported)
    examples/macros/Toggle.idl
    examples/macros/AliasType.idl
    examples/macros/If0Guard.idl
    examples/macros/NestedArgs.idl
    # examples/macros/Stringize.idl        # stringize operator (not yet supported)
    examples/macros/TokenPaste.idl
    examples/macros/test_stringize.idl
    examples/macros/test_token_paste.idl
)

# Invalid IDL files - must fail validation
invalid_files=(
    examples/invalid/UnionDuplicateLabels.idl
    examples/invalid/BitBoundExceed.idl
    examples/invalid/FqnAmbiguous.idl
    examples/invalid/BitsetOverlap.idl
    examples/invalid/DataRepresentationOnMember.idl
    examples/invalid/AnnotationsConflict.idl
    examples/invalid/DataRepresentationInvalid.idl
    examples/invalid/EnumDuplicateNames.idl
    examples/invalid/MapInvalidKey.idl
    examples/invalid/AutoIdSequential.idl
    examples/invalid/UnionDefaultAnnotationConflict.idl
    examples/invalid/UnionMultipleDefault.idl
    examples/invalid/NonSerializedOnType.idl
    examples/invalid/CustomAnnotationMissingParam.idl
)

echo "=== Valid IDL files (must parse OK) ==="
for f in "${valid_files[@]}"; do
    if [ ! -f "$f" ]; then
        echo "  SKIP $f (not found)"
        SKIP=$((SKIP + 1))
        continue
    fi
    if $HDDSGEN check "$f" > /dev/null 2>&1; then
        PASS=$((PASS + 1))
    else
        echo "  FAIL $f"
        FAIL=$((FAIL + 1))
    fi
done

echo "=== Invalid IDL files (must fail validation) ==="
for f in "${invalid_files[@]}"; do
    if [ ! -f "$f" ]; then
        echo "  SKIP $f (not found)"
        SKIP=$((SKIP + 1))
        continue
    fi
    if $HDDSGEN check "$f" > /dev/null 2>&1; then
        echo "  FAIL $f (should have failed but passed)"
        FAIL=$((FAIL + 1))
    else
        PASS=$((PASS + 1))
    fi
done

TOTAL=$((PASS + FAIL + SKIP))
echo ""
echo "Results: $PASS passed, $FAIL failed, $SKIP skipped (total: $TOTAL)"

if [ $FAIL -gt 0 ]; then
    exit 1
fi
echo "All IDL tests passed!"
