#!/usr/bin/env python3
"""Zero the LC_UUID load command in a Mach-O file, in place.

Apple's linker derives LC_UUID from a hash that includes the object-file paths, so the same
sources built in two different directories produce two different UUIDs. That is a real path
dependency, but it is confined to 16 bytes of metadata and says nothing about whether the compiled
code matches — which is what the reproducibility payload tier is asking. Zeroing it lets the
comparison see the code. See DESIGN.md §20.3.

Handles thin and fat (universal) binaries. Exits 0 whether or not a UUID was found; a Mach-O
without LC_UUID is already normalized.
"""

import struct
import sys

LC_UUID = 0x1B

# Keyed by the first four bytes read little-endian: (struct prefix for this file's fields, 64-bit?).
THIN = {
    0xFEEDFACF: ("<", True),   # little-endian 64-bit  (bytes CF FA ED FE)
    0xFEEDFACE: ("<", False),  # little-endian 32-bit
    0xCFFAEDFE: (">", True),   # big-endian 64-bit
    0xCEFAEDFE: (">", False),  # big-endian 32-bit
}
FAT = {0xCAFEBABE: ">", 0xBEBAFECA: "<"}


def normalize_thin(buf: bytearray, offset: int) -> int:
    """Zero LC_UUID in the Mach-O header at `offset`. Returns the number of UUIDs zeroed."""
    if offset + 32 > len(buf):
        return 0
    magic = struct.unpack_from("<I", buf, offset)[0]
    if magic not in THIN:
        return 0
    endian, is64 = THIN[magic]
    ncmds = struct.unpack_from(endian + "I", buf, offset + 16)[0]
    lc = offset + (32 if is64 else 28)
    zeroed = 0
    for _ in range(ncmds):
        if lc + 8 > len(buf):
            break
        cmd, cmdsize = struct.unpack_from(endian + "II", buf, lc)
        if cmdsize < 8 or lc + cmdsize > len(buf):
            break
        if cmd == LC_UUID and lc + 24 <= len(buf):
            buf[lc + 8 : lc + 24] = b"\x00" * 16
            zeroed += 1
        lc += cmdsize
    return zeroed


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: macho-normalize.py <file>", file=sys.stderr)
        return 2
    path = sys.argv[1]
    with open(path, "rb") as fh:
        buf = bytearray(fh.read())
    if len(buf) < 8:
        return 0

    magic = struct.unpack_from("<I", buf, 0)[0]
    if magic in FAT:
        endian = FAT[magic]
        nfat = struct.unpack_from(endian + "I", buf, 4)[0]
        zeroed = 0
        for i in range(nfat):
            # fat_arch is 20 bytes; `offset` is the third field.
            arch_off = 8 + i * 20
            if arch_off + 20 > len(buf):
                break
            slice_off = struct.unpack_from(endian + "I", buf, arch_off + 8)[0]
            zeroed += normalize_thin(buf, slice_off)
    else:
        zeroed = normalize_thin(buf, 0)

    if zeroed:
        with open(path, "wb") as fh:
            fh.write(buf)
    return 0


if __name__ == "__main__":
    sys.exit(main())
