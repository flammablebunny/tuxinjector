# Companion Apps

Companion apps are small helper programs - most commonly **Ninjabrain Bot** (NBB) - that Tuxinjector launches, captures, and feeds input to so they appear as an overlay inside the game and respond to your in-game hotkeys.

## How it works (Linux)

When you launch an **anchored** companion app, Tuxinjector starts a private, headless **Xvfb** (X virtual framebuffer) server that only it owns, and runs the app inside it. Tuxinjector then:

- **Captures** the app's window from the Xvfb and composites it into the overlay.
- **Injects keys** into the app via **XTEST**, so your configured hotkeys reach it even though it has no real window focus.
- **Bridges F3+C**: when you copy coordinates in-game (F3+C), Tuxinjector hooks `glfwSetClipboardString` and takes ownership of the X `CLIPBOARD`/`PRIMARY` selections inside the Xvfb, so NBB reads the coordinates with no extra setup.

### Xvfb prerequisite

Because companion apps run in a private Xvfb, an `Xvfb` binary must be available:

- **Most distros:** install the X virtual framebuffer package (e.g. `xorg-server-xvfb`, `xvfb`).
- **NixOS:** nothing to do - the flake pulls `Xvfb` in for you, and `tuxinjector-wrapper` points `TUXINJECTOR_XVFB` at it automatically.

On any other setup where `Xvfb` isn't on the game process's `PATH`, set the `TUXINJECTOR_XVFB` environment variable to the absolute path of the `Xvfb` binary in your launch command.

### Key injection on X11 and Wayland

XTEST injects **X11 keycodes**, while GLFW reports scancodes that differ between backends (the Wayland backend reports evdev scancodes - X keycode = evdev + 8 - while the X11/Xorg backend already reports X keycodes). Tuxinjector auto-detects the right offset at runtime (Escape's X keycode is always 9, so the offset is `9 - glfwGetKeyScancode(GLFW_KEY_ESCAPE)`: +8 on Wayland/XWayland, 0 on plain X11). So hotkey injection works on **both** session types with no configuration.

## Launch Detached

The **Launch Detached** option starts the app on your **host display** instead of inside Tuxinjector's private Xvfb. Detached apps are *not* captured into the overlay - this mode exists so you can open the app's real window to **rebind its hotkeys in your normal X11 namespace** (the keycodes there match what gets injected into the Xvfb). Use it once to set hotkeys up, then launch anchored for normal play.

## Lifecycle

Companion apps are terminated (SIGTERM) when the game exits, so they don't linger as zombie processes.

## macOS

On macOS there is no Xvfb. Companion apps are captured with ScreenCaptureKit (falling back to CoreGraphics), which requires **Screen Recording** permission for the Java binary Minecraft runs on - grant it under *System Settings -> Privacy & Security -> Screen Recording*. See [Installation](installation.md#macos).
