// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Instance `KeyHash` of generated Python classes, generated and run with
//! `python3` (RTPS 2.5 9.6.4.8). Skipped, with a message, when `python3` is
//! not installed.
//!
//! Expected values: the same vectors as `tests/key_hash_rust.rs` (SPEC
//! and REF, see there), plus a derived type whose key member is
//! declared in its base: `Base { @key long id; }` and `Derived : Base {
//! @key short sub; }` give `00000007 0003` padded (REF).

#![allow(
    clippy::expect_used,
    clippy::needless_raw_string_hashes,
    clippy::needless_pass_by_value
)]

use std::fs;
use std::path::Path;
use std::process::Command;

use hddsgen::{Backend, Parser};

const GOLDEN_IDL: &str = include_str!("fixtures/key_hash_golden.idl");
const EXT_IDL: &str = include_str!("fixtures/key_hash_ext.idl");
const INHERITANCE_IDL: &str = "
struct Base { @key long id; long payload; };
struct Derived : Base { @key short sub; long more; };
";

const RUNNER: &str = r#"import importlib.util, sys
def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path); m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m); return m
import os
d = os.path.dirname(os.path.abspath(__file__))
g = load('g', os.path.join(d, 'kh_golden.py')); x = load('x', os.path.join(d, 'kh_ext.py')); h = load('h', os.path.join(d, 'kh_inh.py'))
fails = 0
def chk(label, v, exp):
    global fails
    got = v.compute_key().hex()
    if got == exp: print('OK', label)
    else: print('FAIL', label, got, exp); fails += 1
