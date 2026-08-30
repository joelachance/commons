# Working on commons

Commons is a capture surface for notes. One hotkey summons a panel on the
right third of the screen, you say or type a thought, and it lands in a
Markdown file. Sorting the corpus is a later pass over that same directory.

## Commands

```bash
make run      # launch the panel (needs the sidecar; the target builds it)
make test     # cargo test
make sidecar  # build the Python environment dictation needs
cd src-tauri && cargo build --release
```

Nothing needs network access or an API key. The ASR model comes from the local
Hugging Face cache; the sidecar sets `HF_HUB_OFFLINE` so a first launch on a
plane behaves the same as one at a desk.

## Constraints worth knowing

**Capture never waits on a model.** Pressing the hotkey and typing must work
with the transcriber unstarted, the microphone unplugged, and the sorting pass
never having run. Anything slow is warmed in the background while the user is
already writing - which is why `start_dictation` opens the microphone first and
spawns the sidecar second, on another thread.

**The Markdown file is the record.** An entry is a file with front matter, and
that is the whole database. `grep` has to keep working, an editor has to be
able to fix a typo, and the corpus has to outlive this app.

**Organisation is metadata, never path.** When the sorting pass lands it writes
`topic:` and `tags:` back into front matter. It must not move a file: a moved
file breaks every link the user made to it by hand, and makes undo a merge.

**The microphone is closed when it is not recording.** The stream is built on
arm and dropped on disarm, so macOS shows no microphone indicator while Commons
is idle. This costs about 100ms at the top of a take, and it is worth it.

**The frame is re-asserted on every show.** macOS anchors a hidden window's
resize to its bottom edge and lets its position drift, so `overlay` recomputes
nothing but re-applies the layout it computed at setup each time the panel
appears.

## Layout

```
src-tauri/src/
  main.rs      the commands the page can call, and app setup
  overlay.rs   where the panel sits, and how it comes and goes
  store.rs     entries on disk
  audio.rs     microphone capture and WAV writing
  sidecar.rs   the Parakeet process and its JSON protocol
sidecar/       the Python side of that protocol
src/           the page: index.html, app.css, app.js - no build step
```

**The frontend has no build step on purpose.** A note-taking surface should be
editable in the time it takes to have the idea for the change, so the page is
three files loaded straight off disk with `withGlobalTauri`.

## Storage

`~/.commons/entries/<id>.md`, overridable with `COMMONS_DIR`. Ids are
`YYYYMMDDTHHMMSS-xxxx` in UTC so a directory listing sorts chronologically.
