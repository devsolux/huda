#!/usr/bin/env python3
"""Reject wrong-architecture CI or Rosetta before qualifying a native target."""
import subprocess
import sys
from package_desktop import native_host, TARGETS

if len(sys.argv) != 2 or sys.argv[1] not in TARGETS:
    raise SystemExit("Provide exactly one supported Rust target")
target = sys.argv[1]
native_host(target)
result = subprocess.run(["rustc", "-vV"], text=True, capture_output=True, check=True, timeout=20)
if f"host: {target}\n" not in result.stdout:
    raise SystemExit(f"Selected Rust is not native {target}: {result.stdout}")
print(f"Native runner and compiler match: {target}")
