#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["pillow"]
# ///
"""Build the macOS application icon (AppIcon.icns) from a logo in assets/.

Resizes the 1024x1024 logo into the ten representations an `.iconset` needs and runs
`iconutil -c icns` over it. macOS-only, by definition.

Run it through `just`: `just icns`.
"""

from __future__ import annotations

import argparse
import subprocess
import tempfile
from pathlib import Path

from PIL import Image

REPO = Path(__file__).resolve().parents[1]
DEFAULT_INPUT = REPO / "assets" / "logo-white-on-blue.png"
DEFAULT_OUTPUT = REPO / "target" / "AppIcon.icns"

# The ten files `iconutil` expects: name -> pixel size. The @2x entries double the
# nominal size because the source is 1024x1024 and iconset wants it at every scale.
REPRESENTATIONS = {
    "icon_16x16.png": 16,
    "icon_16x16@2x.png": 32,
    "icon_32x32.png": 32,
    "icon_32x32@2x.png": 64,
    "icon_128x128.png": 128,
    "icon_128x128@2x.png": 256,
    "icon_256x256.png": 256,
    "icon_256x256@2x.png": 512,
    "icon_512x512.png": 512,
    "icon_512x512@2x.png": 1024,
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, default=DEFAULT_INPUT, help="1024x1024 logo PNG")
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT, help="the .icns to write")
    args = parser.parse_args()

    source = Image.open(args.input)
    if source.size != (1024, 1024):
        parser.error(f"{args.input} is {source.size}; an .icns wants a 1024x1024 source")

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        iconset = Path(tmp) / "AppIcon.iconset"
        iconset.mkdir()
        for name, size in REPRESENTATIONS.items():
            source.resize((size, size), Image.LANCZOS).save(iconset / name)
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(args.output)],
            check=True,
        )

    print(f"{args.output}: {args.input.name} scaled to {len(REPRESENTATIONS)} sizes")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())