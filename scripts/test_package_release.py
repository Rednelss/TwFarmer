import hashlib
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

from package_release import PUBLIC_FILES, TARGETS, package


class ReleasePackagingTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.binary = self.root / 'compiled-binary'
        self.binary.write_bytes(b'binary fixture')
        for name in PUBLIC_FILES:
            (self.root / name).write_text(f'public: {name}', encoding='utf-8')
        (self.root / 'packaging').mkdir()
        (self.root / 'packaging/twfarmer.service').write_text('public unit', encoding='utf-8')
        (self.root / 'twfarmer.toml').write_text('private configuration', encoding='utf-8')
        (self.root / 'twfarmer.toml.accounts').mkdir()
        (self.root / 'twfarmer.toml.accounts/user.auth.json').write_text('secret fixture', encoding='utf-8')
        (self.root / 'twfarmer.toml.auth.json').write_text('legacy secret fixture', encoding='utf-8')
        (self.root / 'twfarmer.toml.control.json').write_text('control secret fixture', encoding='utf-8')
        self.output = self.root / 'dist'

    def test_all_formats_exclude_private_files_and_have_valid_checksums(self):
        for target in sorted(TARGETS):
            with self.subTest(target=target), patch('package_release.subprocess.check_output', return_value='twfarmer 0.1.0\n'):
                archive = package(self.root, self.binary, target, self.output, '0.1.0')
                prefix = f'twfarmer-v0.1.0-{target}/'
                expected = set(PUBLIC_FILES) | {'twfarmer.exe' if 'windows' in target else 'twfarmer'}
                if 'linux' in target:
                    expected.add('packaging/twfarmer.service')
                if 'windows' in target:
                    with zipfile.ZipFile(archive) as bundle:
                        names = {name.removeprefix(prefix) for name in bundle.namelist()}
                        payloads = [bundle.read(name) for name in bundle.namelist()]
                else:
                    with tarfile.open(archive) as bundle:
                        files = [entry for entry in bundle.getmembers() if entry.isfile()]
                        names = {entry.name.removeprefix(prefix) for entry in files}
                        payloads = [bundle.extractfile(entry).read() for entry in files]
                        executable = bundle.getmember(prefix + 'twfarmer')
                        self.assertEqual(executable.mode & 0o777, 0o755)
                self.assertEqual(names, expected)
                self.assertFalse(any(b'secret fixture' in contents for contents in payloads))
                checksum = archive.with_name(archive.name + '.sha256').read_bytes()
                digest = hashlib.sha256(archive.read_bytes()).hexdigest()
                self.assertEqual(checksum, f'{digest}  {archive.name}\n'.encode())

    def test_mismatched_binary_is_rejected_before_creating_archive(self):
        with patch('package_release.subprocess.check_output', return_value='twfarmer 0.0.9\n'):
            with self.assertRaisesRegex(ValueError, 'version mismatch'):
                package(self.root, self.binary, 'x86_64-unknown-linux-gnu', self.output, '0.1.0')
        self.assertFalse(self.output.exists())

    def test_public_file_cannot_be_replaced_with_a_symlink_to_credentials(self):
        license_path = self.root / 'LICENSE'
        license_path.unlink()
        try:
            license_path.symlink_to(self.root / 'twfarmer.toml.auth.json')
        except OSError:
            self.skipTest('Creating symlinks is unavailable on this system')
        with patch('package_release.subprocess.check_output', return_value='twfarmer 0.1.0\n'):
            with self.assertRaisesRegex(ValueError, 'regular file'):
                package(self.root, self.binary, 'x86_64-unknown-linux-gnu', self.output, '0.1.0')
        self.assertEqual(list(self.output.iterdir()), [])


if __name__ == '__main__':
    unittest.main()
