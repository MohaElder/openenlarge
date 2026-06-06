# Tethered Watch-Folder ("Shoot & See") Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Watch a user-chosen folder; the instant a camera's tether app drops a new scan there, auto-import and develop it with the active roll's base and show the finished positive.

**Architecture:** A Rust `notify` watcher (in a new `tether.rs`) filters new files, waits until each is fully written, and emits a `tether://new-file` Tauri event. A frontend controller listens, runs the existing `import_image → develop_image` pipeline, and (optionally) jumps the viewport to the newest frame. The watched folder *is* the active roll, so the per-roll base (`folderBaseByPath`) applies automatically — no new develop code.

**Tech Stack:** Rust + Tauri 2 (`notify` crate, `tempfile` for tests), SvelteKit + TypeScript, Vitest, CSV-driven i18n (`scripts/gen-i18n.py`).

**Spec:** `docs/superpowers/specs/2026-06-05-tethered-watch-folder-design.md`

---

## File Structure

**Backend (`app/src-tauri/`):**
- Create `src/tether.rs` — accepted-scan filter, stability gate, `TetherState`, `tether_start`/`tether_stop` commands, notify wiring, event emit.
- Modify `Cargo.toml` — add `notify` dep, `tempfile` dev-dep.
- Modify `src/lib.rs` — `mod tether;`, `.manage(TetherState)`, register the two commands.

**Frontend (`app/src/lib/tether/`):**
- Create `store.ts` — tether UI stores.
- Create `controller.ts` — `processNewFile`, `startTether`, `stopTether`, event listener.
- Create `controller.test.ts` — Vitest for `processNewFile`.
- Create `TetherPanel.svelte` — Start/Stop + folder pick + status.
- Modify `src/lib/api.ts` — `tetherStart` / `tetherStop` wrappers.
- Modify `src/lib/panels/Source.svelte` — mount `TetherPanel`.

**i18n:**
- Modify `i18n-strings.csv` — new `tether.*` rows; regenerate `app/src/lib/i18n/dict.ts`.

---

## Task 1: Accepted-scan filter (backend, pure)

**Files:**
- Create: `app/src-tauri/src/tether.rs`
- Modify: `app/src-tauri/src/lib.rs` (add `mod tether;`)

- [ ] **Step 1: Add the module declaration**

In `app/src-tauri/src/lib.rs`, add `mod tether;` to the module list at the top (alphabetical, after `mod session;`):

```rust
mod metadata;
mod session;
mod tether;
```

- [ ] **Step 2: Write the failing test**

Create `app/src-tauri/src/tether.rs` with only the test module and a stub:

```rust
//! Tethered watch-folder: watch a directory, emit an event per fully-written scan.

/// File extensions we treat as scans, lowercase, no dot. Mirrors the import
/// dialog filter in `panels/Source.svelte`.
const SCAN_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "dng", "tif", "tiff", "raf", "rw2", "nef", "arw", "cr3", "3fr", "raw",
];

/// True if `file_name` is a scan we should auto-develop: a known image extension,
/// not a hidden dotfile, not an editor/OS temp, not an XMP sidecar.
pub fn is_accepted_scan(file_name: &str) -> bool {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_known_raw_and_image_extensions() {
        assert!(is_accepted_scan("DSCF1234.RAF"));
        assert!(is_accepted_scan("IMG_0001.dng"));
        assert!(is_accepted_scan("scan.tiff"));
        assert!(is_accepted_scan("frame.JPG"));
    }

    #[test]
    fn rejects_unknown_extensions_and_no_extension() {
        assert!(!is_accepted_scan("notes.txt"));
        assert!(!is_accepted_scan("Makefile"));
        assert!(!is_accepted_scan("movie.mov"));
    }

    #[test]
    fn rejects_sidecars_hidden_and_temp_files() {
        assert!(!is_accepted_scan("DSCF1234.xmp"));
        assert!(!is_accepted_scan(".DS_Store"));
        assert!(!is_accepted_scan(".hidden.dng"));
        assert!(!is_accepted_scan("DSCF1234.dng.tmp"));
        assert!(!is_accepted_scan("~temp.dng"));
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --manifest-path app/src-tauri/Cargo.toml tether::tests -- --nocapture`
Expected: compiles, tests panic with `not yet implemented` (todo!).

