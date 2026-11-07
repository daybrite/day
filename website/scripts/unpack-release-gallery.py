#!/usr/bin/env python3
"""Read only indexed PNGs from a verified release bundle; never extract tar paths."""
import hashlib
import json
from pathlib import Path, PurePosixPath
import struct
import sys
import tarfile


def unpack(archive, destination):
    root = Path(destination)
    with tarfile.open(archive, 'r:xz') as bundle:
        manifest = bundle.getmember('gallery.json')
        if not manifest.isfile() or manifest.size > 10_000_000:
            raise ValueError('Invalid gallery index member')
        data = json.load(bundle.extractfile(manifest))
        for shot in data['screenshots']:
            segments = [shot['platform'], *([shot['device']] if shot.get('device') else []), shot['variant'], shot['file']]
            if any(not s or s in ('.', '..') or '/' in s or '\\' in s for s in segments):
                raise ValueError('Unsafe screenshot path')
            relative = PurePosixPath(*segments)
            if relative.suffix != '.png':
                raise ValueError('Expected PNG screenshot')
            member = bundle.getmember(str(relative))
            if not member.isfile() or member.size > 100_000_000:
                raise ValueError('Invalid screenshot member')
            content = bundle.extractfile(member).read()
            expected = shot.get('archived', {}).get('sha256', shot['sha256'])
            if hashlib.sha256(content).hexdigest() != expected:
                raise ValueError(f'Screenshot checksum mismatch: {relative}')
            if content[:8] != b'\x89PNG\r\n\x1a\n' or struct.unpack('>II', content[16:24]) != (shot['width'], shot['height']):
                raise ValueError(f'Screenshot dimensions mismatch: {relative}')
            output = root / relative
            output.parent.mkdir(parents=True, exist_ok=True)
            if not output.exists():
                output.write_bytes(content)
            shot['extractedPath'] = str(relative)
        (root / 'index.json').write_text(json.dumps(data))


if __name__ == '__main__':
    unpack(*sys.argv[1:])
