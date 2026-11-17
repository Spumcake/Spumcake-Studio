#!/usr/bin/env python3
"""Makes the take-over-animatic fixture with Spumcake-Skills' own method, so layers/src/edges.rs's line_art and
over_animatic are compared with it, not with a second method written beside it. The lines marked below are
build-animatic/scripts/overlay.py's per-frame lines at Spumcake-Skills 508f86a, unchanged, applied to one frame,
with one animatic frame as both the line art and the pre-vis, as Studio has one animatic. Run from this folder,
after make_previs_check.py has written render.png:
    python3 -I make_take_over_animatic.py
It writes animatic.png, an animatic frame made here from shapes: dark line art over a lighter, shaded place; and
expected-line-art.png, the lines alone over render.png, and expected-over-animatic.png, the edges then the lines."""
from PIL import Image, ImageDraw, ImageFilter

render = Image.open("render.png").convert("RGB")
size = render.size
animatic = Image.new("RGB", size, (235, 232, 225))
d = ImageDraw.Draw(animatic)
d.rectangle([0, 44, 95, 63], fill=(190, 186, 178))
d.rectangle([62, 6, 88, 44], fill=(160, 150, 140))
d.ellipse([18, 10, 34, 26], outline=(20, 20, 20), width=2)
d.line([26, 26, 26, 50], fill=(30, 30, 30), width=2)
d.line([26, 34, 14, 42], fill=(60, 50, 40), width=1)
d.line([26, 34, 40, 30], fill=(127, 127, 127), width=1)
d.line([26, 50, 18, 60], fill=(128, 128, 128), width=1)
d.line([26, 50, 36, 60], fill=(10, 10, 10), width=2)
animatic.save("animatic.png")

for name, with_edges in (("expected-line-art.png", False), ("expected-over-animatic.png", True)):
    img, line_col, pre_col = render.copy(), (255, 255, 255), (0, 220, 255)
    pf = [animatic] if with_edges else []
    # overlay.py, unchanged but for reading each frame from the image made above:
    if pf:
        g = animatic.convert("L")
        edges = g.filter(ImageFilter.FIND_EDGES).point(lambda v: 255 if v > 18 else 0)
        img = Image.composite(Image.blend(img, Image.new("RGB", size, pre_col), 0.85), img, edges)
    lines = animatic.convert("L").point(lambda v: 255 if v < 128 else 0)
    img = Image.composite(Image.new("RGB", size, line_col), img, lines)
    # end of overlay.py's lines
    img.save(name)