- [ ] **Step 4: Implement `is_accepted_scan`**

Replace the `todo!()` body:

```rust
pub fn is_accepted_scan(file_name: &str) -> bool {
    // Hidden dotfiles and tilde temp files are never scans.
    if file_name.starts_with('.') || file_name.starts_with('~') {
        return false;
    }
    let lower = file_name.to_ascii_lowercase();
    // Reject common in-progress/temp suffixes that wrap a real name.
    if lower.ends_with(".tmp") || lower.ends_with(".part") || lower.ends_with(".xmp") {
        return false;
    }
    match lower.rsplit_once('.') {
        Some((_, ext)) => SCAN_EXTS.contains(&ext),
        None => false,
    }
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test --manifest-path app/src-tauri/Cargo.toml tether::tests -- --nocapture`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add app/src-tauri/src/tether.rs app/src-tauri/src/lib.rs
git commit -m "feat(tether): accepted-scan filename filter"
```

---

## Task 2: Stability gate (backend)

Wait until a file has stopped growing before we develop it — tether apps write large RAWs incrementally or write-then-rename.

**Files:**
- Modify: `app/src-tauri/src/tether.rs`
- Modify: `app/src-tauri/Cargo.toml` (add `tempfile` dev-dependency)

- [ ] **Step 1: Add the `tempfile` dev-dependency**

In `app/src-tauri/Cargo.toml`, add a dev-dependencies section (after the `[dependencies]` block, before the `[profile.dev]` block):

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Write the failing test**

Add to the `tests` module in `app/src-tauri/src/tether.rs`:

```rust
    use std::io::Write;
    use std::time::Duration;

    #[test]
    fn stable_returns_true_for_a_complete_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("done.dng");
        std::fs::write(&p, b"already fully written").unwrap();
        // Short cadence so the test is fast; file is already stable.
        assert!(wait_until_stable(&p, Duration::from_millis(10), Duration::from_secs(2)));
    }

    #[test]
    fn stable_returns_false_for_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nope.dng");
        assert!(!wait_until_stable(&p, Duration::from_millis(10), Duration::from_millis(80)));
    }

    #[test]
    fn stable_waits_out_a_growing_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("growing.dng");
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(b"chunk1").unwrap();
        f.flush().unwrap();
        let p2 = p.clone();
        // Append once more after a beat, then stop growing.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            let mut g = std::fs::OpenOptions::new().append(true).open(&p2).unwrap();
            g.write_all(b"chunk2").unwrap();
            g.flush().unwrap();
        });
        assert!(wait_until_stable(&p, Duration::from_millis(20), Duration::from_secs(2)));
        // Final size reflects both chunks (gate didn't fire mid-write).
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 12);
    }
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test --manifest-path app/src-tauri/Cargo.toml tether::tests -- --nocapture`
Expected: compile error — `wait_until_stable` not found.

- [ ] **Step 4: Implement `wait_until_stable`**

Add to `tether.rs` (above the `#[cfg(test)]` module):

```rust
use std::path::Path;
use std::time::{Duration, Instant};

/// Block until `path`'s size is unchanged across two consecutive reads spaced by
/// `poll`, or until `max_wait` elapses. Returns true once stable, false on
/// timeout or if the file never becomes readable. Cheap: just stats the file.
pub fn wait_until_stable(path: &Path, poll: Duration, max_wait: Duration) -> bool {
    let deadline = Instant::now() + max_wait;
    let mut last: Option<u64> = None;
    loop {
        let size = std::fs::metadata(path).map(|m| m.len()).ok();
        match (last, size) {
            (Some(prev), Some(cur)) if prev == cur && cur > 0 => return true,
            _ => {}
        }
        last = size;
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(poll);
    }
}
```

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test --manifest-path app/src-tauri/Cargo.toml tether::tests -- --nocapture`
Expected: PASS (6 tests total).

- [ ] **Step 6: Commit**

```bash
git add app/src-tauri/src/tether.rs app/src-tauri/Cargo.toml
git commit -m "feat(tether): file stability gate"
```

---

## Task 3: TetherState + start/stop commands + notify wiring

Wire the watcher: a managed `TetherState` holds the live watcher; `tether_start` begins watching and `tether_stop` drops it. On each accepted, stabilized file, emit `tether://new-file`.

