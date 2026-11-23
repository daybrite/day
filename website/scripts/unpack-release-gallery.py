#!/usr/bin/env python3
"""Read only indexed PNGs from a verified release bundle; never extract tar paths."""
import hashlib
import json
from pathlib import Path, PurePosixPath
import struct
import sys
import tarfile
import lzma
import zipfile
import stat


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


def stream(archive, index, output=None):
    """Emit length-prefixed JSON/PNG pairs without extracting files or archive paths."""
    output = output or sys.stdout.buffer
    data = json.loads(Path(index).read_text())
    expected = {}
    for shot in data['screenshots']:
        segments = [shot['platform'], *([shot['device']] if shot.get('device') else []), shot['variant'], shot['file']]
        if any(not isinstance(s, str) or not s or s in ('.', '..') or '/' in s or '\\' in s for s in segments):
            raise ValueError('Unsafe screenshot path')
        relative = str(PurePosixPath(*segments))
        if not relative.endswith('.png') or relative in expected:
            raise ValueError('Invalid or duplicate screenshot path')
        expected[relative] = shot

    def emit(name, size, regular, read):
        if name not in expected:
            return
        if not regular or size > 100_000_000:
            raise ValueError('Invalid screenshot member')
        shot = expected.pop(name)
        content = read()
        archived = shot.get('archived') or {}
        if hashlib.sha256(content).hexdigest() != archived.get('sha256', shot['sha256']):
            raise ValueError(f'Screenshot checksum mismatch: {name}')
        dimensions = (archived.get('width', shot['width']), archived.get('height', shot['height']))
        if content[:8] != b'\x89PNG\r\n\x1a\n' or len(content) < 24 or struct.unpack('>II', content[16:24]) != dimensions:
            raise ValueError(f'Screenshot dimensions mismatch: {name}')
        header = json.dumps({'shot': shot, 'relative': name, 'bytes': len(content)}).encode()
        output.write(struct.pack('>I', len(header)))
        output.write(header)
        output.write(content)

    if str(archive).endswith('.zip'):
        with zipfile.ZipFile(archive) as bundle:
            for member in bundle.infolist():
                if not expected:
                    break
                mode = member.external_attr >> 16
                emit(member.filename, member.file_size,
                     not member.is_dir() and not stat.S_ISLNK(mode), lambda: bundle.read(member))
    else:
        with lzma.open(archive, 'rb') as compressed, tarfile.open(fileobj=compressed, mode='r|') as bundle:
            for member in bundle:
                if not expected:
                    break
                emit(member.name.removeprefix('./'), member.size, member.isfile(),
                     lambda: bundle.extractfile(member).read())
    if expected:
        raise ValueError(f'Missing screenshot members: {list(expected)[:3]}')


if __name__ == '__main__':
    if sys.argv[1] == '--stream':
        stream(*sys.argv[2:])
    else:
        unpack(*sys.argv[1:])
