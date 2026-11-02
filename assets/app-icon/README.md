# OverFilm app icon

A clapperboard: an orange clapper stick above a white screen with a play button, on a dark grey
rounded tile. Designed by the OverFilm owner (Dominicus0830) on 2026-11-02; it replaces the
FilmCraft owl in this fork. The file names keep FilmCraft's (`filmcraft.*`,
`ai.storyteller.filmcraft.png`) so the code and packaging that load them did not change.

## Palette

| Colour | Hex | Used for |
|---|---|---|
| Tile | `#4A4B4D` | the full-bleed rounded square and the play triangle |
| Accent | `#F99E1F` | the clapper stick |
| Ink | `#F4F6FA` | the screen |

## Geometry

The design is a 100-unit drawing (tile `rx=22`; stick polygon `15,31 80,17 83,29 18,43`; screen
`x=16 y=49 68×36 rx=4`; play triangle `M42 56 L62 67 L42 78 Z`), scaled ×5.12 into the 512-unit
viewBox that `packaging/icons.sh` expects (`rx=112`). macOS renders put the tile in Apple's grid
(824 of 1024 px, transparent margin); Windows, Linux and the web use the full-bleed tile.

## Files

| File | What |
|---|---|
| `filmcraft.svg` | canonical artwork |
| `filmcraft-small.svg` | the same file (the drawing is already small) |
| `filmcraft-1024.png` | 1024 px render |
| `filmcraft-macos-512.png` | runtime Dock icon on macOS (embedded by `apps/filmcraft/src/main.rs`) |
| `filmcraft.icns` | macOS bundle icon |
| `filmcraft.ico` | Windows icon, 16–256 px: the exe's resource icon (`apps/filmcraft/build.rs`, and OverFilmSubs' `apps/overfilm-subs/build.rs`) and the installer's shortcuts |
| `hicolor/<size>/apps/ai.storyteller.filmcraft.png`, `hicolor/scalable/…svg` | Linux icon theme; the 256 px PNG is also the runtime window icon on Windows and Linux (editor and OverFilmSubs) |

The web app's `apps/filmcraft-web/web/favicon.png` (128 px) comes from the same artwork.

## How the icon reaches Windows

- **Exe icon and Task Manager name:** `apps/filmcraft/build.rs` embeds `filmcraft.ico` and a
  VERSIONINFO whose `FileDescription`/`ProductName` is "OverFilm". On a Windows host it uses
  `winresource`; cross-compiling to `x86_64-pc-windows-gnu` from Linux it runs MinGW's
  `x86_64-w64-mingw32-windres` (`apps/filmcraft/winres.rs`).
- **Window, taskbar, Alt-Tab:** the runtime icon (`ViewportBuilder::with_icon`) and the window
  title "OverFilm".
- **Installer, Start menu, desktop shortcut:** the NSIS installer uses `filmcraft.ico` (copied as
  `OverFilm.ico`).

## Regenerate

`packaging/icons.sh` (needs `resvg`; `iconutil` on macOS for the `.icns`; the `.ico` is packed by
`cargo xtask ico`). Every derived file needs its `.attribution` sidecar and a row in
`ATTRIBUTION.md`.