**Files:**
- Modify: `app/src-tauri/Cargo.toml` (add `notify` dependency)
- Modify: `app/src-tauri/src/tether.rs`
- Modify: `app/src-tauri/src/lib.rs` (manage state + register commands)

- [ ] **Step 1: Add the `notify` dependency**

In `app/src-tauri/Cargo.toml` `[dependencies]`, add:

```toml
notify = "6"
```

- [ ] **Step 2: Write the failing test (state replacement)**

Add to the `tests` module in `tether.rs`:

```rust
    #[test]
    fn tether_state_holds_and_replaces_the_watcher_slot() {
        let state = TetherState::default();
        assert!(state.0.lock().unwrap().is_none());
        // Simulate stop clearing the slot.
        *state.0.lock().unwrap() = None;
        assert!(state.0.lock().unwrap().is_none());
    }
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test --manifest-path app/src-tauri/Cargo.toml tether::tests -- --nocapture`
Expected: compile error — `TetherState` not found.

- [ ] **Step 4: Implement state, payload, and commands**

Add to `tether.rs` (top-level, near the other items):

```rust
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

/// Managed state: the single live watcher, or None when not tethering. Dropping
/// the watcher stops it, so replacing the slot cleanly ends the prior session.
#[derive(Default)]
pub struct TetherState(pub Mutex<Option<RecommendedWatcher>>);

/// Event payload sent to the frontend when a new scan is ready to develop.
#[derive(Clone, Serialize)]
struct NewFile {
    path: String,
}

/// Start watching `dir`. Replaces any existing session.
#[tauri::command]
pub fn tether_start(
    dir: String,
    app: AppHandle,
    state: tauri::State<TetherState>,
) -> Result<(), String> {
    let watch_dir = std::path::PathBuf::from(&dir);
    if !watch_dir.is_dir() {
        return Err(format!("not a folder: {dir}"));
    }
    let app_for_events = app.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        // Only react to file creation / rename-into-place.
        let relevant = matches!(
            event.kind,
            notify::EventKind::Create(_) | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
        );
        if !relevant {
            return;
        }
        for path in event.paths {
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !is_accepted_scan(name) {
                continue;
            }
            // Stabilize + emit off the watcher thread so we never block it.
            let app = app_for_events.clone();
            std::thread::spawn(move || {
                if wait_until_stable(&path, Duration::from_millis(250), Duration::from_secs(30)) {
                    let payload = NewFile { path: path.to_string_lossy().to_string() };
                    if let Err(e) = app.emit("tether://new-file", payload) {
                        eprintln!("[tether] emit failed: {e}");
                    }
                } else {
                    eprintln!("[tether] file never stabilized: {}", path.display());
                }
            });
        }
    })
    .map_err(|e| e.to_string())?;
    watcher
        .watch(&watch_dir, RecursiveMode::NonRecursive)
        .map_err(|e| e.to_string())?;
    *state.0.lock().unwrap() = Some(watcher);
    Ok(())
}

/// Stop watching (drops the watcher).
#[tauri::command]
pub fn tether_stop(state: tauri::State<TetherState>) -> Result<(), String> {
    *state.0.lock().unwrap() = None;
    Ok(())
}
```

- [ ] **Step 5: Register state + commands in `lib.rs`**

In `app/src-tauri/src/lib.rs`, add the managed state next to the existing `.manage(...)` (after line 22):

```rust
        .manage(session::Session::default())
        .manage(tether::TetherState::default())
```

