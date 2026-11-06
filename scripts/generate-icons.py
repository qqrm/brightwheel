#!/usr/bin/env python3
"""Generate the fixed BrightWheel application icon from the original bulb.

Run with Python and Pillow when changing icon artwork; neither is needed to
build or run BrightWheel. The original bulb assets remain separate because the
tray colors that bulb and draws its dots according to the monitor brightness.
The application and notification identity always use the five-dot, 100% icon.

The color and indicator geometry match src/tray/brightness_icon.rs at 32px.
The three-pixel bulb offset scales with the other icon sizes.
"""

from pathlib import Path

from PIL import Image


ASSETS = Path(__file__).resolve().parents[1] / "assets"
ON = (255, 212, 59)
INDICATORS = (
    (2, 4, 14, 16),
    (5, 7, 3, 5),
    (15, 17, 0, 2),
    (25, 27, 3, 5),
    (28, 30, 14, 16),
)


def brand_icon(source: Image.Image) -> Image.Image:
    """Apply the existing full-brightness colors and all five indicators."""
    source = source.convert("RGBA")
    width, height = source.size
    offset = max(1, (height * 3 + 16) // 32)
    result = Image.new("RGBA", source.size)
    result.paste(source, (0, offset))
    pixels = result.load()

    for y in range(height):
        for x in range(width):
            red, green, blue, alpha = pixels[x, y]
            if y >= height * 11 // 16 and (red or green or blue):
                pixels[x, y] = (0, 0, 0, max(alpha, 224))
            elif alpha and red > blue + 20 and green > blue + 10:
                pixels[x, y] = (*ON, alpha)

            scaled_x, scaled_y = x * 32 // width, y * 32 // height
            if any(
                left <= scaled_x <= right and top <= scaled_y <= bottom
                for left, right, top, bottom in INDICATORS
            ):
                # PNG stores straight alpha; Windows premultiplies on loading.
                pixels[x, y] = (*ON, 230)

    return result


def main() -> None:
    with Image.open(ASSETS / "brightwheel-bulb.ico") as source:
        sizes = sorted(source.ico.sizes())
        frames = [brand_icon(source.ico.getimage(size)) for size in sizes]
    frames[-1].save(
        ASSETS / "brightwheel.ico",
        format="ICO",
        sizes=sizes,
        append_images=frames[:-1],
    )
    with Image.open(ASSETS / "brightwheel-bulb.png") as source:
        brand_icon(source).save(ASSETS / "brightwheel.png")


if __name__ == "__main__":
    main()
