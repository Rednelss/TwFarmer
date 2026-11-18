"""Package an explicitly selected binary and public project files. Python 3.11+."""

import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import zipfile


ROOT = Path(__file__).resolve().parent.parent
TARGETS = {
    'x86_64-unknown-linux-gnu',
    'x86_64-pc-windows-msvc',
    'aarch64-apple-darwin',
}
PUBLIC_FILES = ('README.md', 'README.ru.md', 'LICENSE', 'CHANGELOG.md', 'CHANGELOG.ru.md', 'twfarmer.example.toml')


def copy_public(source: Path, destination: Path) -> None:
    if source.is_symlink() or not source.is_file():
        raise ValueError(f'Expected a regular file: {source}')
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    destination.chmod(0o644)


def package(root: Path, binary: Path, target: str, output: Path, version: str) -> Path:
    if target not in TARGETS:
        raise ValueError(f'Unsupported release target: {target}')
    if binary.is_symlink() or not binary.is_file():
        raise ValueError('Binary must be a regular file')
    expected = f'twfarmer {version}'
    actual = subprocess.check_output([str(binary.resolve()), '--version'], text=True).strip()
    if actual != expected:
        raise ValueError(f'Binary version mismatch: expected {expected!r}, got {actual!r}')
    name = f'twfarmer-v{version}-{target}'
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='twfarmer-release-') as temporary:
        stage = Path(temporary) / name
        stage.mkdir()
        executable = 'twfarmer.exe' if 'windows' in target else 'twfarmer'
        copy_public(binary, stage / executable)
        (stage / executable).chmod(0o755)
        for filename in PUBLIC_FILES:
            copy_public(root / filename, stage / filename)
        if 'linux' in target:
            copy_public(root / 'packaging/twfarmer.service', stage / 'packaging/twfarmer.service')
        if 'windows' in target:
            archive = output / f'{name}.zip'
            with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED) as bundle:
                for file in sorted(stage.rglob('*')):
                    if file.is_file():
                        bundle.write(file, file.relative_to(Path(temporary)).as_posix())
        else:
            archive = output / f'{name}.tar.gz'

            def archive_permissions(entry: tarfile.TarInfo) -> tarfile.TarInfo:
                # Windows chmod does not provide Unix executable bits.
                entry.mode = 0o755 if entry.isdir() or entry.name == f'{name}/{executable}' else 0o644
                return entry

            with tarfile.open(archive, 'w:gz') as bundle:
                bundle.add(stage, arcname=name, filter=archive_permissions)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    checksum = archive.with_name(archive.name + '.sha256')
    checksum.write_bytes(f'{digest}  {archive.name}\n'.encode('utf-8'))
    return archive


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', choices=sorted(TARGETS))
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--output', type=Path, default=ROOT / 'dist')
    parser.add_argument('--check-tag', action='store_true')
    args = parser.parse_args()
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text(encoding='utf-8'))['package']['version']
    if args.check_tag:
        tag = os.environ.get('GITHUB_REF_NAME')
        if tag != f'v{version}':
            parser.error(f'Release tag {tag!r} must match Cargo.toml: v{version}')
        print(f'Release tag matches v{version}')
        return
    if not args.target or not args.binary:
        parser.error('--target and --binary are required for packaging')
    archive = package(ROOT, args.binary, args.target, args.output, version)
    print(archive)
    print(archive.with_name(archive.name + '.sha256'))


if __name__ == '__main__':
    main()
