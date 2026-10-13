#!/usr/bin/env python3
"""Only --help / --version smoke checks; never initializes a profile or calls providers."""
from pathlib import Path
import subprocess
import sys
from package_desktop import ROOT, TARGETS, check_binary, metadata, native_host

if len(sys.argv) != 2 or sys.argv[1] not in TARGETS:
    raise SystemExit("Provide exactly one native Rust target")
target = sys.argv[1]
native_host(target)
exe = ROOT / "target" / target / "release" / ("huda.exe" if "windows" in target else "huda")
check_binary(exe, target)
for argument in ("--version", "--help"):
    result = subprocess.run([str(exe), argument], capture_output=True, text=True, check=True, timeout=20)
    if argument == "--version" and metadata()["version"] not in result.stdout:
        raise SystemExit("Helper version does not match Cargo.toml")
    print(result.stdout)
print("Helper CLI startup passed; this is not GUI, keyring, IPC or installation qualification")