chk('LongKey', g.LongKey(id=0x12345678, x=9), '12345678000000000000000000000000')
chk('SpecEx2', g.SpecEx2(label='BLUE', id=0x123456789abcdef0, x=1, y=2), 'f91a59e32e4535d9a69cd5d9f5b6e36e')
chk('SpecEx3', g.SpecEx3(label='BLUE', m_nested=g.Nested(m_long=0x12345678, u=1, w=2), x=1, y=2), '374b96e2e727237f016cc4cebb6eb71e')
chk('StrKey', g.StrKey(label='BLUE', x=1), 'cac217c318363f8ef1160eeedef9e886')
chk('StrKey8', g.StrKey8(label='BLUE', x=1), '00000005424c55450000000000000000')
chk('Str11', g.Str11(label='BLUE', x=1), '00000005424c55450000000000000000')
chk('Str12', g.Str12(label='BLUE', x=1), 'cac217c318363f8ef1160eeedef9e886')
chk('ShapeType', g.ShapeType(color='BLUE', x=1, y=2, shapesize=3), 'cac217c318363f8ef1160eeedef9e886')
chk('TwoKey', g.TwoKey(a=1, b=2, x=3), '00000001000200000000000000000000')
chk('TwoKeyId', g.TwoKeyId(a=1, b=2, x=3), '00020000000000010000000000000000')
chk('Outer', g.Outer(in_=g.Inner(k=0x11223344, u=7), tag=0x55, x=1), '11223344550000000000000000000000')
chk('OuterU', g.Outer(in_=g.Inner(k=0x11223344, u=-7), tag=0x55, x=2), '11223344550000000000000000000000')
chk('OuterNoKey', g.OuterNoKey(p=g.Inner2(a=1, b=2), x=1), '00000001000000020000000000000000')
chk('AlignKey', g.AlignKey(t=1, v=2), '01000000000000000000000200000000')
chk('Key16', g.Key16(a=1, b=2), '00000000000000010000000000000002')
chk('CapKey', g.CapKey(t=1, v=2, c=3), '01000000000000000000000200000003')
chk('BigKey', g.BigKey(a=1, b=2, c=3), '8898488e44b5e511051f83ce5e917bb8')
chk('StrKey55', g.StrKey(label='Q' * 50, x=1), 'fa67225c93f8071a5bbcf6df1bf9bfcd')
chk('StrKey56', g.StrKey(label='Q' * 51, x=1), '424bcbae3ad0fa3c57426e73ad1261af')
chk('StrKey64', g.StrKey(label='Q' * 59, x=1), '4b60eb0dd74deb107a2c9cad4e781130')
chk('StrKey65', g.StrKey(label='Q' * 60, x=1), 'e7c43e8aa8fbb48e72bc38d01eb741a5')
chk('Scalars', x.Scalars(b=True, c='A', wc='Z', i8v=-2, s=0x0102, d=1.5, us=0xbeef, f=-1.0, nk=9), '777ac52171394c4abb7947cd8f237982')
chk('Enums', x.Enums(c=x.Color.BLUE, sm=x.Small.S2, md=x.Mid.M1, f8=x.Flags8(3), f32=x.Flags32(2), f64=x.Flags64(1 << 40), ml=-5), 'e2265fcd85db8fc8343f51c33f672a8b')
chk('Colls', x.Colls(sq=[1, 2], bsq=[9, 8, 7], arr=[1, 2, 3], oct2=[[1, 2], [3, 4]], ws='hé', n6='abc'), '7b44e4a1ef2ab87d5bc2b6b8d253beca')
chk('Unions', x.Unions(u=x.U(_discriminator=2, _value='xy'), uk=x.UK(_discriminator=x.Color.GREEN, _value=2.5), tail=0x77), '45ec7942ba3bf7038d5ebc3abfc11459')
chk('Hashed', x.Hashed(color=0x11, shapesize=0x22), '00000022000000110000000000000000')
chk('FinalIds', x.FinalIds(a=0x0a0b0c0d, b=1), '010000000a0b0c0d0000000000000000')
chk('MutSeq', x.MutSeq(a=1, b=2, c='Q'), 'ae5426ae810065bd695fddf3baf7b5c1')
chk('SeqChars', x.SeqChars(c=[0x78, 0x79], w=[0x41, 0x42], x=1), '7cfc7a60d95c3037ff7785cfd6822d1c')
chk('SeqCharU', x.SeqCharU(c=[0x78, 0x79], x=1), '62a27c5288ec9d48152240ad0670d151')
chk('DefaultsCase', x.Defaults(u=x.UD(_discriminator=1, _value=5)), '00010000000000050000000000000000')
chk('ModHashed', x.ModHashed(color=0x11, shapesize=0x22), '00000022000000110000000000000000')
chk('DefaultsDefault', x.Defaults(u=x.UD(_discriminator=7, _value=0x42)), '00074200000000000000000000000000')
chk('Derived', h.Derived(id=7, payload=1, sub=3, more=4), '00000007000300000000000000000000')
if not h.Derived.has_key():
    print('FAIL has_key: a base key member makes Derived keyed'); fails += 1
if h.Base(id=1, payload=2).compute_key() != h.Base(id=1, payload=3).compute_key():
    print('FAIL non-key member moved the KeyHash'); fails += 1
sys.exit(1 if fails else 0)
"#;

fn generate(idl: &str) -> String {
    let mut p = Parser::try_new(idl).expect("lexer");
    let ast = p.parse().expect("parse");
    Backend::Python.generator().generate(&ast).expect("codegen")
}

#[test]
fn python_key_hash_matches_the_golden_vectors() {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("python3 not found; Python KeyHash test not run");
        return;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("key_hash_python");
    fs::create_dir_all(&dir).expect("dir");
    fs::write(dir.join("kh_golden.py"), generate(GOLDEN_IDL)).expect("golden");
    fs::write(dir.join("kh_ext.py"), generate(EXT_IDL)).expect("ext");
    fs::write(dir.join("kh_inh.py"), generate(INHERITANCE_IDL)).expect("inheritance");
    fs::write(dir.join("run.py"), RUNNER).expect("runner");
    let out = Command::new("python3")
        .arg(dir.join("run.py"))
        .output()
        .expect("python3");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let ok = stdout.lines().filter(|l| l.starts_with("OK ")).count();
    assert_eq!(ok, 34, "{stdout}");
}
