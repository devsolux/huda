#!/usr/bin/env python3
"""Carry one actual Cargo lock across native jobs; never manufacture package checksums."""
import argparse
import hashlib
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[1]


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("write", "verify", "accept"))
    parser.add_argument("source", nargs="?")
    args = parser.parse_args()
    lock = ROOT / "Cargo.lock"
    record = ROOT / "Cargo.lock.sha256"
    if args.action == "accept":
        if not args.source:
            parser.error("accept needs the downloaded lock directory")
        source = ROOT / args.source
        expected = (source / "Cargo.lock.sha256").read_text(encoding="utf-8").split()[0]
        actual = digest(source / "Cargo.lock")
        if expected != actual:
            raise ValueError("Downloaded lock differs from the resolver's digest")
        if lock.exists() and digest(lock) != actual:
            raise ValueError("Refusing to replace a different committed Cargo.lock")
        shutil.copyfile(source / "Cargo.lock", lock)
        shutil.copyfile(source / "Cargo.lock.sha256", record)
    elif args.action == "write":
        record.write_text(f"{digest(lock)}  Cargo.lock\n", encoding="utf-8")
    else:
        expected = record.read_text(encoding="utf-8").split()[0]
        if digest(lock) != expected:
            raise ValueError("Cargo.lock changed after shared dependency resolution")
    print(f"Cargo.lock SHA-256: {digest(lock)}")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError) as error:
        print(f"Lock verification failed: {error}", file=sys.stderr)
        raise SystemExit(2) from error
