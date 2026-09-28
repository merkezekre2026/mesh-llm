#!/usr/bin/env python3
"""Verify and unpack a published mesh-llm release archive.

Consumers that wrap an already released product (such as the desktop
installer workflow) must use the exact published bytes. This script checks the
archive against its ``<archive>.sha256`` sidecar (the ``digest  name`` format
written by ``write_checksum_sidecar`` in ``scripts/package-release.sh``),
extracts it without letting any member escape the output directory, and
confirms the ``mesh-bundle/`` layout: the host executable plus a non-empty
``native-runtimes/`` directory.

Usage:
    unpack-release-product.py <archive> <output-dir>

Prints the path of the unpacked ``mesh-bundle`` directory on success.
"""

from __future__ import annotations

import argparse
import hashlib
from pathlib import Path, PurePosixPath
import sys
import tarfile
import zipfile


BUNDLE_DIR = "mesh-bundle"
RUNTIMES_DIR = "native-runtimes"


class ProductError(Exception):
    """The archive, its checksum, or its layout is not a usable product."""


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def expected_digest(archive: Path) -> str:
    sidecar = archive.with_name(archive.name + ".sha256")
    if not sidecar.is_file():
        raise ProductError(f"missing checksum sidecar {sidecar.name}")
    fields = sidecar.read_text(encoding="utf-8").split()
    if len(fields) != 2:
        raise ProductError(f"malformed checksum sidecar {sidecar.name}")
    digest, name = fields
    if name.lstrip("*") != archive.name:
        raise ProductError(f"checksum sidecar names {name}, expected {archive.name}")
    digest = digest.lower()
    if len(digest) != 64 or any(ch not in "0123456789abcdef" for ch in digest):
        raise ProductError(f"malformed digest in {sidecar.name}")
    return digest


def verify_checksum(archive: Path) -> None:
    expected = expected_digest(archive)
    actual = sha256_file(archive)
    if actual != expected:
        raise ProductError(
            f"checksum mismatch for {archive.name}: expected {expected}, got {actual}"
        )


def safe_member_path(name: str) -> PurePosixPath:
    path = PurePosixPath(name.replace("\\", "/"))
    if path.is_absolute() or ".." in path.parts or (path.parts and ":" in path.parts[0]):
        raise ProductError(f"archive member escapes the output directory: {name}")
    return path


def extract_tar(archive: Path, output: Path) -> None:
    with tarfile.open(archive, "r:gz") as bundle:
        members = bundle.getmembers()
        for member in members:
            safe_member_path(member.name)
            if member.issym() or member.islnk():
                raise ProductError(f"archive contains a link: {member.name}")
            if not (member.isfile() or member.isdir()):
                raise ProductError(f"archive contains a special file: {member.name}")
        # Python 3.12+ also applies the stdlib "data" filter as defense in depth.
        options = {"filter": "data"} if hasattr(tarfile, "data_filter") else {}
        bundle.extractall(output, members=members, **options)


def extract_zip(archive: Path, output: Path) -> None:
    with zipfile.ZipFile(archive) as bundle:
        for name in bundle.namelist():
            safe_member_path(name)
        bundle.extractall(output)


def extract(archive: Path, output: Path) -> None:
    output.mkdir(parents=True, exist_ok=True)
    if archive.name.endswith(".tar.gz"):
        extract_tar(archive, output)
    elif archive.name.endswith(".zip"):
        extract_zip(archive, output)
    else:
        raise ProductError(f"unsupported archive type: {archive.name}")


def verify_layout(output: Path) -> Path:
    bundle = output / BUNDLE_DIR
    hosts = [bundle / "mesh-llm", bundle / "mesh-llm.exe"]
    if not any(host.is_file() for host in hosts):
        raise ProductError(f"{BUNDLE_DIR}/ has no mesh-llm executable")
    runtimes = bundle / RUNTIMES_DIR
    if not runtimes.is_dir() or not any(entry.is_dir() for entry in runtimes.iterdir()):
        raise ProductError(f"{BUNDLE_DIR}/{RUNTIMES_DIR}/ has no runtime")
    return bundle


def unpack(archive: Path, output: Path) -> Path:
    verify_checksum(archive)
    extract(archive, output)
    return verify_layout(output)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("archive", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args(argv)
    try:
        bundle = unpack(args.archive, args.output)
    except (ProductError, OSError, tarfile.TarError, zipfile.BadZipFile) as error:
        sys.stderr.write(f"error: {error}\n")
        return 1
    sys.stdout.write(f"{bundle}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