And add the two commands to the `generate_handler!` list (after `commands::sample_base_at,`):

```rust
            commands::sample_base_at,
            tether::tether_start,
            tether::tether_stop,
        ])
```

- [ ] **Step 6: Run tests + build to verify**

Run: `cargo test --manifest-path app/src-tauri/Cargo.toml tether::tests -- --nocapture`
Expected: PASS (7 tests).
Run: `cargo build --manifest-path app/src-tauri/Cargo.toml`
Expected: builds clean (warnings ok).

- [ ] **Step 7: Commit**

```bash
git add app/src-tauri/src/tether.rs app/src-tauri/src/lib.rs app/src-tauri/Cargo.toml app/src-tauri/Cargo.lock
git commit -m "feat(tether): notify watcher + start/stop commands"
```

---

## Task 4: Frontend tether stores

**Files:**
- Create: `app/src/lib/tether/store.ts`

- [ ] **Step 1: Create the stores**

Create `app/src/lib/tether/store.ts`:

```ts
import { writable } from "svelte/store";

/** True while a watch-folder session is active. */
export const tetherWatching = writable<boolean>(false);

/** The folder currently being watched (also the active roll), or null. */
export const tetherDir = writable<string | null>(null);

/** When true, each new shot becomes active and switches to Develop. */
export const tetherAutoAdvance = writable<boolean>(true);

/** Status of the most recent capture, for the panel's status line. */
export const tetherLast = writable<{ name: string; ok: boolean; error?: string } | null>(null);
```

- [ ] **Step 2: Typecheck**

Run: `npm --prefix app run check`
Expected: no new errors in `tether/store.ts`.

- [ ] **Step 3: Commit**

```bash
git add app/src/lib/tether/store.ts
git commit -m "feat(tether): frontend stores"
```

---

## Task 5: Frontend controller (`processNewFile`) — TDD

**Files:**
- Create: `app/src/lib/tether/controller.ts`
- Create: `app/src/lib/tether/controller.test.ts`
- Modify: `app/src/lib/api.ts` (add `tetherStart` / `tetherStop`)

- [ ] **Step 1: Add the api wrappers**

In `app/src/lib/api.ts`, inside the `api` object (near `importImage` / `developImage`, around line 145–163), add:

```ts
  tetherStart: (dir: string) => invoke<void>("tether_start", { dir }),
  tetherStop: () => invoke<void>("tether_stop"),
```

- [ ] **Step 2: Write the failing test**

Create `app/src/lib/tether/controller.test.ts`:

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";
import { get } from "svelte/store";

vi.mock("../api", async (orig) => {
  const actual = await orig<typeof import("../api")>();
  return {
    ...actual,
    api: {
      ...actual.api,
      importImage: vi.fn(async (path: string) => ({
        id: path, path, file_name: path.split("/").pop()!, thumbnail: "t",
        metadata: { width: 0, height: 0, file_size: 0 }, developed: false, has_ir: false, offline: false,
      })),
      developImage: vi.fn(async (id: string) => ({
        id, path: id, file_name: id.split("/").pop()!, thumbnail: "dev",
        metadata: { width: 10, height: 10, file_size: 0 }, developed: true, has_ir: false, offline: false,
      })),
    },
  };
});

