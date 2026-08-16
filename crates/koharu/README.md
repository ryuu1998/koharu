# koharu

`koharu` is Koharu's native composition package. It owns process startup,
diagnostics, Tauri build integration, and application configuration.

## Application boundary

```text
React
  | direct Tauri commands
  | typed IPC channels
  v
koharu-app
  | Tauri-managed project, canvas, pipeline, jobs, and channel state
  +-> koharu-scene
  +-> koharu-desktop -> koharu-canvas
  +-> koharu-renderer -> raster / koharu-psd

koharu -> koharu-app + koharu-desktop
```

## Batch translation

The **Batch** destination in the desktop title bar provides the primary batch
workflow. Choose a folder of CBZ chapters, optionally choose a different export
folder, select any number of chapter cards, and start processing. The workspace
shows page-level progress for the active chapter and chapter-level progress for
the entire selection. It uses the pipeline, provider, target-language, and
typesetting configuration already saved in Koharu.

The native executable can translate one CBZ archive or every top-level CBZ in
a directory without opening the desktop window. Batch mode reads the same
pipeline, provider, target-language, and typesetting configuration saved by the
desktop application. Close the desktop application before a large run so both
processes do not compete for model memory.

```powershell
koharu batch --input "Manga Project"
```

Outputs are written to `Manga Project/Translated` by default. Each archive keeps
the source chapter filename. Rendered pages are composited onto white and
encoded as quality-95 JPEG before being stored in the CBZ, avoiding the much
larger lossless-PNG archives. Existing outputs are skipped unless `--overwrite`
is supplied; `--output`, `--jpeg-quality`, and `--cpu` provide explicit
overrides.

Every operation has a named Tauri command. Commands that mutate a project take
its id and current revision directly. The frontend serializes those mutations
and uses the returned revision for the next call.

Native updates do not share an event envelope. `connect` binds independent
typed channels for project snapshots, canvas state, jobs, downloads,
preferences, resource telemetry, and cleanup reports. Tauri state is the only
application state container.

Thumbnails are read with `get_thumbnail`; the frontend creates a temporary
object URL from the returned bytes. There is no custom URI scheme or resource
protocol.

## Generated bindings

Rust command signatures and data types are authoritative:

```powershell
cargo run -p koharu-app --bin generate
```

Focused validation:

```powershell
cargo check -p koharu -p koharu-app -p koharu-desktop
bun x tsc --noEmit -p packages/koharu/tsconfig.json
cd packages/koharu
bun run test
```
