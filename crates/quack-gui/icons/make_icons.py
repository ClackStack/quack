"""Generate the quack app icons in icons/ (Tauri's standard set).

Palette and shapes follow ui/src/app.css: sky tile, duck-yellow duck, beak
orange, dark ink eye, and two white water lines. Run from crates/quack-gui.

	python3 icons/make_icons.py
"""

from pathlib import Path

from PIL import Image, ImageDraw

S = 1024
SUPER = 2
U = S * SUPER

SKY = (94, 200, 229, 255)
DUCK = (255, 214, 10, 255)
DUCK_LINE = (232, 207, 85, 255)
BEAK = (255, 159, 67, 255)
INK = (22, 33, 46, 255)
WHITE = (255, 255, 255, 170)


def rounded_rect(draw, box, radius, fill):
	box = [v * SUPER for v in box]
	draw.rounded_rectangle(box, radius=radius * SUPER, fill=fill)


def duck_icon() -> Image.Image:
	img = Image.new("RGBA", (U, U), (0, 0, 0, 0))
	d = ImageDraw.Draw(img)
	d.rounded_rectangle((0, 0, U - 1, U - 1), radius=int(U * 0.225), fill=SKY)

	# water lines under the duck
	for y in (860, 910):
		rounded_rect(d, (250, y, 774, y + 18), 9, WHITE)

	# body (facing right, tail up on the left)
	rounded_rect(d, (252, 480, 800, 830), 150, DUCK)
	# tail: a wedge rising from the body's left end
	d.polygon([(v * SUPER, w * SUPER) for v, w in [(300, 560), (180, 430), (380, 500)]], fill=DUCK)
	# head
	d.ellipse([v * SUPER for v in (430, 200, 750, 520)], fill=DUCK)
	# beak: a solid `>`-style wedge, back edge tucked into the head
	d.polygon(
		[(v * SUPER, w * SUPER) for v, w in [(700, 295), (935, 375), (700, 455)]],
		fill=BEAK,
	)
	# eye
	d.ellipse([v * SUPER for v in (600, 300, 668, 368)], fill=INK)
	# wing: a darker ellipse on the body
	d.ellipse([v * SUPER for v in (360, 560, 660, 760)], fill=DUCK_LINE)

	return img.resize((S, S), Image.LANCZOS)


def write(out: Path, img: Image.Image):
	out.parent.mkdir(parents=True, exist_ok=True)
	fmt = out.suffix.lstrip(".").lower()
	if fmt == "ico":
		img.save(out, sizes=[(32, 32), (128, 128), (256, 256)])
	elif fmt == "icns":
		img.save(out, append_images=[img.resize((s, s), Image.LANCZOS) for s in (512, 256, 128)])
	else:
		img.save(out)


def main():
	base = Path(__file__).parent
	duck = duck_icon()
	small = {
		32: duck.resize((32, 32), Image.LANCZOS),
		128: duck.resize((128, 128), Image.LANCZOS),
		256: duck.resize((256, 256), Image.LANCZOS),
	}
	write(base / "icon.png", duck)
	write(base / "32x32.png", small[32])
	write(base / "128x128.png", small[128])
	write(base / "128x128@2x.png", small[256])
	write(base / "icon.ico", duck)
	write(base / "icon.icns", duck)
	print(f"wrote icons to {base}")


if __name__ == "__main__":
	main()
