#!/usr/bin/env python3
"""Package already-built native binaries. No build, install, login change or data mutation.

Requires Python >=3.11. Native tools: codesign/hdiutil (macOS), dpkg-deb (Linux),
ISCC 7.1 (Windows installer). Credentials/signing/notarization are never inferred.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import struct
import subprocess
import sys
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "aarch64-apple-darwin": ("Darwin", {"arm64", "aarch64"}),
    "x86_64-apple-darwin": ("Darwin", {"x86_64", "amd64"}),
    "x86_64-pc-windows-msvc": ("Windows", {"amd64", "x86_64"}),
    "x86_64-unknown-linux-gnu": ("Linux", {"x86_64", "amd64"}),
}


def metadata(root: Path = ROOT) -> dict[str, str]:
    data = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    app = data["package"]["metadata"]["huda"]
    return {"name": app["product-name"], "id": app["identifier"],
            "version": data["package"]["version"]}


def run(*args: str | Path, timeout: int = 600) -> None:
    subprocess.run([str(arg) for arg in args], check=True, timeout=timeout)


def native_host(target: str) -> None:
    system, machines = TARGETS[target]
    if platform.system() != system or platform.machine().lower() not in machines:
        raise ValueError(f"Packaging {target} requires its native OS/architecture; cross-packaging is not qualified")
    if system == "Darwin":
        check = subprocess.run(["sysctl", "-in", "sysctl.proc_translated"], text=True,
                               capture_output=True, check=False)
        if check.stdout.strip() == "1":
            raise ValueError("Rosetta packaging is not a native Intel qualification run")


def check_binary(path: Path, target: str) -> None:
    """Reject missing, symlinked or wrong-architecture inputs before making any package."""
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"Expected a regular compiled binary: {path}")
    with path.open("rb") as source:
        header = source.read(4096)
        if "apple" in target:
            expected = 0x0100000C if target.startswith("aarch64") else 0x01000007
            valid = len(header) >= 32 and header[:4] == b"\xcf\xfa\xed\xfe" and struct.unpack_from("<I", header, 4)[0] == expected
        elif "linux" in target:
            valid = len(header) >= 64 and header[:6] == b"\x7fELF\x02\x01" and struct.unpack_from("<H", header, 18)[0] == 62
        else:
            valid = False
            if len(header) >= 64 and header[:2] == b"MZ":
                offset = struct.unpack_from("<I", header, 0x3C)[0]
                if 64 <= offset <= 1024 * 1024:
                    source.seek(offset)
                    signature = source.read(6)
                    valid = signature == b"PE\0\0\x64\x86"
    if not valid:
        raise ValueError(f"Wrong executable format or architecture for {target}: {path}")


def copy_file(source: Path, destination: Path, executable: bool = False) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    destination.chmod(0o755 if executable else 0o644)


def legal_and_build_info(destination: Path, target: str, root: Path = ROOT) -> None:
    destination.mkdir(parents=True, exist_ok=True)
    for name in ("LICENSE", "NOTICE.md"):
        copy_file(root / name, destination / name)
    lock = root / "Cargo.lock"
    if not lock.is_file():
        raise ValueError("A real Cargo.lock is required to identify the packaged dependency resolution")
    copy_file(lock, destination / "Cargo.lock")
    # The complete transitive license obligations still need release review, not just MIT copying.
    data = {**metadata(root), "target": target, "qualification": "unqualified-source-candidate",
            "cargo_lock_sha256": hashlib.sha256(lock.read_bytes()).hexdigest(),
            "source_revision": os.environ.get("GITHUB_SHA", "local-unverified"),
            "signed_release": False, "notarized": False}
    (destination / "build-info.json").write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")


def stage_mac(stage: Path, binaries: Path, target: str, root: Path = ROOT) -> Path:
    info = metadata(root)
    app = stage / f"{info['name']}.app"
    contents = app / "Contents"
    for name in ("huda", "huda-desktop"):
        check_binary(binaries / name, target)
        copy_file(binaries / name, contents / "MacOS" / name, executable=True)
    copy_file(root / "assets/icons/huda.icns", contents / "Resources/huda.icns")
    legal_and_build_info(contents / "Resources", target, root)
    plist = {"CFBundleName": info["name"], "CFBundleDisplayName": info["name"],
             "CFBundleIdentifier": info["id"], "CFBundleExecutable": "huda-desktop",
             "CFBundleIconFile": "huda.icns", "CFBundlePackageType": "APPL",
             "CFBundleShortVersionString": info["version"], "CFBundleVersion": info["version"],
             "LSMinimumSystemVersion": "13.0", "NSHighResolutionCapable": True,
             "LSApplicationCategoryType": "public.app-category.productivity"}
    (contents / "Info.plist").write_bytes(plistlib.dumps(plist, sort_keys=True))
    return app


def stage_linux(stage: Path, binaries: Path, target: str, root: Path = ROOT) -> Path:
    info = metadata(root)
    for name in ("huda", "huda-desktop"):
        check_binary(binaries / name, target)
        copy_file(binaries / name, stage / "usr/lib/huda" / name, executable=True)
        link = stage / "usr/bin" / name
        link.parent.mkdir(parents=True, exist_ok=True)
        link.symlink_to(f"../lib/huda/{name}")
    legal_and_build_info(stage / "usr/share/doc/huda", target, root)
    desktop = (root / "packaging/linux/com.devsolux.huda.desktop").read_text(encoding="utf-8")
    desktop_path = stage / f"usr/share/applications/{info['id']}.desktop"
    desktop_path.parent.mkdir(parents=True, exist_ok=True)
    desktop_path.write_text(desktop, encoding="utf-8")
    for size in (16, 24, 32, 48, 64, 128, 256, 512, 1024):
        copy_file(root / f"assets/icons/huda-{size}.png",
                  stage / f"usr/share/icons/hicolor/{size}x{size}/apps/{info['id']}.png")
    control = stage / "DEBIAN/control"
    control.parent.mkdir(parents=True, exist_ok=True)
    # Fixed Ubuntu 24.04/glibc baseline; private core still does not require a running display.
    control.write_text(
        f"Package: huda\nVersion: {info['version']}\nArchitecture: amd64\n"
        "Section: utils\nPriority: optional\nMaintainer: DevSolux\n"
        "Depends: libc6 (>= 2.39), libgcc-s1, libstdc++6, libx11-6, libx11-xcb1, "
        "libxcb1, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libegl1, libgl1, dbus-user-session\n"
        "Recommends: gnome-keyring | kwalletmanager\n"
        "Description: Huda local relation-first knowledge space\n"
        " Native desktop and private shared Core. Offline by default.\n"
        " Login launch is opt-in; removal preserves all profile data.\n", encoding="utf-8")
    return stage


def stage_windows(stage: Path, binaries: Path, target: str, root: Path = ROOT) -> Path:
    app = stage / "Huda"
    for name in ("huda.exe", "huda-desktop.exe"):
        check_binary(binaries / name, target)
        copy_file(binaries / name, app / name, executable=True)
    copy_file(root / "assets/icons/huda.ico", app / "huda.ico")
    legal_and_build_info(app, target, root)
    return app


def windows_zip(app: Path, output: Path) -> None:
    with zipfile.ZipFile(output, "x", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in sorted(app.rglob("*")):
            if path.is_file():
                archive.write(path, path.relative_to(app.parent).as_posix())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--format", choices=("all", "app", "dmg", "deb", "zip", "installer"), default="all")
    args = parser.parse_args()
    native_host(args.target)
    info = metadata()
    allowed = {"Darwin": {"all", "app", "dmg"}, "Linux": {"all", "deb"},
               "Windows": {"all", "zip", "installer"}}[platform.system()]
    if args.format not in allowed:
        raise ValueError("The requested format does not belong to this native target")
    binaries = ROOT / "target" / args.target / "release"
    if not (ROOT / "Cargo.lock").is_file():
        raise ValueError("Cargo.lock missing: resolve, review and build with the same real lock first")
    dist = ROOT / "dist" / args.target
    dist.mkdir(parents=True, exist_ok=True)
    base = f"Huda-{info['version']}-{args.target}"
    suffixes = {"Darwin": [".app", ".dmg"], "Linux": [".deb"], "Windows": ["-portable.zip", "-setup.exe"]}[platform.system()]
    if any((dist / (base + suffix)).exists() for suffix in suffixes):
        raise ValueError(f"Output exists in {dist}. Move previous build artifacts explicitly; nothing was overwritten")
    with tempfile.TemporaryDirectory(prefix=".huda-stage-", dir=dist) as temporary:
        stage = Path(temporary)
        if platform.system() == "Darwin":
            app = stage_mac(stage, binaries, args.target)
            # Ad-hoc signatures permit local testing; they are NOT Developer ID/notarization.
            for name in ("huda", "huda-desktop"):
                run("codesign", "--force", "--sign", "-", app / "Contents/MacOS" / name)
            run("codesign", "--force", "--sign", "-", app)
            run("codesign", "--verify", "--deep", "--strict", app)
            if args.format in {"all", "dmg"}:
                image_source = stage / "image"
                image_source.mkdir()
                shutil.copytree(app, image_source / "Huda.app")
                (image_source / "Applications").symlink_to("/Applications")
                run("hdiutil", "create", "-volname", "Huda", "-srcfolder", image_source,
                    "-format", "UDZO", dist / f"{base}.dmg")
                run("hdiutil", "verify", dist / f"{base}.dmg")
            if args.format in {"all", "app"}:
                shutil.copytree(app, dist / f"{base}.app")
        elif platform.system() == "Linux":
            stage_linux(stage / "root", binaries, args.target)
            run("dpkg-deb", "--root-owner-group", "--build", stage / "root", dist / f"{base}.deb")
            run("dpkg-deb", "--info", dist / f"{base}.deb")
        else:
            app = stage_windows(stage, binaries, args.target)
            if args.format in {"all", "zip"}:
                windows_zip(app, dist / f"{base}-portable.zip")
            if args.format in {"all", "installer"}:
                iscc = os.environ.get("HUDA_ISCC") or shutil.which("ISCC.exe")
                if not iscc:
                    raise ValueError("Inno Setup 7.1 ISCC.exe is required; set HUDA_ISCC to its full path")
                run(iscc, f"/DAppVersion={info['version']}", f"/DAppIdentifier={info['id']}",
                    f"/DStageDir={app}", f"/DOutputDir={dist}", f"/DOutputName={base}-setup",
                    ROOT / "packaging/windows/Huda.iss")
    # File packages only; each .app includes its own lock and build metadata.
    lines = [f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}" for p in sorted(dist.iterdir())
             if p.is_file() and p.suffix in {".dmg", ".deb", ".zip", ".exe"}]
    # LF on every host so `sha256sum -c` works on files produced by Windows runners.
    (dist / "SHA256SUMS").write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"Created native test packages in {dist}; signing/notarization and install qualification remain separate")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(f"Packaging failed: {error}", file=sys.stderr)
        raise SystemExit(2) from error
