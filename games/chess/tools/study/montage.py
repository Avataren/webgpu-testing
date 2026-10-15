"""Paste preview renders side by side: montage.py out.png a.png b.png ..."""
import sys
from PIL import Image
ims = [Image.open(f) for f in sys.argv[2:]]
h = max(i.height for i in ims)
c = Image.new("RGB", (sum(i.width for i in ims), h))
x = 0
for i in ims:
    c.paste(i, (x, 0))
    x += i.width
c.save(sys.argv[1])
