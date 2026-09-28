from __future__ import annotations

import hashlib
import importlib.util
import io
from pathlib import Path
import tarfile
import tempfile
import unittest
import zipfile


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "unpack_release_product", ROOT / "scripts" / "unpack-release-product.py"
)
assert SPEC is not None and SPEC.loader is not None
UNPACK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(UNPACK)


def write_sidecar(archive: Path, digest: str | None = None, name: str | None = None) -> None:
    digest = digest or hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_name(archive.name + ".sha256").write_text(
        f"{digest}  {name or archive.name}\n", encoding="utf-8"
    )


def add_tar_file(bundle: tarfile.TarFile, name: str, data: bytes = b"x", mode: int = 0o755) -> None:
    info = tarfile.TarInfo(name)
    info.size = len(data)
    info.mode = mode
    bundle.addfile(info, io.BytesIO(data))


def product_tar(path: Path, extra: callable | None = None) -> Path:
    with tarfile.open(path, "w:gz") as bundle:
        add_tar_file(bundle, "mesh-bundle/mesh-llm")
        add_tar_file(bundle, "mesh-bundle/native-runtimes/cpu/manifest.json", b"{}", 0o644)
        if extra is not None:
            extra(bundle)
    return path


class UnpackReleaseProductTests(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.out = self.tmp / "out"

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def test_unpacks_verified_tar_product(self) -> None:
        archive = product_tar(self.tmp / "mesh-llm-v1.2.3-aarch64-apple-darwin.tar.gz")
        write_sidecar(archive)

        bundle = UNPACK.unpack(archive, self.out)

        self.assertEqual(bundle, self.out / "mesh-bundle")
        host = bundle / "mesh-llm"
        self.assertTrue(host.is_file())
        self.assertTrue(host.stat().st_mode & 0o100)

    def test_unpacks_verified_zip_product(self) -> None:
        archive = self.tmp / "mesh-llm-v1.2.3-x86_64-pc-windows-msvc.zip"
        with zipfile.ZipFile(archive, "w") as bundle:
            bundle.writestr("mesh-bundle/mesh-llm.exe", b"MZ")
            bundle.writestr("mesh-bundle/native-runtimes/cpu/manifest.json", b"{}")
        write_sidecar(archive)

        bundle = UNPACK.unpack(archive, self.out)

        self.assertTrue((bundle / "mesh-llm.exe").is_file())

    def test_rejects_missing_or_mismatched_checksum(self) -> None:
        archive = product_tar(self.tmp / "mesh-llm-v1.2.3-aarch64-apple-darwin.tar.gz")
        with self.assertRaisesRegex(UNPACK.ProductError, "missing checksum sidecar"):
            UNPACK.unpack(archive, self.out)

        write_sidecar(archive, digest="0" * 64)
        with self.assertRaisesRegex(UNPACK.ProductError, "checksum mismatch"):
            UNPACK.unpack(archive, self.out)

        write_sidecar(archive, name="other.tar.gz")
        with self.assertRaisesRegex(UNPACK.ProductError, "names other.tar.gz"):
            UNPACK.unpack(archive, self.out)

        archive.with_name(archive.name + ".sha256").write_text("garbage\n", encoding="utf-8")
        with self.assertRaisesRegex(UNPACK.ProductError, "malformed checksum sidecar"):
            UNPACK.unpack(archive, self.out)
        self.assertFalse(self.out.exists())

    def test_rejects_members_that_escape_or_link(self) -> None:
        escaping = product_tar(
            self.tmp / "escape.tar.gz",
            lambda bundle: add_tar_file(bundle, "../evil"),
        )
        write_sidecar(escaping)
        with self.assertRaisesRegex(UNPACK.ProductError, "escapes the output directory"):
            UNPACK.unpack(escaping, self.out)

        def add_link(bundle: tarfile.TarFile) -> None:
            info = tarfile.TarInfo("mesh-bundle/link")
            info.type = tarfile.SYMTYPE
            info.linkname = "/etc/passwd"
            bundle.addfile(info)

        linking = product_tar(self.tmp / "link.tar.gz", add_link)
        write_sidecar(linking)
        with self.assertRaisesRegex(UNPACK.ProductError, "contains a link"):
            UNPACK.unpack(linking, self.out)

        zipped = self.tmp / "escape.zip"
        with zipfile.ZipFile(zipped, "w") as bundle:
            bundle.writestr("C:/evil.exe", b"MZ")
        write_sidecar(zipped)
        with self.assertRaisesRegex(UNPACK.ProductError, "escapes the output directory"):
            UNPACK.unpack(zipped, self.out)

    def test_rejects_product_without_host_or_runtime(self) -> None:
        archive = self.tmp / "no-runtime.tar.gz"
        with tarfile.open(archive, "w:gz") as bundle:
            add_tar_file(bundle, "mesh-bundle/mesh-llm")
        write_sidecar(archive)
        with self.assertRaisesRegex(UNPACK.ProductError, "has no runtime"):
            UNPACK.unpack(archive, self.out)

        archive = self.tmp / "no-host.tar.gz"
        with tarfile.open(archive, "w:gz") as bundle:
            add_tar_file(bundle, "mesh-bundle/native-runtimes/cpu/manifest.json", b"{}")
        write_sidecar(archive)
        with self.assertRaisesRegex(UNPACK.ProductError, "no mesh-llm executable"):
            UNPACK.unpack(archive, self.tmp / "out2")

    def test_main_reports_errors_and_bundle_path(self) -> None:
        archive = product_tar(self.tmp / "mesh-llm-v1.2.3-aarch64-apple-darwin.tar.gz")
        self.assertEqual(UNPACK.main([str(archive), str(self.out)]), 1)
        write_sidecar(archive)
        self.assertEqual(UNPACK.main([str(archive), str(self.out)]), 0)


if __name__ == "__main__":
    unittest.main()
