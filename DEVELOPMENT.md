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

You can also run `"Koharu Dev.bat" 1` or `"Koharu Dev.bat" 2` from a command prompt.

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
