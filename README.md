# commons

A notes app that gets out of the way. `⌥⌘Space` brings up a panel on the right
third of the screen; type or dictate a thought; `↩` files it. The same hotkey
puts it away.

Everything is local. Dictation runs NVIDIA's Parakeet on your own machine
through MLX - no audio leaves it, and no API key is involved.

## Running it

```bash
make run
```

The first run builds a Python environment for dictation (~1GB of wheels) and
uses the Parakeet weights from your Hugging Face cache, downloading them once
if they are not there.

Commons has no Dock icon. It lives in the menu bar, and follows you onto every
Space including over a fullscreen app.

## Keys

| | |
|---|---|
| `⌥⌘Space` | show / hide the panel |
| `↩` | save the entry |
| `⇧↩` | newline |
| `⌘D` | start / stop dictation |
| `esc` | hide the panel, or abandon a recording |

Dictation drops its transcript into the input rather than saving it, so you can
read it before it becomes an entry.

## Where notes go

`~/.commons/entries/`, one Markdown file per entry with front matter:

```markdown
---
id: 20250829T204233-3f2a
created_at: 1756500153
source: voice
topic: ""
tags: []
---

Call the roofer about the skylight estimate on Thursday.
```

`topic` and `tags` are what the sorting pass fills in. It does not exist yet -
this is the capture half. Files are never moved, so anything you organise by
hand stays organised.
