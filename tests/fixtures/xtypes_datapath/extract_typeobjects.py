#!/usr/bin/env python3
"""Extract the complete TypeObjects of Cyclone idlc's TYPE_MAP_CDR arrays.

Reads corpus.c, prints one line per distinct complete TypeObject:
<hash14 hex> md5ok <the serialized TypeObject, hex: DHEADER, EK_COMPLETE, then the
CompleteTypeObject (TK discriminator first), aligned as in the stream: the
CompleteTypeObject starts at byte 5>
and checks that the hash is MD5 of the serialized TypeObject (DDS-XTypes 1.3
7.3.4.1.2 / EquivalenceHash), so the extraction itself is verified."""
import hashlib, re, sys

src = open(sys.argv[1]).read()
maps = re.findall(r'#define TYPE_MAP_CDR_(\w+) \(const unsigned char \[\]\)\{ \\\n(.*?)\}', src, re.S)
out = {}
for name, body in maps:
    b = bytes(int(x, 16) for x in re.findall(r'0x([0-9a-f]{2})', body))
    pos = 0
    def u32(p): return int.from_bytes(b[p:p+4], 'little')
    def al(p): return (p + 3) & ~3
    # the minimal pairs: skipped
    dh = u32(pos); pos = pos + 4 + dh
    pos = al(pos)
    pos += 4  # DHEADER of the complete pairs
    n = u32(pos); pos += 4
    for _ in range(n):
        assert b[pos] == 0xf2, (name, hex(b[pos]))
        h = b[pos+1:pos+15]; pos += 15
        pos = al(pos)
        L = u32(pos)
        to = b[pos:pos+4+L]
        pos = pos + 4 + L
        # TypeObject serialization (appendable union): DHEADER + disc + value
        md5 = hashlib.md5(to).digest()[:14]
        ok = 'md5ok' if md5 == h else 'md5MISMATCH'
        out[h.hex()] = (to.hex(), ok)
for h, (body, ok) in sorted(out.items()):
    print(h, ok, body)
