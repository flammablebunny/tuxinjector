# Mode Backgrounds

Each mode has a `background` block that fills the **margins around the game viewport** - the letterboxed area you see in resized modes (Thin, Tall, windowed) where the game doesn't cover the whole screen. The game itself always occupies the centre; the background only paints what's around it.

```lua
background = {
  selectedMode = "image",   -- "none" | "color" | "image" | "gradient"
  image = "~/Pictures/wall.png",
  imageFit = "fill",
  color = { 0, 0, 0, 0 },   -- matte for letterboxed image fits
}
```

## Image backgrounds

Set `selectedMode = "image"` and point `image` at a file. The path must be **absolute** or `~/`-prefixed.

### `imageFit`

| Value | Behaviour |
|-------|-----------|
| `"fill"` *(default)* | Cover the screen, preserve aspect ratio, crop the overflow. |
| `"fit"` | Fit inside the screen, preserve aspect ratio, letterbox the remainder. |
| `"stretch"` | Stretch to the screen, ignoring aspect ratio. |
| `"center"` | Native size, centred (crops if larger than the screen, letterboxes if smaller). |

For the letterboxing fits (`"fit"` and `"center"`), `background.color` is used as the **matte** colour behind the bands. With `fill`/`stretch` the image covers everything, so the matte isn't shown.

!!! note "Rendered zero-copy"
    The image is uploaded to a GL texture once (re-uploaded only when the file changes) and drawn as margin strips with the correct sub-region of the image, so it reads as one continuous wallpaper with the game on top - no per-frame copies.

## Other background modes

- `"none"` - transparent (the area shows whatever's behind the overlay).
- `"color"` - a solid `color` (RGBA, 0–255).
- `"gradient"` - an animated gradient (`gradientStops`, `gradientAngle`, `gradientAnimation`, `gradientAnimationSpeed`, `gradientColorFade`).