describe("processNewFile", () => {
  beforeEach(async () => {
    const { images, activeId, module } = await import("../store");
    const { tetherAutoAdvance, tetherLast } = await import("./store");
    images.set([]); activeId.set(null); module.set("library");
    tetherAutoAdvance.set(true); tetherLast.set(null);
  });

  it("imports, develops, adds the developed entry, and auto-advances", async () => {
    const { processNewFile } = await import("./controller");
    const { images, activeId, module } = await import("../store");
    await processNewFile("/roll/DSCF1.dng");
    const list = get(images);
    expect(list).toHaveLength(1);
    expect(list[0].developed).toBe(true);
    expect(get(activeId)).toBe("/roll/DSCF1.dng");
    expect(get(module)).toBe("develop");
    expect(get((await import("./store")).tetherLast)).toEqual({ name: "DSCF1.dng", ok: true });
  });

  it("does not change active/module when auto-advance is off", async () => {
    const { processNewFile } = await import("./controller");
    const { activeId, module } = await import("../store");
    const { tetherAutoAdvance } = await import("./store");
    tetherAutoAdvance.set(false);
    await processNewFile("/roll/DSCF2.dng");
    expect(get(activeId)).toBeNull();
    expect(get(module)).toBe("library");
  });

  it("records an error and does not throw when develop fails", async () => {
    const { api } = await import("../api");
    (api.developImage as ReturnType<typeof vi.fn>).mockRejectedValueOnce(new Error("decode boom"));
    const { processNewFile } = await import("./controller");
    const { tetherLast } = await import("./store");
    await expect(processNewFile("/roll/BAD.dng")).resolves.toBeUndefined();
    expect(get(tetherLast)).toEqual({ name: "BAD.dng", ok: false, error: "Error: decode boom" });
  });
});
```

- [ ] **Step 3: Run to verify it fails**

Run: `npm --prefix app run test:unit -- src/lib/tether/controller.test.ts`
Expected: FAIL — cannot resolve `./controller`.

- [ ] **Step 4: Implement the controller**

Create `app/src/lib/tether/controller.ts`:

```ts
import { get } from "svelte/store";
import { api } from "../api";
import { images, activeId, module } from "../store";
import { tetherAutoAdvance, tetherLast } from "./store";

/** Import → develop one freshly-captured file, then (optionally) bring it to the
 * front. Never throws: a bad frame is recorded in `tetherLast` and the session
 * keeps watching. The develop step inherits the folder's base via the existing
 * resolve path, so no base wiring is needed here. */
export async function processNewFile(path: string): Promise<void> {
  const name = path.split(/[\\/]/).pop() ?? path;
  try {
    const entry = await api.importImage(path);
    const developed = await api.developImage(entry.id);
    images.update((xs) =>
      xs.some((i) => i.id === developed.id)
        ? xs.map((i) => (i.id === developed.id ? developed : i))
        : [...xs, developed],
    );
    if (get(tetherAutoAdvance)) {
      activeId.set(developed.id);
      module.set("develop");
    }
    tetherLast.set({ name, ok: true });
  } catch (e) {
    tetherLast.set({ name, ok: false, error: String(e) });
  }
}
```

- [ ] **Step 5: Run to verify it passes**

Run: `npm --prefix app run test:unit -- src/lib/tether/controller.test.ts`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add app/src/lib/tether/controller.ts app/src/lib/tether/controller.test.ts app/src/lib/api.ts
git commit -m "feat(tether): controller import+develop pipeline"
```

---

## Task 6: Start/stop wiring + event listener

Add `startTether` / `stopTether` to the controller: they call the backend, bind the watched folder as the active roll, and register/unregister the `tether://new-file` listener.

**Files:**
- Modify: `app/src/lib/tether/controller.ts`

- [ ] **Step 1: Add start/stop with listener management**

Append to `app/src/lib/tether/controller.ts`:

```ts
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { selectedFolder } from "../store";
import { tetherWatching, tetherDir } from "./store";

let unlisten: UnlistenFn | null = null;

/** Begin a tether session on `dir`. The watched folder becomes the active roll. */
export async function startTether(dir: string): Promise<void> {
  await api.tetherStart(dir);
  if (!unlisten) {
    unlisten = await listen<{ path: string }>("tether://new-file", (e) => {
      void processNewFile(e.payload.path);
    });
  }
  selectedFolder.set(dir);
  tetherDir.set(dir);
  tetherWatching.set(true);
}

/** End the tether session. */
export async function stopTether(): Promise<void> {
  await api.tetherStop();
  if (unlisten) {
    unlisten();
    unlisten = null;
  }
  tetherWatching.set(false);
}
```

Note: the `import { get } ...` and `import { api } ...` lines from Task 5 already exist at the top of the file; add only the three new `import` lines shown above (keep all imports grouped at the top).

- [ ] **Step 2: Typecheck**

