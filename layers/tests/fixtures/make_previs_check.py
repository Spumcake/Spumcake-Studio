#!/usr/bin/env python3
"""Makes the pre-vis check's fixture with Spumcake-Skills' own method, so layers/src/edges.rs is compared with it,
not with a second method written beside it. The lines marked below are build-animatic/scripts/overlay.py's
per-frame lines at Spumcake-Skills 508f86a, unchanged, applied to one frame. Run from this folder:
    python3 -I make_previs_check.py
It writes render.png and two pre-vis renders made here from shapes, previs.png in grey and previs-colour.png in
colour, so the grey conversion is compared too, and expected.png and expected-colour.png."""
from PIL import Image, ImageDraw, ImageFilter

size = (96, 64)
render = Image.new("RGB", size, (40, 60, 90))
d = ImageDraw.Draw(render)
d.rectangle([10, 8, 50, 40], fill=(200, 160, 90))
d.ellipse([55, 20, 85, 55], fill=(230, 230, 210))
for x in range(size[0]):
    d.point((x, 60), fill=(x * 2 % 256, 120, 30))
previs = Image.new("RGB", size, (128, 128, 128))
d = ImageDraw.Draw(previs)
d.rectangle([12, 10, 48, 38], fill=(180, 180, 180))
d.line([0, 50, 95, 46], fill=(70, 70, 70), width=2)
d.rectangle([60, 25, 80, 50], outline=(30, 30, 30))
colour = Image.new("RGB", size, (90, 140, 60))
d = ImageDraw.Draw(colour)
d.rectangle([5, 5, 40, 30], fill=(200, 40, 40))
d.ellipse([50, 10, 90, 58], fill=(30, 60, 220))
d.line([0, 62, 95, 40], fill=(250, 240, 20), width=3)
render.save("render.png")
previs.save("previs.png")
colour.save("previs-colour.png")

for name, previs in (("expected.png", previs), ("expected-colour.png", colour)):
    img, pre_col = render.convert("RGB"), (0, 220, 255)
    # overlay.py, unchanged:
    g = previs.convert("L")
    edges = g.filter(ImageFilter.FIND_EDGES).point(lambda v: 255 if v > 18 else 0)
    img = Image.composite(Image.blend(img, Image.new("RGB", size, pre_col), 0.85), img, edges)
    # end of overlay.py's lines
    img.save(name)
