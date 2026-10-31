"""Generate the installer artwork from the app icon (run once; outputs are committed).

welcome.bmp  - left panel of the Welcome/Finish pages (2x of NSIS's 164x314)
header.bmp   - top-right image of the inner pages (2x of NSIS's 150x57)
Requires Pillow.
"""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent
ICON = ROOT.parent / "crates" / "gui" / "assets" / "icon.png"
OUT = ROOT / "assets"
TOP, BOTTOM = (0x8B, 0x7C, 0xF6), (0x2E, 0x25, 0x7A)  # violet accent -> deep indigo


def font(size, bold=False):
    for path, index in [("/System/Library/Fonts/Helvetica.ttc", 1 if bold else 0),
                        ("C:/Windows/Fonts/segoeuib.ttf" if bold else "C:/Windows/Fonts/segoeui.ttf", 0)]:
        try:
            return ImageFont.truetype(path, size, index=index)
        except OSError:
            continue
    return ImageFont.load_default()


def gradient(w, h):
    img = Image.new("RGB", (w, h))
    px = img.load()
    for y in range(h):
        t = y / (h - 1)
        c = tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM))
        for x in range(w):
            px[x, y] = c
    return img


def welcome():
    w, h = 328, 628
    img = gradient(w, h).convert("RGBA")
    icon = Image.open(ICON).convert("RGBA").resize((136, 136), Image.LANCZOS)
    img.alpha_composite(icon, ((w - 136) // 2, 150))
    d = ImageDraw.Draw(img)
    title, tag = "SysCleanX", "Clean · Fast · Safe"
    f1, f2 = font(44, bold=True), font(22)
    tw = d.textlength(title, font=f1)
    d.text(((w - tw) / 2, 320), title, font=f1, fill=(255, 255, 255, 255))
    tw = d.textlength(tag, font=f2)
    d.text(((w - tw) / 2, 378), tag, font=f2, fill=(255, 255, 255, 190))
    img.convert("RGB").save(OUT / "welcome.bmp")


def header():
    w, h = 300, 114
    img = Image.new("RGBA", (w, h), (255, 255, 255, 255))
    icon = Image.open(ICON).convert("RGBA").resize((84, 84), Image.LANCZOS)
    img.alpha_composite(icon, (w - 84 - 14, (h - 84) // 2))
    img.convert("RGB").save(OUT / "header.bmp")


if __name__ == "__main__":
    OUT.mkdir(exist_ok=True)
    welcome()
    header()
    print("wrote", *(p.name for p in OUT.glob("*.bmp")))
