# mmv application icon

`mmv.png` is the transparent version of the approved M²V image.
The charcoal navy rounded tile and white lettering are retained; the exterior
background is transparent. `mmv.ico` contains 16, 24, 32, 48, 64, 128 and 256 px
RGBA images for Windows display sizes.

The Windows build embeds `mmv.rc` into the `mmv` binary. Resource ID 1 is used
because the pinned `gpui-pre-windows 0.3.11` backend loads that icon from the
executable and assigns it to its window class. The same embedded icon is
available to Windows Explorer. No external image file is needed at runtime.

Build on Windows with the MSVC Rust toolchain and Windows SDK installed:

```powershell
cargo build --locked --release
.\target\release\mmv.exe open .\docs\requirements\index.md
```

Check the executable in Explorer, the taskbar/Alt+Tab icon, and the window title
(`mmv`). Check light and dark backgrounds and Windows display scaling: the
rounded corners should have no white rectangle. Windows can cache executable
icons; if a previous icon persists, check a freshly copied executable at a new
path and re-pin existing taskbar shortcuts.

To regenerate the ICO after deliberately replacing the PNG (Pillow required):

```python
from PIL import Image

Image.open("assets/mmv.png").save(
    "assets/mmv.ico",
    format="ICO",
    sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)],
)
```