Run: `npm --prefix app run check`
Expected: no new errors in `tether/controller.ts`.

- [ ] **Step 3: Re-run controller tests (regression)**

Run: `npm --prefix app run test:unit -- src/lib/tether/controller.test.ts`
Expected: PASS (still 3) — the new imports don't affect `processNewFile`.

- [ ] **Step 4: Commit**

```bash
git add app/src/lib/tether/controller.ts
git commit -m "feat(tether): start/stop session + event listener"
```

---

## Task 7: i18n strings

**Files:**
- Modify: `i18n-strings.csv`
- Regenerate: `app/src/lib/i18n/dict.ts` (via `scripts/gen-i18n.py`)

- [ ] **Step 1: Append rows to the CSV**

Add these lines to the end of `i18n-strings.csv` (keep the exact quoting style of existing rows):

```csv
tether.start,"Start tethering","开始联机拍摄","src/lib/tether/TetherPanel.svelte","button"
tether.stop,"Stop tethering","停止联机拍摄","src/lib/tether/TetherPanel.svelte","button"
tether.pickFolder,"Choose folder…","选择文件夹…","src/lib/tether/TetherPanel.svelte","button"
tether.watching,"Watching {dir}","正在监视 {dir}","src/lib/tether/TetherPanel.svelte","status"
tether.hint,"Point your camera's tether app at this folder.","将相机的联机拍摄软件指向此文件夹。","src/lib/tether/TetherPanel.svelte","hint"
tether.autoAdvance,"Jump to newest shot","跳到最新拍摄","src/lib/tether/TetherPanel.svelte","label"
tether.lastOk,"Last: {name}","最近：{name}","src/lib/tether/TetherPanel.svelte","status"
tether.lastErr,"Failed: {name}","失败：{name}","src/lib/tether/TetherPanel.svelte","status"
```

- [ ] **Step 2: Regenerate the dict**

Run: `python3 scripts/gen-i18n.py`
Expected: `app/src/lib/i18n/dict.ts` updated; `git diff --stat` shows it changed.

- [ ] **Step 3: Verify the keys landed**

Run: `grep -c "tether\." app/src/lib/i18n/dict.ts`
Expected: `16` (8 keys × en + zh).

- [ ] **Step 4: Commit**

```bash
git add i18n-strings.csv app/src/lib/i18n/dict.ts
git commit -m "i18n(tether): EN/ZH strings for tether panel"
```

---

## Task 8: TetherPanel UI + mount

**Files:**
- Create: `app/src/lib/tether/TetherPanel.svelte`
- Modify: `app/src/lib/panels/Source.svelte` (mount the panel)

- [ ] **Step 1: Create the panel**

Create `app/src/lib/tether/TetherPanel.svelte`:

```svelte
<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { selectedFolder } from "../store";
  import { startTether, stopTether } from "./controller";
  import { tetherWatching, tetherDir, tetherAutoAdvance, tetherLast } from "./store";

  let error = "";

  async function toggle() {
    error = "";
    try {
      if ($tetherWatching) {
        await stopTether();
        return;
      }
      const dir = await open({ directory: true, defaultPath: $selectedFolder ?? undefined });
      if (!dir || Array.isArray(dir)) return;
      await startTether(dir);
    } catch (e) {
      error = String(e);
    }
  }
</script>

<div class="tether">
  <button class="toggle" class:on={$tetherWatching} on:click={toggle}>
    {$tetherWatching ? $t("tether.stop") : $t("tether.start")}
  </button>

  {#if $tetherWatching}
    <div class="status">{$t("tether.watching", { dir: $tetherDir ?? "" })}</div>
    <label class="adv">
      <input type="checkbox" bind:checked={$tetherAutoAdvance} />
      {$t("tether.autoAdvance")}
    </label>
    <div class="hint">{$t("tether.hint")}</div>
    {#if $tetherLast}
      <div class="last" class:err={!$tetherLast.ok}>
        {$tetherLast.ok
          ? $t("tether.lastOk", { name: $tetherLast.name })
          : $t("tether.lastErr", { name: $tetherLast.name })}
      </div>
    {/if}
  {/if}
  {#if error}<div class="last err">{error}</div>{/if}
</div>

<style>
  .tether { display: flex; flex-direction: column; gap: 6px; margin-top: 10px; }
  .toggle { width: 100%; padding: 9px; border-radius: 10px; border: 0;
    background: rgba(255,255,255,0.08); color: var(--text); font-weight: 600; cursor: pointer; }
  .toggle.on { background: rgba(244,157,78,0.22); }
  .status { font-size: 12px; opacity: 0.8; }
  .adv { display: flex; align-items: center; gap: 6px; font-size: 12px; opacity: 0.9; }
  .hint { font-size: 11px; opacity: 0.6; }
  .last { font-size: 12px; opacity: 0.85; }
  .last.err { color: #ff8a8a; }
</style>
```

