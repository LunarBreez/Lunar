#!/usr/bin/env python3
"""Graines de la cible de fuzzing `packet_frames`, tirées de captures pcap.

Chaque graine est un octet de type de liaison suivi d'une trame brute. Seul le
format pcap classique est lu ; un pcapng, une capture illisible ou un type de
liaison au-delà de 255 ne donne aucune graine.

  extract-frames.py SORTIE CAPTURE.pcap [...]
"""

import hashlib
import struct
import sys
from pathlib import Path

# Nombre magique lu en gros-boutiste -> boutisme des en-têtes du fichier
# (secondes/microsecondes, puis secondes/nanosecondes).
MAGICS = {
    0xA1B2C3D4: ">",
    0xD4C3B2A1: "<",
    0xA1B23C4D: ">",
    0x4D3CB2A1: "<",
}
MAX_FRAME = 16 * 1024
MAX_PER_FILE = 40


def frames(path: Path):
    """Trames d'une capture, avec son type de liaison."""
    data = path.read_bytes()
    if len(data) < 24:
        return
    endian = MAGICS.get(struct.unpack(">I", data[:4])[0])
    if endian is None:
        return
    (link_type,) = struct.unpack(endian + "I", data[20:24])
    if link_type == 12:  # DLT_RAW historique -> LINKTYPE_RAW
        link_type = 101
    if link_type > 255:
        return
    offset, count = 24, 0
    while offset + 16 <= len(data) and count < MAX_PER_FILE:
        (captured,) = struct.unpack(endian + "I", data[offset + 8 : offset + 12])
        offset += 16
        if captured == 0 or offset + captured > len(data):
            return
        yield link_type, data[offset : offset + min(captured, MAX_FRAME)]
        offset += captured
        count += 1


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    for capture in sys.argv[2:]:
        for link_type, frame in frames(Path(capture)):
            seed = bytes([link_type]) + frame
            (out / hashlib.sha256(seed).hexdigest()[:40]).write_bytes(seed)
    return 0


if __name__ == "__main__":
    sys.exit(main())
