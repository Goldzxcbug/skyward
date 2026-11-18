#!/usr/bin/env python3
"""Populate the project Cargo cache from Cargo.lock, checking crate SHA-256.

Useful when Cargo's sparse index refresh stalls. Run with Cargo stopped, then
use cargo --offline. No dependency versions or checksums are changed.
"""
from __future__ import annotations

import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import tempfile
import time
import tomllib
import urllib.request


ROOT = Path(__file__).resolve().parent.parent
CARGO_HOME = Path(os.environ.get("CARGO_HOME", ROOT / ".build-tools/cargo"))
REGISTRY = "index.crates.io-1949cf8c6b5b557f"
INDEX = CARGO_HOME / "registry/index" / REGISTRY
CACHE = CARGO_HOME / "registry/cache" / REGISTRY


def index_path(name: str) -> str:
    name = name.lower()
    if len(name) == 1:
        return f"1/{name}"
    if len(name) == 2:
        return f"2/{name}"
    if len(name) == 3:
        return f"3/{name[0]}/{name}"
    return f"{name[:2]}/{name[2:4]}/{name}"


def fetch(url: str) -> tuple[bytes, str]:
    for attempt in range(3):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": "skyward-locked-cache/1"})
            with urllib.request.urlopen(request, timeout=45) as response:
                return response.read(), response.headers.get("ETag", '""')
        except OSError:
            if attempt == 2:
                raise
            time.sleep(2**attempt)
    raise AssertionError("unreachable")


def atomic_write(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".locked-cache-", delete=False) as stream:
        temporary = Path(stream.name)
        try:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
            stream.close()
            os.replace(temporary, path)
        finally:
            temporary.unlink(missing_ok=True)


def ensure_index(name: str, versions: set[str]) -> str:
    relative = index_path(name)
    path = INDEX / ".cache" / relative
    if path.is_file():
        existing = path.read_bytes()
        if existing[:5] == b"\x03\x02\x00\x00\x00":
            present = set(existing[5:].split(b"\0")[1::2])
            if all(version.encode() in present for version in versions):
                return "cached"
    content, etag = fetch("https://index.crates.io/" + relative)
    records: list[bytes] = []
    present: set[str] = set()
    for line in content.splitlines():
        record = json.loads(line)
        present.add(record["vers"])
        records.extend((record["vers"].encode(), line))
    if not versions <= present:
        raise RuntimeError(f"index lacks locked version: {name} {sorted(versions - present)}")
    encoded = b"\x03\x02\x00\x00\x00" + b"etag: " + etag.encode() + b"\0"
    encoded += b"\0".join(records) + b"\0"
    atomic_write(path, encoded)
    return f"index {name}"


def ensure_crate(package: dict) -> str:
    name, version, checksum = package["name"], package["version"], package["checksum"]
    path = CACHE / f"{name}-{version}.crate"
    if path.is_file() and hashlib.sha256(path.read_bytes()).hexdigest() == checksum:
        return "cached"
    content, _ = fetch(f"https://static.crates.io/crates/{name}/{name}-{version}.crate")
    if hashlib.sha256(content).hexdigest() != checksum:
        raise RuntimeError(f"checksum mismatch: {name} {version}")
    atomic_write(path, content)
    return f"crate {name} {version}"


def main() -> None:
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
    packages = [p for p in lock["package"] if p.get("source") == "registry+https://github.com/rust-lang/crates.io-index"]
    unsupported = [p["source"] for p in lock["package"] if p.get("source") and p["source"] != "registry+https://github.com/rust-lang/crates.io-index"]
    if unsupported:
        raise RuntimeError(f"unsupported registry or Git source: {unsupported}")
    INDEX.mkdir(parents=True, exist_ok=True)
    CACHE.mkdir(parents=True, exist_ok=True)
    if not (INDEX / "config.json").exists():
        content, _ = fetch("https://index.crates.io/config.json")
        json.loads(content)
        atomic_write(INDEX / "config.json", content)
    versions: dict[str, set[str]] = {}
    for package in packages:
        versions.setdefault(package["name"], set()).add(package["version"])
    failures = []
    updated = 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=12) as pool:
        futures = [pool.submit(ensure_index, name, wanted) for name, wanted in versions.items()]
        futures += [pool.submit(ensure_crate, package) for package in packages]
        for future in concurrent.futures.as_completed(futures):
            try:
                result = future.result()
                if result != "cached":
                    updated += 1
                    print(result, flush=True)
            except Exception as error:
                failures.append(str(error))
                print(f"FAILED: {error}", flush=True)
    if failures:
        raise SystemExit(f"{len(failures)} downloads failed; rerun to resume verified files")
    print(f"Ready: {len(packages)} locked registry packages verified; {updated} cache entries fetched.", flush=True)


if __name__ == "__main__":
    main()
