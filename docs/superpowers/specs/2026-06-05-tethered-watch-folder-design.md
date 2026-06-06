# Tethered Watch-Folder ("Shoot & See") Design

**Date:** 2026-06-05
**Branch:** `main`
**Status:** Approved, ready for implementation planning

## Goal

Let a user on a copy stand **shoot a negative and see the finished positive on
the spot**, with no SD-card shuffling and no manual import. OpenEnlarge watches a
folder; the instant a new scan lands, it imports, develops it with the active
roll's base + settings, and shows the positive.

We deliberately do **not** drive the camera (no shutter trigger, no USB/PTP, no
vendor SDK). The user points their camera's own free tether app
(Sony Imaging Edge, Canon EOS Utility, Nikon NX Tether, Fujifilm X Acquire,
Capture One / Lightroom tether, …) at an output folder; OpenEnlarge stands
downstream of that folder. This is what makes the feature work with **every
camera on every OS** at zero licensing/driver risk.

## Why watch-folder (feasibility verdict)

In-app tethering was explored and rejected for v1. The honest landscape:

- **libgphoto2** (generic PTP, ~2500 cameras) is clean on macOS/Linux but **not
  officially supported on Windows** — it needs fragile WinUSB driver binding that
  fights vendor drivers. Fails the "all 3 OSes" goal.
- **Vendor SDKs:** Sony's Camera Remote SDK is the only one whose license is
  friendly to an open-source MIT app; **Canon EDSDK and Nikon SDK require
  developer-program approval and have redistribution terms incompatible with
  shipping in an open-source repo** — a hard blocker, not just effort. Fujifilm
  has no broadly-available public tether SDK.

So "any camera, in-app, on Windows, open-source" does not exist as one coherent
thing. The watch-folder gives the full "shoot & see" loop with none of that
risk, and — critically — **any future in-app tether is strictly additive**: it
would only need to drop files into the same watched folder, leaving everything
downstream untouched. The watch-folder is the foundation, not a stopgap.

## Resolved decisions

- **Watched folder = the active roll.** Starting a tether session picks/confirms
  a destination folder (defaulting to the current roll); that folder becomes the
  active roll. New shots inherit `folderBaseByPath[dir]`, so the user calibrates
  the orange base **once** on the first frame and every later shot develops with
  it automatically. Reuses the per-roll base model as-is; no new base concept.
- **The watcher lives in Rust** using the `notify` crate (FSEvents / inotify /
  ReadDirectoryChangesW), emitting a Tauri event per new file. No frontend
  polling.
- **One destination, one session.** A single active watched folder at a time.
  Multi-folder watching is out of scope.
- **The vendor tether app is the file producer.** OpenEnlarge does not trigger
  the shutter. Documented as a setup step, not a code path.
- **Reuse the existing pipeline.** A new file runs the *same* `import_image` →
  `develop_image` path the manual importer uses; tether adds no new develop code.

## Current state (relevant facts)

- Import is two steps: `import_image(path)` (light — thumbnail + metadata +
  catalog upsert, `commands.rs:283`) then `develop_image(id)` (heavy — decode +
  invert, samples/applies the base, `commands.rs:311`). The frontend wrappers are
  `api.importImage` / `api.developImage` (`api.ts:145,163`).
- The manual import flow (`panels/Source.svelte`) calls `importImage`, upserts
  into the `images` store (dedupe by id), then `developAll()` (`workflow.ts`)
  develops undeveloped images in the selected folder and switches to Develop.
- A **roll is a folder.** `folderBaseByPath` (keyed by image dir, persisted as
  `app_state` `folder_base:{dir}`) holds the per-roll base; per-image
  `base_override` wins over it (`store.ts:57`, `develop/base.ts`,
  per-roll-base-calibration spec). `selectedFolder` / `folderImages` track the
  active roll (`store.ts`, `library/folderScope.ts`).
- Accepted scan extensions (from the import dialog filter, `Source.svelte:13`):
  `jpg jpeg png dng tif tiff raf rw2 nef arw cr3 3fr raw`.
- Tauri plugins already in use: dialog, opener, updater, process. No FS-watch
  plugin yet; `notify` is a new direct dependency in `app/src-tauri`.

## Architecture

Three units, each independently testable:

### 1. Backend watcher — `app/src-tauri/src/tether.rs` (new)

- State: an `Option<active watcher>` behind a `Mutex` in `Session` (or a
  dedicated `TetherState`), holding the `notify` watcher handle + watched dir so
  starting a new session cleanly drops the old one.
