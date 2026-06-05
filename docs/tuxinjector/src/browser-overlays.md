# Browser Overlays

Browser overlays render a live web page directly into the game as an overlay. The page is rendered off-screen by a small **WebKitGTK** helper process (`tuxinjector-browser`) and composited into the backbuffer like any other overlay element.

!!! warning "Linux x86_64 / aarch64 only"
    Browser overlays are available on **Linux x86_64 and aarch64** only. On macOS and 32-bit targets the feature is a silent no-op (the helper isn't built there).

## Requirements

The helper links against WebKitGTK and GTK 3. Install the development packages when building from source:

| Distro | Packages |
|--------|----------|
| Debian/Ubuntu | `libwebkit2gtk-4.1-dev`, `libgtk-3-dev` |
| NixOS | `webkitgtk_4_1`, `gtk3` |

Official GitHub release builds already embed the helper, so downloaded binaries need no extra setup.

## Configuration

Browser overlays live under `overlays.browserOverlays` (a list), and are shown in a mode by listing their `name` in that mode's `browserOverlayIds`.

```lua
return {
  overlays = {
    browserOverlays = {
      {
        name = "chat",
        url = "https://oshhyy.github.io/JstChat2/#/",
        width = 800,
        height = 200,
        fps = 30,
        x = 20,
        y = 20,
        relativeTo = "topLeftScreen",
        transparentBackground = true,
      },
    },
  },
  modes = {
    {
      id = "Fullscreen",
      browserOverlayIds = { "timer" },
      -- ...
    },
  },
}
```

### Keys

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | string | `""` | Unique id, referenced from a mode's `browserOverlayIds`. |
| `url` | string | `""` | Page to load. |
| `customCss` | string | `""` | Extra CSS injected into the page (e.g. to hide elements or set a transparent body). |
| `width` | int | `800` | Render width of the off-screen page, in pixels. |
| `height` | int | `600` | Render height of the off-screen page, in pixels. |
| `fps` | int | `15` | How often the page is re-captured. Keep this low unless the page animates. |
| `x` / `y` | int | `0` | Position offset, **relative to the OS window origin** (top-left of the game window), not the game viewport. |
| `scale` | float | `1.0` | Scale factor applied when drawing the captured page. |
| `relativeTo` | string | `"topLeftScreen"` | Anchor the position is measured from. |
| `opacity` | float | `1.0` | `0.0`–`1.0` overlay opacity. |
| `cropTop` / `cropBottom` / `cropLeft` / `cropRight` | int | `0` | Pixels to crop off each edge of the captured page. |
| `pixelatedScaling` | bool | `false` | Use nearest-neighbour instead of linear filtering when scaling. |
| `transparentBackground` | bool | `false` | Render the page with a transparent background (combine with `customCss` setting `body { background: transparent }`). |
| `border` | table | - | Optional border (see the `border` block used by mirrors/images: `enabled`, `color`, `width`, `radius`). |

!!! note "x/y are window-relative"
    A browser overlay's `x`/`y` are measured from the **window** origin, so the overlay stays put when the game viewport is resized or re-centered (e.g. when switching to a Thin/Tall mode). This is intentional - it keeps web widgets anchored to the screen rather than drifting with the game view.

## How it works

`tuxinjector-browser` runs as a separate process hosting an off-screen WebKitGTK view. Tuxinjector's `BrowserCaptureManager` pulls frames from it and draws them into the scene as a `SceneElement::Textured` element each frame. Because it's a separate process, a misbehaving page can't take down the game.
