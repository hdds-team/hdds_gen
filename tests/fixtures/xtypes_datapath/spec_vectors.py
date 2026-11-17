#!/usr/bin/env python3
"""Write spec_plprobe.txt, spec_mutable_union.txt and
spec_mutable_union_hashed.txt: byte strings built header by header from the
specification text, never captured from a toolchain.

Specification text references are line numbers of DDS-XTypes 1.3
(DDS-XTypes-1.3.txt) and DDSI-RTPS 2.5 (DDSI-RTPS-2.5.txt):

  XCDR1 parameter list (PL_CDR, XTypes 7.4.1.2):
    short header: 16-bit parameter id (FLAG_MUST_UNDERSTAND 0x4000 set if
      and only if the member must be understood, lines 6029-6036), 16-bit
      length; the short form maps member ids up to 0x3F00.
    a short length counts the bytes to the next parameter id, a multiple of
      four (RTPS 9.4.2.11, RTPS lines 8560-8561): "padded".
    extended header PID_EXTENDED 0x7F01, slength 8, eMemberHeader (FLAG_2 =
      must understand, 0x40000000), llength = "the exact length of the
      serialized member" (lines 6100-6124).
    PID_LIST_END 0x7F02 (Table 34); readers are robust to 0x3F02 and to any
      setting of FLAG_MUST_UNDERSTAND on the reserved ids (lines 6085-6087).
  XCDR2 PL_CDR2 (7.4.3.4.2, lines 6527-6560): DHEADER, then per member
    EMHEADER1 = (M << 31) + (LC << 28) + id, LC 0..3 = 1/2/4/8 bytes, LC 4 =
    NEXTINT bytes, LC 5 = NEXTINT bytes and NEXTINT is also the member's
    first word (a string's length).
  Verdicts: a reader that does not recognize a member whose must-understand
    attribute is true "must discard the entire data sample"; otherwise it
    omits the member (lines 2292-2296). Key members and a union's
    discriminator always have the attribute (lines 2299-2300, 2367-2368);
    the flag of a member the reader knows changes nothing (lines 6085-6087).
  @autoid(HASH) member id: the first four bytes of MD5(name), little
    endian, masked to 28 bits (7.3.1.2.1.1, lines 3866-3873); the spec's
    Example 2 gives 0x0FA5DD70 for "color" and 0x047790DA for "shapesize".

Run from this directory: python3 spec_vectors.py
"""
import hashlib
import struct


def le32(v):
    return struct.pack('<I', v & 0xFFFFFFFF)


def pad4(b):
    return b + b'\0' * (-len(b) % 4)


def short(pid, value, padded):
    """XCDR1 short parameter, then the padding to the next 4-byte boundary."""
    length = len(value) + (-len(value) % 4 if padded else 0)
    return pad4(struct.pack('<HH', pid, length) + value)


def ext(header, value, padded=False):
    """XCDR1 PID_EXTENDED parameter (exact llength unless `padded`)."""
    length = len(value) + (-len(value) % 4 if padded else 0)
    return pad4(struct.pack('<HH', 0x7F01, 8) + le32(header) + le32(length) + value)


def emh(m, lc, mid):
    return le32((m << 31) | (lc << 28) | mid)


def hash_id(name):
    return struct.unpack('<I', hashlib.md5(name.encode()).digest()[:4])[0] & 0x0FFFFFFF


assert hash_id('color') == 0x0FA5DD70 and hash_id('shapesize') == 0x047790DA

LIST_END = struct.pack('<HH', 0x7F02, 0)
LIST_END_UNFLAGGED = struct.pack('<HH', 0x3F02, 0)
PL_CDR_LE = bytes.fromhex('00030000')
PL_CDR2_LE = bytes.fromhex('000b0000')
CDR_LE = bytes.fromhex('00010000')
PLAIN_CDR2_LE = bytes.fromhex('00070000')

# @mutable struct PlProbe { @key @id(1) long id; @id(2) short s;
#   @id(3) string name; @id(16144) long long big; @key @id(5) octet tag; };
# id = 0x11223344, s = -2, name = "hdds", big = 0x0102030405060708, tag = 0xab
ID = struct.pack('<i', 0x11223344)
S = struct.pack('<h', -2)
NAME = le32(5) + b'hdds\0'
BIG = struct.pack('<q', 0x0102030405060708)
TAG = b'\xab'
BIG_ID = 16144  # 0x3F10, above the short form's 0x3F00


def x1_probe(key_mu, padded, extra=b'', keys_extended=False, all_extended=False,
             ext_end=0x7F01, end=LIST_END):
    """PlProbe as a parameter list; `ext_end` is the PID_EXTENDED value used."""
    out = b''
    for mid, value, key in ((1, ID, True), (2, S, False), (3, NAME, False),
                            (BIG_ID, BIG, False), (5, TAG, True)):
        flag = key and key_mu
        if all_extended or mid > 0x3F00 or (key and keys_extended):
            p = ext((0x40000000 if flag else 0) | mid, value,
                    padded=padded and all_extended)
            if ext_end != 0x7F01:
                p = struct.pack('<H', ext_end) + p[2:]
            out += p
        else:
            out += short((0x4000 if flag else 0) | mid, value, padded)
    return PL_CDR_LE + out + extra + end


