"""Synthetic release fixtures: guard against traversal, links, and corrupt images."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import struct
import tarfile
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

if __name__ == '__main__':
    unittest.main()