- Commands (registered in `lib.rs`):
  - `tether_start(dir: String)` — validate dir exists, start a `notify`
    recommended watcher on it, return Ok. Replaces any existing session.
  - `tether_stop()` — drop the watcher.
- On a create/rename event for a path whose extension is in the accepted set
  (case-insensitive), run the **stability gate** (below), then emit Tauri event
  `tether://new-file` with `{ path }`.
- **Stability gate** (the one real engineering wrinkle): tether apps write large
  RAWs incrementally or write-then-rename. Before emitting, poll the file size
  every ~250 ms until it is unchanged across two reads (and the file is
  openable). Cap with a timeout; on timeout, skip and log. This is the unit most
  worth testing in isolation.
- **Filters:** ignore hidden/temp files (leading `.`, known temp suffixes),
  sidecars (`.xmp`), and de-dupe rapid duplicate events for the same path.

### 2. Frontend controller — `app/src/lib/tether/` (new)

- `controller.ts`: `listen("tether://new-file", …)` →
  1. `api.importImage(path)`, upsert into `images` (same dedupe as Source).
  2. `api.developImage(entry.id)` (inherits the folder base via the existing
     resolve path — no extra wiring).
  3. Update the `images` store with the developed entry; if "auto-advance" is on,
     `activeId.set(entry.id)` and `module.set("develop")` so the latest positive
     is front-and-center.
  4. On failure, record it in tether status; **keep the session running**.
- A small store: `{ watching: boolean, dir: string|null, last: {name,status}|null,
  error: string|null }`.
- Pure helpers (testable without Tauri): extension check, the
  event→import→develop sequence with an injected `api` mock.

### 3. UI — tether panel

- A compact control (in the Library tab / Source panel area): **Start tethering**
  → folder picker (via existing `@tauri-apps/plugin-dialog` `open({directory:
  true})`, defaulting to the current roll) → calls `tether_start`. **Stop** calls
  `tether_stop`.
- Status line: *Watching <folder>… · last: <file> ✓ / ✗ <error>*.
- An "auto-advance to newest" toggle (default on).
- A one-line hint: *"Point your camera's tether app at this folder."*
- New i18n strings (EN/ZH) following the existing `i18n-strings.csv` + `dict.ts`
  workflow.

## Data flow

```
camera ──USB──▶ vendor tether app ──writes file──▶ watched folder
                                                        │
                                          notify event (create/rename)
                                                        │
                                            stability gate + ext/sidecar filter
                                                        │
                                          emit  tether://new-file { path }   (Rust)
                                                        │  (JS)
                                    import_image ─▶ develop_image (folder base applied)
                                                        │
                              update images store ─▶ (auto-advance) activeId + Develop view
                                                        ▼
                                            finished positive on screen
```

## Error handling

- **File still being written** → stability gate waits; never imports a partial
  file. Timeout → skip + status note.
- **Decode/develop failure** (corrupt/unsupported frame) → surface in tether
  status, keep watching. One bad frame never stops the session.
- **Folder deleted/unmounted mid-session** → notify error → set tether error
  state, mark not-watching, let the user restart.
- **Duplicate / temp / sidecar files** → filtered before emit.
- **App restart** → tether session does not auto-resume in v1 (explicit Start).

## Testing

- **Rust (`tether.rs`):** unit-test the stability gate and the
  extension/sidecar/hidden filters against a `tempfile` dir — synthesize a file
  written in chunks and assert it emits only once, after the size settles; assert
  `.xmp`/hidden/unknown-extension files are ignored. (notify itself is trusted;
  test our logic around it.)
- **Frontend (`tether/*.test.ts`, Vitest):** test the controller's
  event→import→develop sequence with a mocked `api` (matches the existing
  `workflow.test.ts` / `catalog.test.ts` pattern): asserts dedupe-upsert, that
  develop is called, store updates, auto-advance behavior, and that a thrown
  develop is caught and recorded without stopping.

## Out of scope (v1, YAGNI)

- Shutter triggering / remote capture, USB/PTP, libgphoto2, any vendor SDK.
- Live-view / inverted live preview.
- Watching multiple folders simultaneously; auto-resume on app launch.
- Auto-creating/auto-stacking bracketed frames.

## Future (strictly additive, separate specs)

- **Phase 2** — optional in-app trigger+download via libgphoto2 (macOS/Linux
  beta) writing into the watched folder.
- **Phase 3** — Sony Camera Remote SDK for in-app tether + live-view on Windows.

Both plug in *upstream* of `tether://new-file`; nothing in this design changes.
