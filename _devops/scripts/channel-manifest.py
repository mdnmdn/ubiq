#!/usr/bin/env python3
"""Write the update-channel manifest (<channel>.json) for one release. Stdlib only.

Each artifact file names its platform by file name; a platform appears only if its file is given.
"""
import argparse
import hashlib
import json
import os
import sys
from datetime import datetime, timezone

# artifact file name -> platform key. The updater downloads these two.
PLATFORMS = {
    "Ubiq-macos-arm64.zip": "macos-aarch64",
    "Ubiq-Setup-x86_64.exe": "windows-x86_64",
}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--channel", required=True, choices=["stable", "beta", "nightly"])
    ap.add_argument("--version", required=True, help="no leading v")
    ap.add_argument("--tag", required=True)
    ap.add_argument("--repo", required=True, help="owner/name")
    ap.add_argument("--out", help="output file (default <channel>.json)")
    ap.add_argument("files", nargs="+", help="artifact files")
    a = ap.parse_args()

    base = f"https://github.com/{a.repo}/releases"
    platforms = {}
    for path in a.files:
        name = os.path.basename(path)
        key = PLATFORMS.get(name)
        if key is None:
            continue
        with open(path, "rb") as f:
            digest = hashlib.sha256(f.read()).hexdigest()
        platforms[key] = {
            "url": f"{base}/download/{a.tag}/{name}",
            "sha256": digest,
            "size": os.path.getsize(path),
        }
    if not platforms:
        print("no updatable artifact among the inputs", file=sys.stderr)
        return 1

    manifest = {
        "channel": a.channel,
        "version": a.version.removeprefix("v"),
        "published": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "notes_url": f"{base}/tag/{a.tag}",
        "platforms": platforms,
    }
    with open(a.out or f"{a.channel}.json", "w") as f:
        json.dump(manifest, f, indent=2)
        f.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
