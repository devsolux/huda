#!/usr/bin/env python3
"""Offline icon/packaging/script regressions; stdlib only. NOT a Rust/native build test.

Package tests deliberately use minimal synthetic executable HEADER fixtures in a
new temporary directory. They test layout/identity/architecture rejection, never
claim these fixtures are compiled programs or real installable release artifacts.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
import zipfile

import package_desktop as package

ROOT = Path(__file__).resolve().parents[1]


def binary_fixture(target: str) -> bytes:
    data = bytearray(256)
    if "apple" in target:
        data[:4] = b"\xcf\xfa\xed\xfe"
        struct.pack_into("<I", data, 4, 0x0100000C if target.startswith("aarch64") else 0x01000007)
    elif "linux" in target:
        data[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<H", data, 18, 62)
    else:
        data[:2] = b"MZ"
        struct.pack_into("<I", data, 0x3C, 128)
        data[128:134] = b"PE\0\0\x64\x86"
    return bytes(data)


class DesktopSourceTests(unittest.TestCase):
    def test_product_identity(self):
        self.assertEqual(package.metadata(), {"name": "Huda", "id": "com.devsolux.huda", "version": "1.0.0"})
        rust = (ROOT / "src/lib.rs").read_text()
        self.assertIn('APP_NAME: &str = "Huda"', rust)
        self.assertIn('APP_ID: &str = "com.devsolux.huda"', rust)

    def test_all_png_sizes_and_source_hash(self):
        for size in (16, 24, 32, 48, 64, 128, 256, 512, 1024):
            data = (ROOT / f"assets/icons/huda-{size}.png").read_bytes()
            self.assertEqual(data[:8], b"\x89PNG\r\n\x1a\n")
            self.assertEqual(struct.unpack_from(">II", data, 16), (size, size))
        manifest = json.loads((ROOT / "assets/icons/manifest.json").read_text())
        for name, expected in manifest["sha256"].items():
            self.assertEqual(hashlib.sha256((ROOT / "assets/icons" / name).read_bytes()).hexdigest(), expected)

    def test_windows_ico_directory_contains_all_sizes(self):
        data = (ROOT / "assets/icons/huda.ico").read_bytes()
        reserved, kind, count = struct.unpack_from("<HHH", data)
        self.assertEqual((reserved, kind), (0, 1))
        sizes = set()
        for i in range(count):
            width, height, _, _, _, _, length, offset = struct.unpack_from("<BBBBHHII", data, 6 + i * 16)
            self.assertEqual(width, height)
            self.assertLessEqual(offset + length, len(data))
            sizes.add(width or 256)
        self.assertTrue({16, 24, 32, 48, 64, 128, 256}.issubset(sizes))

    def test_macos_icns_container_is_complete(self):
        data = (ROOT / "assets/icons/huda.icns").read_bytes()
        self.assertEqual(data[:4], b"icns")
        self.assertEqual(struct.unpack_from(">I", data, 4)[0], len(data))
        offset, types = 8, set()
        while offset < len(data):
            kind, length = struct.unpack_from(">4sI", data, offset)
            self.assertGreaterEqual(length, 8)
            types.add(kind)
            offset += length
        self.assertEqual(offset, len(data))
        self.assertIn(b"ic10", types)  # 1024px/512@2x

    def test_workflow_has_four_explicit_native_targets_and_sha_actions(self):
        source = (ROOT / ".github/workflows/desktop.yml").read_text()
        self.assertEqual(set(re.findall(r"- target: ([\w-]+)", source)), set(package.TARGETS))
        actions = re.findall(r"uses: ([\w/-]+)@([^\s]+)", source)
        self.assertTrue(actions)
        for action, revision in actions:
            # GitHub's own actions plus the Sigstore installer; nothing else.
            self.assertRegex(action, r"^(actions/[\w-]+|sigstore/cosign-installer)$")
            self.assertRegex(revision, r"^[0-9a-f]{40}$")
        self.assertIn("contents: read", source)
        self.assertNotIn("pull_request_target", source)
        self.assertNotIn("continue-on-error", source)
        self.assertIn("digest-mismatch: error", source)
        self.assertIn("--locked", source)
        # Only the keyless signing job may mint an OIDC identity token, after every gate and
        # without checking out code, and it verifies each signature against this workflow.
        self.assertEqual(source.count("id-token: write"), 1)
        sign = source.split("\n  sign:\n", 1)[1].split("\n  release-assets:\n", 1)[0]
        self.assertIn("id-token: write", sign)
        self.assertIn("needs: [dependencies, formatting, native]", sign)
        self.assertNotIn("actions/checkout", sign)
        self.assertNotIn("contents: write", sign)
        self.assertRegex(sign, r"cosign-release: v\d+\.\d+\.\d+\n")
        self.assertIn("cosign verify-blob", sign)
        self.assertIn("--certificate-identity", sign)
        self.assertIn("--certificate-oidc-issuer", sign)
        # Only the final release-asset job may write, only for published releases, without
        # checking out code and without replacing existing assets.
        self.assertEqual(source.count("contents: write"), 1)
        job = source.split("\n  release-assets:\n", 1)[1]
        self.assertIn("contents: write", job)
        self.assertIn("if: github.event_name == 'release'", job)
        self.assertIn("needs: [dependencies, formatting, native, sign]", job)
        self.assertIn(".sigstore.json", job)
        self.assertNotIn("actions/checkout", job)
        self.assertNotIn("--clobber", source)

    def test_headless_feature_does_not_require_desktop(self):
        import tomllib
        data = tomllib.loads((ROOT / "Cargo.toml").read_text())
        self.assertNotIn("desktop", data["features"]["default"])
        self.assertTrue(data["dependencies"]["auto-launch"]["optional"])
        self.assertTrue(data["dependencies"]["tray-icon"]["optional"])
        self.assertEqual(data["dependencies"]["auto-launch"]["version"], "=0.6.0")
        self.assertEqual(data["dependencies"]["tray-icon"]["version"], "=0.26.1")

    def test_dev_help_has_no_native_prerequisite_side_effect(self):
        bash = shutil.which("bash")
        if not bash:
            self.skipTest("bash unavailable; native workflow validates on macOS")
        result = subprocess.run([bash, str(ROOT / "dev.sh"), "--help"], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("aarch64-apple-darwin", (ROOT / "dev.sh").read_text())
        self.assertIn("No sudo", result.stdout)

    @unittest.skipIf(sys.platform == "darwin", "Wrong-OS rejection exercised on Linux/Windows")
    def test_dev_rejects_wrong_os_before_build(self):
        bash = shutil.which("bash")
        if not bash:
            self.skipTest("bash unavailable")
        result = subprocess.run([bash, str(ROOT / "dev.sh"), "build"], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 2)
        self.assertIn("native macOS arm64", result.stderr)


class PackageLayoutTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="huda-package-fixture-")
        self.base = Path(self.temp.name)
        self.root = self.base / "source"
        self.root.mkdir()
        shutil.copyfile(ROOT / "Cargo.toml", self.root / "Cargo.toml")
        shutil.copyfile(ROOT / "LICENSE", self.root / "LICENSE")
        shutil.copyfile(ROOT / "NOTICE.md", self.root / "NOTICE.md")
        # Isolated fixture only: not a Rust dependency resolution, never included in delivery.
        (self.root / "Cargo.lock").write_text("# SYNTHETIC PACKAGE-LAYOUT TEST FIXTURE ONLY\n")
        shutil.copytree(ROOT / "assets/icons", self.root / "assets/icons")
        shutil.copytree(ROOT / "packaging/linux", self.root / "packaging/linux")
        self.binaries = self.base / "binaries"
        self.binaries.mkdir()

    def tearDown(self):
        self.temp.cleanup()

    def binaries_for(self, target):
        suffix = ".exe" if "windows" in target else ""
        for name in ("huda", "huda-desktop"):
            (self.binaries / (name + suffix)).write_bytes(binary_fixture(target))

    def test_mac_bundle_identity_and_sibling_helper(self):
        target = "aarch64-apple-darwin"
        self.binaries_for(target)
        app = package.stage_mac(self.base / "stage", self.binaries, target, self.root)
        with (app / "Contents/Info.plist").open("rb") as stream:
            info = plistlib.load(stream)
        self.assertEqual(info["CFBundleIdentifier"], "com.devsolux.huda")
        self.assertEqual(info["CFBundleShortVersionString"], "1.0.0")
        self.assertEqual(info["CFBundleExecutable"], "huda-desktop")
        self.assertTrue((app / "Contents/MacOS/huda").is_file())
        self.assertTrue((app / "Contents/Resources/huda.icns").is_file())
        self.assertNotIn("LSUIElement", info)  # ordinary recoverable desktop, not hidden-only agent

    def test_intel_bundle_uses_intel_inputs(self):
        target = "x86_64-apple-darwin"
        self.binaries_for(target)
        app = package.stage_mac(self.base / "intel", self.binaries, target, self.root)
        package.check_binary(app / "Contents/MacOS/huda", target)

    def test_windows_portable_layout(self):
        target = "x86_64-pc-windows-msvc"
        self.binaries_for(target)
        app = package.stage_windows(self.base / "windows", self.binaries, target, self.root)
        archive = self.base / "portable.zip"
        package.windows_zip(app, archive)
        with zipfile.ZipFile(archive) as contents:
            self.assertIsNone(contents.testzip())
            self.assertIn("Huda/huda.exe", contents.namelist())
            self.assertIn("Huda/huda-desktop.exe", contents.namelist())
            self.assertIn("Huda/huda.ico", contents.namelist())
            info = json.loads(contents.read("Huda/build-info.json"))
            self.assertFalse(info["signed_release"])

    @unittest.skipIf(sys.platform == "win32", "Linux package symlinks require native Unix")
    def test_linux_layout_has_no_profile_or_autostart_hooks(self):
        target = "x86_64-unknown-linux-gnu"
        self.binaries_for(target)
        stage = package.stage_linux(self.base / "linux", self.binaries, target, self.root)
        self.assertTrue((stage / "usr/bin/huda").is_symlink())
        self.assertEqual(os.readlink(stage / "usr/bin/huda"), "../lib/huda/huda")
        self.assertEqual((stage / "usr/bin/huda").resolve(), (stage / "usr/lib/huda/huda").resolve())
        self.assertTrue((stage / "usr/share/applications/com.devsolux.huda.desktop").is_file())
        self.assertFalse((stage / "DEBIAN/postinst").exists())
        self.assertFalse((stage / "DEBIAN/postrm").exists())
        self.assertFalse(list(stage.rglob("*.db")))
        self.assertNotIn("autostart", " ".join(str(p.relative_to(stage)) for p in stage.rglob("*")))

    def test_wrong_architecture_is_rejected(self):
        target = "aarch64-apple-darwin"
        self.binaries_for(target)
        with self.assertRaises(ValueError):
            package.check_binary(self.binaries / "huda", "x86_64-apple-darwin")
        with self.assertRaises(ValueError):
            package.check_binary(self.binaries / "huda", "x86_64-unknown-linux-gnu")

    def test_missing_helper_is_rejected(self):
        target = "aarch64-apple-darwin"
        self.binaries_for(target)
        (self.binaries / "huda").unlink()
        with self.assertRaises(ValueError):
            package.stage_mac(self.base / "missing", self.binaries, target, self.root)

    def test_missing_real_lock_cannot_be_ignored_by_packager(self):
        (self.root / "Cargo.lock").unlink()
        with self.assertRaisesRegex(ValueError, "Cargo.lock"):
            package.legal_and_build_info(self.base / "info", "aarch64-apple-darwin", self.root)


if __name__ == "__main__":
    unittest.main(verbosity=2)
