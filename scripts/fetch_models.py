#!/usr/bin/env python3
"""Download the models listed in models/manifest.toml into models/weights/.

Usage: python3 scripts/fetch_models.py [--opt-in] [--check]
  --opt-in  also fetch models with distribution = "opt-in" (restricted licenses)
  --check   only validate the manifest (CI)
"""
import hashlib
import sys
import tomllib
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "models" / "manifest.toml"
WEIGHTS = ROOT / "models" / "weights"
KNOWN_LICENSES = {"MIT", "Apache-2.0", "BSD-3-Clause", "GPL-3.0", "CC-BY-4.0", "CC-BY-NC-4.0"}
RESTRICTED = {"CC-BY-NC-4.0"}


def validate(models):
    errors = []
    for m in models:
        if m["license"] not in KNOWN_LICENSES:
            errors.append(f"{m['id']}: unknown license {m['license']}")
        if m["license"] in RESTRICTED and m["distribution"] != "opt-in":
            errors.append(f"{m['id']}: {m['license']} must be distribution = \"opt-in\"")
        if m["distribution"] not in ("default", "opt-in"):
            errors.append(f"{m['id']}: bad distribution {m['distribution']}")
    return errors


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
        for f in m["file"]:
            dest = WEIGHTS / f["name"]
            if dest.exists() and ("sha256" not in f or sha256(dest) == f["sha256"]):
                print(f"ok   {f['name']}")
                continue
            print(f"get  {f['name']}")
            tmp = dest.with_suffix(dest.suffix + ".part")
            urllib.request.urlretrieve(f["url"], tmp)
            if "sha256" in f and sha256(tmp) != f["sha256"]:
                tmp.unlink()
                sys.exit(f"checksum mismatch for {f['name']}")
            tmp.rename(dest)


if __name__ == "__main__":
    main()
