"""Generate the quack app icons in icons/ (Tauri's standard set).

Loads the transparent icon PNG (source-icon.png or ../../Quack Terminal Duck Icon.png)
and generates the standard Tauri icon set. Run from crates/quack-gui:

	python3 icons/make_icons.py
"""

from pathlib import Path
from PIL import Image

S = 512


def write(out: Path, img: Image.Image):
	out.parent.mkdir(parents=True, exist_ok=True)
	fmt = out.suffix.lstrip(".").lower()
	if fmt == "ico":
		img.save(out, sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
	elif fmt == "icns":
		img.save(out, append_images=[img.resize((s, s), Image.LANCZOS) for s in (512, 256, 128, 64, 32, 16)])
	else:
		img.save(out)


def get_source_icon(base: Path) -> Image.Image:
	candidates = [
		base / "source-icon.png",
		base.parents[1] / "Quack Terminal Duck Icon.png",
	]
	for cand in candidates:
		if cand.exists():
			return Image.open(cand).convert("RGBA")
	raise FileNotFoundError(f"Source icon not found in: {candidates}")


def main():
	base = Path(__file__).parent
	duck = get_source_icon(base)

	master = duck.resize((S, S), Image.LANCZOS)
	small = {
		32: duck.resize((32, 32), Image.LANCZOS),
		128: duck.resize((128, 128), Image.LANCZOS),
		256: duck.resize((256, 256), Image.LANCZOS),
	}
	write(base / "icon.png", master)
	write(base / "32x32.png", small[32])
	write(base / "128x128.png", small[128])
	write(base / "128x128@2x.png", small[256])
	write(base / "icon.ico", master)
	write(base / "icon.icns", master)
	print(f"wrote icons to {base}")


if __name__ == "__main__":
	main()
