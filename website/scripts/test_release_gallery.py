"""Synthetic release fixtures: guard against traversal, links, and corrupt images."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import struct
import tarfile
import zipfile
import lzma
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('unpack', Path(__file__).with_name('unpack-release-gallery.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class ReleaseTests(unittest.TestCase):
    def fixture(self, directory, *, platform='ios-uikit', link=False, digest=None):
        # Only the header is relevant to these extraction guards; production images
        # must also decode through sharp before any rendition is published.
        png = b'\x89PNG\r\n\x1a\n' + b'\0\0\0\rIHDR' + struct.pack('>II', 360, 720)
        shot = dict(platform=platform, variant='light', file='top.png', width=360, height=720, sha256=digest or hashlib.sha256(png).hexdigest())
        path = Path(directory) / 'shots.tar.xz'
        with tarfile.open(path, 'w:xz') as archive:
            raw = json.dumps({'screenshots': [shot]}).encode()
            info = tarfile.TarInfo('gallery.json'); info.size=len(raw); archive.addfile(info, io.BytesIO(raw))
            info = tarfile.TarInfo(f'{platform}/light/top.png')
            if link:
                info.type=tarfile.SYMTYPE; info.linkname='/outside'; archive.addfile(info)
            else:
                info.size=len(png); archive.addfile(info, io.BytesIO(png))
        return path

    def test_extracts_only_verified_regular_images(self):
        with tempfile.TemporaryDirectory() as directory:
            module.unpack(self.fixture(directory), directory)
            self.assertTrue((Path(directory) / 'ios-uikit/light/top.png').exists())

    def test_rejects_unsafe_or_corrupt_members(self):
        for kwargs in [{'platform':'..'}, {'link':True}, {'digest':'0'*64}]:
            with self.subTest(kwargs=kwargs), tempfile.TemporaryDirectory() as directory:
                with self.assertRaises(ValueError):
                    module.unpack(self.fixture(directory, **kwargs), directory)

    def stream_fixture(self, directory, **kwargs):
        archive = self.fixture(directory, **kwargs)
        with tarfile.open(archive) as bundle:
            index = Path(directory) / 'input.json'
            index.write_bytes(bundle.extractfile('gallery.json').read())
        return archive, index

    def test_streams_verified_tar_and_zip_without_raw_extraction(self):
        for kind in ['tar', 'zip', 'concatenated-xz']:
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as directory:
                archive, index = self.stream_fixture(directory)
                if kind == 'zip':
                    zipped = Path(directory) / 'shots.zip'
                    with tarfile.open(archive) as source, zipfile.ZipFile(zipped, 'w') as target:
                        for member in source:
                            target.writestr(member.name, source.extractfile(member).read())
                    archive = zipped
                elif kind == 'concatenated-xz':
                    raw = lzma.decompress(archive.read_bytes())
                    archive.write_bytes(lzma.compress(raw[:1024]) + lzma.compress(raw[1024:]))
                output = io.BytesIO()
                module.stream(archive, index, output)
                raw = output.getvalue()
                length = struct.unpack('>I', raw[:4])[0]
                header = json.loads(raw[4:4+length])
                self.assertEqual(header['relative'], 'ios-uikit/light/top.png')
                self.assertEqual(len(raw) - 4 - length, header['bytes'])
                self.assertFalse((Path(directory) / 'ios-uikit').exists())

    def test_stream_rejects_traversal_links_hash_and_missing_members(self):
        for kwargs in [{'platform': '..'}, {'link': True}, {'digest': '0'*64}, {}]:
            with self.subTest(kwargs=kwargs), tempfile.TemporaryDirectory() as directory:
                archive, index = self.stream_fixture(directory, **kwargs)
                if not kwargs:
                    data = json.loads(index.read_text())
                    data['screenshots'][0]['file'] = 'missing.png'
                    index.write_text(json.dumps(data))
                with self.assertRaises(ValueError):
                    module.stream(archive, index, io.BytesIO())

    def test_stream_rejects_zip_symlinks_and_wrong_dimensions(self):
        for kind in ['symlink', 'dimensions']:
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as directory:
                archive, index = self.stream_fixture(directory)
                if kind == 'symlink':
                    archive = Path(directory) / 'shots.zip'
                    with zipfile.ZipFile(archive, 'w') as bundle:
                        member = zipfile.ZipInfo('ios-uikit/light/top.png')
                        member.create_system = 3
                        member.external_attr = 0o120777 << 16
                        bundle.writestr(member, '/outside')
                else:
                    data = json.loads(index.read_text())
                    data['screenshots'][0]['width'] = 1
                    index.write_text(json.dumps(data))
                with self.assertRaises(ValueError):
                    module.stream(archive, index, io.BytesIO())

if __name__ == '__main__':
    unittest.main()