- [ ] **Step 2: Mount it in the Source panel**

In `app/src/lib/panels/Source.svelte`, add the import (after the existing imports in `<script>`):

```ts
  import TetherPanel from "../tether/TetherPanel.svelte";
```

Then mount it just before the closing `</div>` of `.wrap` (after the Develop-all button block, before `</div></GlassPanel>`):

```svelte
    <TetherPanel />
  </div>
</GlassPanel>
```

- [ ] **Step 3: Typecheck**

Run: `npm --prefix app run check`
Expected: no new errors.

- [ ] **Step 4: Full frontend test run (regression)**

Run: `npm --prefix app run test:unit`
Expected: all suites pass, including `tether/controller.test.ts`.

- [ ] **Step 5: Commit**

```bash
git add app/src/lib/tether/TetherPanel.svelte app/src/lib/panels/Source.svelte
git commit -m "feat(tether): TetherPanel UI + mount in Source"
```

---

## Task 9: End-to-end manual verification

No code; confirm the real loop works.

- [ ] **Step 1: Launch the app**

Run: `npm --prefix app run tauri dev`
Expected: app builds and opens.

- [ ] **Step 2: Start a tether session**

In Library, click **Start tethering**, pick an empty folder. Status shows *Watching <folder>*.

- [ ] **Step 3: Simulate a capture**

Copy a sample scan into the watched folder, e.g.:
`cp samples/<some-negative>.dng "<watched-folder>/shot-001.dng"`
Expected: within ~1–2 s the app imports + develops it; with *Jump to newest* on, the Develop view shows the positive; status shows *Last: shot-001.dng*.

- [ ] **Step 4: Verify per-roll base inheritance**

Calibrate the base on the first frame (Develop base tool), then copy a second scan in. Expected: the second frame develops using the same folder base automatically.

- [ ] **Step 5: Verify error resilience**

Copy a non-image file (e.g. a `.txt`) into the folder — nothing happens (filtered). Copy a truncated/corrupt file — status shows *Failed: …* and watching continues.

- [ ] **Step 6: Stop**

Click **Stop tethering**. Copy another scan in — it is NOT imported. Confirms the watcher dropped.

---

## Self-Review Notes

- **Spec coverage:** watcher (Tasks 1–3), roll = watched folder + base inheritance (Task 6 `selectedFolder.set`, verified Task 9 Step 4), reuse existing import/develop pipeline (Task 5), auto-advance toggle (Tasks 4/5/8), status + error resilience (Tasks 5/8/9), single-session replacement (Task 3 slot replace), filters for sidecar/temp/dup (Task 1), stability gate (Task 2), tests Rust + Vitest (Tasks 1–3, 5), i18n EN/ZH (Task 7). Out-of-scope items (SDK/PTP/live-view) intentionally absent.
- **No app-restart auto-resume** — matches spec ("explicit Start", out of scope).
- **De-dup of rapid duplicate events** is handled implicitly: duplicate events spawn duplicate stabilize threads, but `import_image` upserts by path (same catalog id) and `processNewFile`'s store update dedupes by id, so a double-fire converges to one entry. Acceptable for v1.
