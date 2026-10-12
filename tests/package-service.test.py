"""Fast unit: expected<1s/max5s; platform validation, no runtime claim."""
import importlib.util
from pathlib import Path
import struct
import unittest

SPEC = importlib.util.spec_from_file_location('package_service', Path(__file__).resolve().parents[1] / 'scripts/package-service.py')
p = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(p)


class NativePackageTests(unittest.TestCase):
    def test_exact_native_platforms(self):
        linux = bytearray(64)
        linux[:6] = b'\x7fELF\x02\x01'
        struct.pack_into('<H', linux, 18, 62)
        p.validate_binary(linux, 'linux-x86_64')
        for name, cpu in (('macos-x86_64', 0x1000007), ('macos-aarch64', 0x100000c)):
            data = bytearray(64)
            data[:4] = b'\xcf\xfa\xed\xfe'
            struct.pack_into('<I', data, 4, cpu)
            p.validate_binary(data, name)
            with self.assertRaises(ValueError):
                p.validate_binary(data, 'linux-x86_64')

    def test_wrong_architecture_or_endianness_refuses(self):
        for magic, cpu, name in ((b'\xcf\xfa\xed\xfe', 0x1000007, 'macos-aarch64'),
                                 (b'\xfe\xed\xfa\xcf', 0x100000c, 'macos-aarch64')):
            data = bytearray(64)
            data[:4] = magic
            struct.pack_into('<I', data, 4, cpu)
            with self.assertRaises(ValueError):
                p.validate_binary(data, name)

    def test_truncated_nonbinary_and_oversize_refuse(self):
        for data in (b'\x7fELF', b'not an executable' * 2, bytes(16 * 1024 * 1024 + 1)):
            with self.assertRaises(ValueError):
                p.validate_binary(data, 'linux-x86_64')


if __name__ == '__main__':
    unittest.main()
