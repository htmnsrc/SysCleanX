"""Resize raw app screenshots for the README and build the GitHub social preview.

Usage: python3 docs/make_previews.py <raw-screenshot-dir> <docs-dir>   (see make_previews.sh)
"""
import sys
from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parent.parent
ICON = ROOT / "crates" / "gui" / "assets" / "icon.png"
TOP, BOTTOM = (0x8B, 0x7C, 0xF6), (0x2E, 0x25, 0x7A)  # violet accent -> deep indigo


def font(size, bold=False):
    for path, index in [("/System/Library/Fonts/Helvetica.ttc", 1 if bold else 0),
                        ("C:/Windows/Fonts/segoeuib.ttf" if bold else "C:/Windows/Fonts/segoeui.ttf", 0),
                        ("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf" if bold
                         else "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 0)]:
        try:
            return ImageFont.truetype(path, size, index=index)
        except OSError:
            continue
    return ImageFont.load_default()


def screenshots(raw: Path, out: Path, width=1600):
    out.mkdir(parents=True, exist_ok=True)
    for src in sorted(raw.glob("*.png")):
        img = Image.open(src).convert("RGB")
        if img.width > width:
            img = img.resize((width, round(img.height * width / img.width)), Image.LANCZOS)
        img.save(out / src.name, optimize=True)
        print("screenshot", out / src.name)


def rounded(img, radius):
    mask = Image.new("L", img.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, *img.size), radius, fill=255)
    out = img.convert("RGBA")
    out.putalpha(mask)
    return out


def social_preview(shot: Path, out: Path):
    """1280x640 image for GitHub > Settings > Social preview."""
    w, h = 1280, 640
    bg = Image.new("RGB", (w, h))
    px = bg.load()
    for y in range(h):
        for x in range(w):
            t = min(1.0, (x / w) * 0.6 + (y / h) * 0.4)
            px[x, y] = tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM))
    canvas = bg.convert("RGBA")

    icon = Image.open(ICON).convert("RGBA").resize((112, 112), Image.LANCZOS)
    canvas.alpha_composite(icon, (72, 150))
    d = ImageDraw.Draw(canvas)
    d.text((72, 290), "SysCleanX", font=font(72, bold=True), fill=(255, 255, 255, 255))
    d.text((76, 380), "Fast, safe PC cleaner for Windows", font=font(28), fill=(255, 255, 255, 220))
    d.text((76, 424), "Clean · Uninstall · Startup · Debloat", font=font(24), fill=(255, 255, 255, 170))

    app = Image.open(shot).convert("RGB")
    app_w = 680
    app = app.resize((app_w, round(app.height * app_w / app.width)), Image.LANCZOS)
    app = rounded(app, 18)
    x, y = w - app_w - 40, (h - app.height) // 2 + 30
    shadow = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle((x + 6, y + 14, x + app_w + 6, y + app.height + 14), 18, fill=(10, 6, 40, 150))
    canvas.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(18)))
    canvas.alpha_composite(app, (x, y))
    canvas.convert("RGB").save(out / "social-preview.png", optimize=True)
    print("social preview", out / "social-preview.png")


if __name__ == "__main__":
    raw, docs = Path(sys.argv[1]), Path(sys.argv[2])
    screenshots(raw, docs / "screenshots")
    social_preview(raw / "clean.png", docs)
