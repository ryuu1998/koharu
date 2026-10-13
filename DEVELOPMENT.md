# Windows debug launcher

Double-click **Koharu Dev.bat** in the repository root.

- **1: Launch saved debug build** starts the current frontend with hot reload and
  opens the saved debug application. It does not invoke Cargo or rebuild Rust.
- **2: Compile changed code and launch** builds the WASM bridge in debug mode,
  regenerates the typed protocol, and incrementally compiles the native debug app.
  Use it for the first launch and after changing Rust code.

Install frontend dependencies once with
`bun install --frozen-lockfile --ignore-scripts`. Building needs Rust, its
`wasm32-unknown-unknown` target, the Windows C++ build tools, and Bun. The launcher
uses this checkout's `target` directory, or your existing `CARGO_TARGET_DIR`.

While the app is open, frontend edits reload automatically. Rust canvas edits
rebuild the debug WASM bridge through the existing watcher. Closing the app also
stops the frontend processes started by this launcher. An occupied port 3000 is
reported instead of stopping another development session.

Frontend logs are in `.local-git/dev`. Native logs remain in the launch terminal.
The debug CEF inspection endpoint is `http://127.0.0.1:4000`.

The first launch of a new upstream version can show **Preparing Koharu** while
its native AI runtime packages are downloaded and installed into the user cache.
This is separate from compilation; later launches reuse those installed packages.

You can also run `"Koharu Dev.bat" 1` or `"Koharu Dev.bat" 2` from a command prompt.

The debug profile optimizes CPU image preparation, model postprocessing, layout,
and export code. Debug symbols, assertions, hot reload, and the inspection endpoint
remain enabled. Compiling these packages takes longer the first time; option 1
continues to reuse the saved executable.

## Comparing batch timings

Use the same source chapter and saved model settings, with a new empty output
folder for each comparison. Existing translated outputs have a green check and
are skipped by default. Use the export-folder icon to choose another destination.

The launch terminal prints import, pipeline, export, and total seconds for each
successful chapter. A first run can include model loading; a second run in the
same app session reuses resident models where memory permits. Record both when
comparing the previous build with this one.

Cloud translation allows up to six page requests in flight while local GPU model
work stays serialized. The batch pipeline retains decoded images across stages,
prepares OCR checkpoint files alongside detection, and prepares the export renderer
during processing. OCR file preparation does not preload another model onto the
GPU. These changes have focused checks; real chapter speedups still need measuring
on the target device.

## Headless chapter translation

The same debug executable can translate a CBZ file or a folder of chapters:

```powershell
.\target\debug\koharu.exe batch "C:\Manga" --output "C:\Translated"
```

It uses saved Koharu settings, skips existing outputs, and preserves source files.
Use `--cpu` for CPU model inference, `--jpeg-quality 95` to choose JPEG quality,
and `--overwrite` only when replacing existing translated outputs is intentional.
Ctrl+C requests a cooperative stop. The command exits with a failure status when
chapters fail, and with status 130 when stopped.
