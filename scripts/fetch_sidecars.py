#!/usr/bin/env python3
"""Download the desktop sidecars pinned in apps/desktop/src-tauri/sidecars.toml.

Usage: python3 scripts/fetch_sidecars.py [--target <rust-triple>] [--check]
  --target  the Rust target triple to fetch for (default: this machine's, from `rustc -vV`)
  --check   only validate the manifest (CI)

Files land in apps/desktop/src-tauri/binaries/ with Tauri's externalBin names
(`yt-dlp-<triple>[.exe]`, ...), each checked against its pinned sha256. Bundle them with
`pnpm --filter desktop tauri build --config src-tauri/tauri.bundle.conf.json`.
"""
import os
import stat
import subprocess
import sys
import tomllib
from pathlib import Path

# Shares the checksummed downloader with fetch_models.py (works under `python -I` too).
sys.path.insert(0, str(Path(__file__).resolve().parent))
from fetch_models import download  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
TAURI = ROOT / "apps" / "desktop" / "src-tauri"
MANIFEST = TAURI / "sidecars.toml"
BINARIES = TAURI / "binaries"
TOOLS = ("yt-dlp", "ffmpeg", "ffprobe")


def host_triple():
    out = subprocess.run(["rustc", "-vV"], capture_output=True, text=True, check=True).stdout
    return next(line.split(": ", 1)[1] for line in out.splitlines() if line.startswith("host: "))


def validate(manifest):
    errors = []
    for triple, files in manifest["target"].items():
        for name in (*TOOLS, "license"):
            entry = files.get(name)
            if entry is None:
                errors.append(f"{triple}: missing {name}")
            elif not entry["url"].startswith("https://github.com/") or len(entry["sha256"]) != 64:
                errors.append(f"{triple}: {name} must be a GitHub release URL with a sha256")
    return errors


def main():
    manifest = tomllib.loads(MANIFEST.read_text())
    errors = validate(manifest)
    if errors:
        sys.exit("\n".join(errors))
    if "--check" in sys.argv:
        print(f"sidecars ok ({len(manifest['target'])} targets)")
        return
    triple = sys.argv[sys.argv.index("--target") + 1] if "--target" in sys.argv else host_triple()
    files = manifest["target"].get(triple)
    if files is None:
        sys.exit(f"no sidecars pinned for {triple}")
    exe = ".exe" if "windows" in triple else ""
    for tool in TOOLS:
        dest = BINARIES / f"{tool}-{triple}{exe}"
        download(files[tool]["url"], dest, files[tool]["sha256"])
        dest.chmod(dest.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    download(files["license"]["url"], BINARIES / "ffmpeg-LICENSE.txt", files["license"]["sha256"])
    print(f"sidecars ready for {triple} in {os.path.relpath(BINARIES, ROOT)}")


if __name__ == "__main__":
    main()