def x2_probe(key_m, string_lc, extra=b''):
    body = emh(key_m, 2, 1) + ID
    body = pad4(body + emh(0, 1, 2) + S)
    if string_lc == 5:
        body = pad4(body + emh(0, 5, 3) + NAME)
    else:
        body = pad4(body + emh(0, 4, 3) + le32(len(NAME)) + NAME)
    body += emh(0, 3, BIG_ID) + BIG
    body += emh(key_m, 0, 5) + TAG
    if extra:
        body = pad4(body) + extra
    return PL_CDR2_LE + le32(len(body)) + body


UNKNOWN9 = struct.pack('<i', 9)
plprobe = [
    ('X1-S0', 'ACCEPT', 'short headers, padded lengths, keys without the flag',
     x1_probe(False, True)),
    ('X1-S1', 'ACCEPT', 'X1-S0 with FLAG_MUST_UNDERSTAND (0x4000) on both keys',
     x1_probe(True, True)),
    ('X1-H0', 'ACCEPT', 'what HDDS writes: keys 0x4000, exact lengths, 0x7F01/0x7F02',
     x1_probe(True, False)),
    ('X1-H1', 'ACCEPT', 'X1-H0, keys without 0x4000',
     x1_probe(False, False)),
    ('X1-H2', 'ACCEPT', 'X1-H0 with the reserved ids unflagged, 0x3F01/0x3F02',
     x1_probe(True, False, ext_end=0x3F01, end=LIST_END_UNFLAGGED)),
    ('X1-H3', 'ACCEPT', 'HDDS 1.4.x shape: no flag anywhere',
     x1_probe(False, False, ext_end=0x3F01, end=LIST_END_UNFLAGGED)),
    ('X1-Y0', 'ACCEPT', 'every member extended, padded llength, keys without FLAG_2 (the form Cyclone DDS 0.11 writes)',
     x1_probe(False, True, all_extended=True)),
    ('X1-Y1', 'ACCEPT', 'X1-Y0 with FLAG_2 on both keys',
     x1_probe(True, True, all_extended=True)),
    ('X1-E0', 'ACCEPT', 'keys extended, exact llength, no FLAG_2; the rest short and exact',
     x1_probe(False, False, keys_extended=True)),
    ('X1-E1', 'ACCEPT', 'X1-E0 with FLAG_2 on both keys',
     x1_probe(True, False, keys_extended=True)),
    ('X1-U0', 'ACCEPT', 'X1-S0 + unknown member 9 without the flag: omitted',
     x1_probe(False, True, extra=short(0x0009, UNKNOWN9, True))),
    ('X1-U1', 'REJECT', 'X1-S0 + unknown member 9 with 0x4000: sample discarded',
     x1_probe(False, True, extra=short(0x4009, UNKNOWN9, True))),
    ('X1-U2', 'REJECT', 'X1-S0 + unknown extended member 0x10 with FLAG_2: sample discarded',
     x1_probe(False, True, extra=ext(0x40000010, struct.pack('<i', 16)))),
    ('X2-S0', 'ACCEPT', 'PL_CDR2, keys without M, LC=5 for the string',
     x2_probe(0, 5)),
    ('X2-S1', 'ACCEPT', 'X2-S0 with M on both keys (what hddsgen writes)',
     x2_probe(1, 5)),
    ('X2-H0', 'ACCEPT', 'keys without M, LC=4 for the string (NEXTINT = 9)',
     x2_probe(0, 4)),
    ('X2-H1', 'ACCEPT', 'X2-H0 with M on both keys (what HDDS writes)',
     x2_probe(1, 4)),
    ('X2-U0', 'ACCEPT', 'X2-S0 + unknown member 9 without M: omitted',
     x2_probe(0, 5, extra=emh(0, 2, 9) + UNKNOWN9)),
    ('X2-U1', 'REJECT', 'X2-S0 + unknown member 9 with M: sample discarded',
     x2_probe(0, 5, extra=emh(1, 2, 9) + UNKNOWN9)),
]

with open('spec_plprobe.txt', 'w') as f:
    f.write('''# Built by spec_vectors.py from the specification text (see its header
# for the rules and their line numbers), not captured from a toolchain:
# the @mutable two-key type below with id=0x11223344 s=-2 name="hdds"
# big=0x0102030405060708 tag=0xab, laid out the ways a writer may lay it
# out, each with the verdict the specification gives a reader that knows
# members 1, 2, 3, 5 and 16144 (encapsulation header included).
#
#   @mutable struct PlProbe { @key @id(1) long id; @id(2) short s; @id(3) string name;
#     @id(16144) long long big; @key @id(5) octet tag; };
#
''')
    for label, verdict, what, b in plprobe:
        f.write(f'# {label} {what}\n{label} {verdict} {b.hex()}\n')


