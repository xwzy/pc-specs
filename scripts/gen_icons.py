#!/usr/bin/env python3
"""Generate multi-platform icon assets for the Tauri bundle and the web frontend.

Sources the master logo from `icon/icon.png` (1024×1024 RGBA) and renders:
  - src-tauri/icons/{32x32, 128x128, 128x128@2x}.png
  - src-tauri/icons/icon.png (1024)
  - src-tauri/icons/icon.ico (Windows multi-size)
  - src-tauri/icons/icon.icns (macOS, via iconutil if present, else Pillow ICNS)
  - public/icon.png (256, frontend favicon)
  - public/logo.png (512, in-app brand image)

Run: `python3 scripts/gen_icons.py`
Requires: Pillow (`pip install pillow`).
"""
from __future__ import annotations
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "icon" / "icon.png"
ICON_DIR = ROOT / "src-tauri" / "icons"
PUBLIC_DIR = ROOT / "public"


def load_master() -> Image.Image:
    if not SRC.exists():
        sys.exit(f"missing source: {SRC}")
    img = Image.open(SRC).convert("RGBA")
    if img.size[0] != img.size[1]:
        # 强制方形（取最大边裁切居中）
        side = max(img.size)
        bg = Image.new("RGBA", (side, side), (0, 0, 0, 0))
        bg.paste(img, ((side - img.size[0]) // 2, (side - img.size[1]) // 2))
        img = bg
    if img.size[0] < 1024:
        img = img.resize((1024, 1024), Image.LANCZOS)
    elif img.size[0] > 1024:
        img = img.resize((1024, 1024), Image.LANCZOS)
    return img


def save_pngs(master: Image.Image) -> None:
    ICON_DIR.mkdir(parents=True, exist_ok=True)
    PUBLIC_DIR.mkdir(parents=True, exist_ok=True)

    targets = {
        ICON_DIR / "32x32.png": 32,
        ICON_DIR / "128x128.png": 128,
        ICON_DIR / "128x128@2x.png": 256,
        ICON_DIR / "icon.png": 1024,
        PUBLIC_DIR / "icon.png": 256,
        PUBLIC_DIR / "logo.png": 512,
    }
    for path, size in targets.items():
        master.resize((size, size), Image.LANCZOS).save(path, "PNG", optimize=True)
        print(f"  {path.relative_to(ROOT)}  ({size}×{size})")


def save_ico(master: Image.Image) -> None:
    # Windows .ico 多尺寸打包；Pillow 自动从给定的 sizes 嵌入。
    out = ICON_DIR / "icon.ico"
    master.save(out, sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    print(f"  {out.relative_to(ROOT)}  (multi-size)")


def save_icns_via_iconutil(master: Image.Image) -> bool:
    """macOS 自带 iconutil 是生成高质量 icns 的官方方式。"""
    if not shutil.which("iconutil"):
        return False
    with tempfile.TemporaryDirectory() as td:
        iconset = Path(td) / "icon.iconset"
        iconset.mkdir()
        # Apple 规范要求的 PNG 尺寸 + Retina 倍率命名
        spec = [
            (16, "icon_16x16.png"),
            (32, "icon_16x16@2x.png"),
            (32, "icon_32x32.png"),
            (64, "icon_32x32@2x.png"),
            (128, "icon_128x128.png"),
            (256, "icon_128x128@2x.png"),
            (256, "icon_256x256.png"),
            (512, "icon_256x256@2x.png"),
            (512, "icon_512x512.png"),
            (1024, "icon_512x512@2x.png"),
        ]
        for size, name in spec:
            master.resize((size, size), Image.LANCZOS).save(iconset / name, "PNG")
        out = ICON_DIR / "icon.icns"
        try:
            subprocess.run(
                ["iconutil", "-c", "icns", str(iconset), "-o", str(out)],
                check=True,
                capture_output=True,
            )
            print(f"  {out.relative_to(ROOT)}  (via iconutil)")
            return True
        except subprocess.CalledProcessError as e:
            print(f"  iconutil failed: {e.stderr.decode(errors='ignore')[:200]}")
            return False


def save_icns_via_pillow(master: Image.Image) -> None:
    """Fallback: Pillow 也支持写 ICNS（需要特定 sizes）。"""
    out = ICON_DIR / "icon.icns"
    sizes = [(16, 16), (32, 32), (64, 64), (128, 128), (256, 256), (512, 512), (1024, 1024)]
    try:
        master.save(out, format="ICNS", sizes=sizes)
        print(f"  {out.relative_to(ROOT)}  (via Pillow)")
    except Exception as e:
        print(f"  ICNS write failed: {e}; writing empty placeholder")
        out.write_bytes(b"")


def main() -> None:
    print(f"source: {SRC.relative_to(ROOT)}")
    master = load_master()
    save_pngs(master)
    save_ico(master)
    if not save_icns_via_iconutil(master):
        save_icns_via_pillow(master)
    print("done.")


if __name__ == "__main__":
    main()
