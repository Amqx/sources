from __future__ import annotations

import os
import sys

from PIL import Image


def process_image(
    input_path: str, output_path: str | None = None, size: tuple[int, int] = (128, 128)
) -> None:
    img = Image.open(input_path).convert("RGBA")

    # Create white background
    white_bg = Image.new("RGBA", img.size, (255, 255, 255, 255))

    # Composite transparency onto white
    combined = Image.alpha_composite(white_bg, img)

    # Convert to RGB (remove alpha)
    rgb_img = combined.convert("RGB")

    # Resize to target size
    resized = rgb_img.resize(size, Image.LANCZOS)

    # Determine output path
    if output_path is None:
        base, ext = os.path.splitext(input_path)
        output_path = f"{base}_white_128.png"

    resized.save(output_path)
    print(f"Saved: {output_path}")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python script.py image1.png image2.png ...")
        sys.exit(1)

    for path in sys.argv[1:]:
        process_image(path)