# Unions. @mutable union UM switch (long) { case 1: long a; case 2: string s;
#   case 3: double d; }; @final struct HoldUM { octet p; UM u; long z; };
# p = 1, z = 9. The holder is @final: p, padding, the union, z.
def x1_union(disc, case_id, value, spec):
    """The union as PL_CDR. `spec`: hddsgen's form, every member extended
    with the exact llength, the discriminator flagged; otherwise short
    headers where the id fits, padded lengths, the discriminator unflagged."""
    if spec:
        out = ext(0x40000000, le32(disc)) + ext(case_id, value)
    else:
        out = short(0x0000, le32(disc), True)
        out += short(case_id, value, True) if case_id <= 0x3F00 else ext(case_id, value)
    return out + LIST_END


def x2_union(disc, case_id, lc, value, spec):
    body = emh(1 if spec else 0, 2, 0) + le32(disc) + emh(0, lc, case_id) + value
    return le32(len(body)) + body


def holder(version, union, z):
    head = b'\x01\0\0\0'
    if version == 'xcdr1':
        out = CDR_LE + head + union
    else:
        out = PLAIN_CDR2_LE + head + union
    if z is not None:
        out = pad4(out) + le32(z)
    return out


def union_lines(samples, z):
    lines = []
    for name, disc, case_id, lc, value in samples:
        for version in ('xcdr1', 'xcdr2'):
            forms = {}
            for spec in (False, True):
                if version == 'xcdr1':
                    u = x1_union(disc, case_id, value, spec)
                else:
                    u = x2_union(disc, case_id, lc, value, spec)
                forms[spec] = holder(version, u, z)
            # No 8-byte member may land off its XCDR1 alignment (double at 8).
            lines.append(f'{name} {version} alt={forms[False].hex()} spec={forms[True].hex()}')
    return lines


HELLO = le32(6) + b'hello\0'
um = [('HoldUM_1', 1, 1, 2, le32(10)),
      ('HoldUM_2', 2, 2, 5, HELLO),
      ('HoldUM_3', 3, 3, 3, struct.pack('<d', 0.25))]
umh = [('HoldUMh_1', 1, hash_id('color'), 2, le32(0x11)),
       ('HoldUMh_2', 2, hash_id('shapesize'), 1, struct.pack('<h', 0x22))]

# The XCDR1 double of HoldUM_3 must start 8-aligned from the stream origin.
for spec in (False, True):
    b = holder('xcdr1', x1_union(3, 3, struct.pack('<d', 0.25), spec), 9)[4:]
    off = b.index(struct.pack('<d', 0.25))
    assert off % 8 == 0, (spec, off)

with open('spec_mutable_union.txt', 'w') as f:
    f.write('''# @mutable unions (DDS-XTypes 1.3 7.4.3.5.3 rules 27-28), which Cyclone DDS
# 0.11 does not support, built by spec_vectors.py from the specification
# text (see its header), for p=1, z=9 and each case: a=10, s="hello",
# d=0.25 (encapsulation header included). The discriminator is member 0,
# the cases members 1, 2, 3.
# `spec=` is the form hddsgen writes: the discriminator's header carries the
# must-understand flag ("the discriminator member shall always have the must
# understand attribute set to true", spec text lines 2299-2300); XCDR1
# parameters use the extended header with the exact member length
# (7.4.1.2.2). `alt=` is another form a writer may use, which a reader must
# read into the same sample: XCDR1 short headers with lengths padded to the
# next parameter (DDSI-RTPS 2.5 lines 8560-8561), and in both versions the
# discriminator without the flag, which changes nothing for a member the
# reader knows (lines 6085-6087).
#
#   @mutable union UM switch (long) { case 1: long a; case 2: string s; case 3: double d; };
#   @final struct HoldUM { octet p; UM u; long z; };
#
''')
    f.write('\n'.join(union_lines(um, 9)) + '\n')

with open('spec_mutable_union_hashed.txt', 'w') as f:
    f.write('''# @mutable @autoid(HASH) union (rules 27-28, member ids by MD5 name hash,
# DDS-XTypes 1.3 7.3.1.2.1.1 lines 3866-3873), built by spec_vectors.py from
# the specification text for p=1 and color=0x11 / shapesize=0x22:
#   @mutable @autoid(HASH) union UMh switch (long) { case 1: long color; case 2: short shapesize; };
#   @final struct HoldUMh { octet p; UMh u; };
# Case ids: 0x0FA5DD70 (color) and 0x047790DA (shapesize), the spec's
# Example 2 values. `spec=` and `alt=` as in spec_mutable_union.txt; a hashed
# case id is above 0x3F00, so its XCDR1 header is extended in both forms.
#
''')
    f.write('\n'.join(union_lines(umh, None)) + '\n')
