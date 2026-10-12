#!/usr/bin/env python3
"""Package one real native singleton; never include fleet configuration."""
import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path
import struct
import subprocess
import tarfile

PLATFORMS = ('linux-x86_64', 'macos-x86_64', 'macos-aarch64')
SOURCE_PATHS = ('Cargo.toml', 'Cargo.lock', 'src')


def validate_binary(data, platform):
    if not 20 <= len(data) <= 16 * 1024 * 1024:
        raise ValueError('native executable size outside release bound')
    if platform == 'linux-x86_64':
        valid = (data[:6] == b'\x7fELF\x02\x01'
                 and struct.unpack_from('<H', data, 18)[0] == 62)
    elif platform in ('macos-x86_64', 'macos-aarch64'):
        cpu = 0x1000007 if platform == 'macos-x86_64' else 0x100000c
        valid = (data[:4] == b'\xcf\xfa\xed\xfe'
                 and struct.unpack_from('<I', data, 4)[0] == cpu)
    else:
        raise ValueError('unsupported native platform')
    if not valid:
        raise ValueError('native executable architecture mismatch')


def git(*args):
    return subprocess.check_output(['git', *args], timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--platform', required=True, choices=PLATFORMS)
    parser.add_argument('--source', required=True)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    if git('rev-parse', 'HEAD').decode().strip() != args.source:
        raise ValueError('release source must match checked-out commit')
    subprocess.run(['git', 'diff', '--exit-code', 'HEAD', '--', *SOURCE_PATHS],
                   check=True, timeout=5, stdout=subprocess.DEVNULL)
    if args.binary.is_symlink() or not args.binary.is_file():
        raise ValueError('regular native executable required')
    with args.binary.open('rb') as handle:
        data = handle.read(16 * 1024 * 1024 + 1)
    validate_binary(data, args.platform)
    sources = [p.decode() for p in git('ls-files', '-z', '--', *SOURCE_PATHS).split(b'\0') if p]
    if not 1 <= len(sources) <= 100:
        raise ValueError('release source closure outside bound')
    pins = {p: hashlib.sha256(Path(p).read_bytes()).hexdigest() for p in sources}
    manifest = {
        'source_commit': args.source, 'platform': args.platform,
        'binary_sha256': hashlib.sha256(data).hexdigest(),
        'source_pins': pins, 'fleet_configuration_included': False,
    }
    metadata = (json.dumps(manifest, sort_keys=True, indent=2) + '\n').encode()
    args.output.mkdir(parents=True, exist_ok=True)
    name = f'mesh-core-{args.platform}.tar.gz'
    archive = args.output / name
    with archive.open('xb') as output, gzip.GzipFile(fileobj=output, mode='wb', mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode='w') as tar:
            for filename, payload, mode in (('mesh-core-service', data, 0o755),
                                            ('release.json', metadata, 0o644)):
                entry = tarfile.TarInfo(filename)
                entry.size = len(payload)
                entry.mode = mode
                entry.mtime = 0
                tar.addfile(entry, io.BytesIO(payload))
    (args.output / f'{name}.sha256').write_text(hashlib.sha256(archive.read_bytes()).hexdigest() + '  ' + name + '\n')
    print(json.dumps({'archive': name, 'platform': args.platform,
                      'source': args.source, 'binary_sha256': manifest['binary_sha256'],
                      'payloads': 2, 'configuration_or_credentials': False}))


if __name__ == '__main__':
    main()
