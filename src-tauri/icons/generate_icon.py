"""Generate the source app icon (1024x1024 PNG) for Voxtray.

Run once to (re)create `icon-source.png`, then let Tauri build the platform
icon set from it with:  npx tauri icon src-tauri/icons/icon-source.png
"""
from PIL import Image, ImageDraw

SIZE = 1024
img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
draw = ImageDraw.Draw(img)


def rounded_rect(d, box, radius, fill):
    d.rounded_rectangle(box, radius=radius, fill=fill)


# Background: rounded square with a vertical two-tone (fake gradient via bands).
top = (99, 102, 241)      # indigo-500
bottom = (139, 92, 246)   # violet-500
margin = 40
radius = 220
# Base fill
rounded_rect(draw, [margin, margin, SIZE - margin, SIZE - margin], radius, bottom)
# Gradient bands, clipped to a rounded mask.
mask = Image.new("L", (SIZE, SIZE), 0)
mdraw = ImageDraw.Draw(mask)
mdraw.rounded_rectangle([margin, margin, SIZE - margin, SIZE - margin], radius=radius, fill=255)
grad = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
gdraw = ImageDraw.Draw(grad)
h = SIZE
for y in range(h):
    t = y / h
    r = int(top[0] + (bottom[0] - top[0]) * t)
    g = int(top[1] + (bottom[1] - top[1]) * t)
    b = int(top[2] + (bottom[2] - top[2]) * t)
    gdraw.line([(0, y), (SIZE, y)], fill=(r, g, b, 255))
img.paste(grad, (0, 0), mask)
draw = ImageDraw.Draw(img)

white = (255, 255, 255, 255)
soft = (255, 255, 255, 235)

# Speaker body (left side): rectangle + triangle cone.
sp_x = 300
draw.rectangle([sp_x, 452, sp_x + 70, 572], fill=white)
draw.polygon(
    [(sp_x + 70, 452), (sp_x + 190, 372), (sp_x + 190, 652), (sp_x + 70, 572)],
    fill=white,
)

# Sound waves (arcs) to the right of the speaker.
cx, cy = sp_x + 150, 512
for i, rad in enumerate((90, 150, 210)):
    bbox = [cx - rad, cy - rad, cx + rad, cy + rad]
    draw.arc(bbox, start=-45, end=45, fill=soft, width=26)

# Text lines beneath, suggesting "selected text".
line_x0 = 300
line_w = [420, 360, 300]
for i, w in enumerate(line_w):
    y = 720 + i * 62
    draw.rounded_rectangle([line_x0, y, line_x0 + w, y + 34], radius=17, fill=soft)

img.save("icon-source.png")
print("wrote icon-source.png")
