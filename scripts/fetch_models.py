#!/usr/bin/env python3
"""Download the models listed in models/manifest.toml into models/weights/.

Usage: python3 scripts/fetch_models.py [--opt-in] [--check]
  --opt-in  also fetch models with distribution = "opt-in" (restricted licenses)
  --check   only validate the manifest (CI)

A model with a `[model.build]` table is not hosted as ONNX: its checkpoint is downloaded into
models/build/ and the export command it names is printed (it needs PyTorch, see the script).
"""
import hashlib
import sys
import tomllib
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "models" / "manifest.toml"
WEIGHTS = ROOT / "models" / "weights"
BUILD = ROOT / "models" / "build"
KNOWN_LICENSES = {"MIT", "Apache-2.0", "BSD-3-Clause", "GPL-3.0", "CC-BY-4.0", "CC-BY-NC-4.0"}
RESTRICTED = {"CC-BY-NC-4.0"}


def validate(models):
    errors = []
    for m in models:
        if m["license"] not in KNOWN_LICENSES:
            errors.append(f"{m['id']}: unknown license {m['license']}")
        if m["license"] in RESTRICTED and m["distribution"] != "opt-in":
            errors.append(f"{m['id']}: {m['license']} must be distribution = \"opt-in\"")
        for f in m["file"]:
            if "url" in f and "sha256" not in f:
                errors.append(f"{m['id']}: {f['name']} has a url but no sha256")
        if m["distribution"] not in ("default", "opt-in"):
            errors.append(f"{m['id']}: bad distribution {m['distribution']}")
    return errors


def download(url, dest, digest):
    """Downloads `url` to `dest` unless it is already there with the right checksum."""
    if dest.exists() and (digest is None or sha256(dest) == digest):
        print(f"ok   {dest.name}")
        return
    print(f"get  {dest.name}")
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_suffix(dest.suffix + ".part")
    urllib.request.urlretrieve(url, tmp)
    if digest is not None and sha256(tmp) != digest:
        tmp.unlink()
        sys.exit(f"checksum mismatch for {dest.name}")
    tmp.rename(dest)


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def main():
    models = tomllib.loads(MANIFEST.read_text())["model"]
    errors = validate(models)
    if errors:
        sys.exit("\n".join(errors))
    if "--check" in sys.argv:
        print(f"manifest ok ({len(models)} models)")
        return
    WEIGHTS.mkdir(parents=True, exist_ok=True)
    for m in models:
        if m["distribution"] == "opt-in" and "--opt-in" not in sys.argv:
            print(f"skip {m['id']} (opt-in, license {m['license']})")
            continue
        if "build" in m:
            b = m["build"]
            missing = [f["name"] for f in m["file"] if not (WEIGHTS / f["name"]).exists()]
            if missing:
                download(b["checkpoint_url"], BUILD / b["checkpoint"], b.get("checkpoint_sha256"))
                print(f"todo {m['id']}: build {', '.join(missing)} with\n     {b['command']}")
            else:
                print(f"ok   {m['id']} (built)")
            continue
        if "manual" in m:
            print(f"todo {m['id']}: {m['manual']}")
            continue
        for f in m["file"]:
            download(f["url"], WEIGHTS / f["name"], f.get("sha256"))


if __name__ == "__main__":
    main()
