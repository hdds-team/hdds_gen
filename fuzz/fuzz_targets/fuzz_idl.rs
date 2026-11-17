// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // Feed arbitrary UTF-8 strings as IDL input
    let Ok(mut parser) = hddsgen::Parser::try_new(data) else {
        return;
    };
    if let Ok(ast) = parser.parse() {
        let _ = hddsgen::validate(&ast);
        // If it parses, try pretty-printing roundtrip
        let pretty = hddsgen::idl_pretty(&ast);
        if let Ok(mut parser2) = hddsgen::Parser::try_new(&pretty) {
            let _ = parser2.parse();
        }
    }
});
