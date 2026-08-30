// The panel's behaviour. Kept to one file with no build step, the same way
// hv's overlay is: a note-taking surface should be editable in the time it
// takes to have the idea for the change.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const stream = document.getElementById("stream");
const empty = document.getElementById("empty");
const input = document.getElementById("input");
const mic = document.getElementById("mic");
const meter = document.getElementById("meter");
const inputRow = document.getElementById("inputrow");
const notice = document.getElementById("notice");
const status = document.getElementById("status");
const panel = document.getElementById("panel");

let entries = [];
let recording = false;
let transcribing = false;
let levelTimer = null;

// --- rendering --------------------------------------------------------------

/** Relative for the first day, then the date. Precision nobody needs is noise. */
function when(secs) {
  const d = new Date(secs * 1000);
  const mins = Math.floor((Date.now() - d) / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  if (mins < 24 * 60) return `${Math.floor(mins / 60)}h ago`;
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function render(freshId) {
  stream.querySelectorAll(".entry").forEach((n) => n.remove());
  empty.hidden = entries.length > 0;
  for (const entry of entries) {
    const el = document.createElement("article");
    el.className = "entry" + (entry.id === freshId ? " fresh" : "");
    el.dataset.id = entry.id;

    const body = document.createElement("div");
    body.className = "body";
    body.textContent = entry.text;

    const meta = document.createElement("div");
    meta.className = "meta";
    const time = document.createElement("span");
    time.textContent = when(entry.created_at);
    meta.append(time);
    if (entry.source === "voice") {
      const src = document.createElement("span");
      src.textContent = "· voice";
      meta.append(src);
    }
    if (entry.topic) {
      const topic = document.createElement("span");
      topic.className = "topic";
      topic.textContent = entry.topic;
      meta.append(topic);
    }
    const kill = document.createElement("button");
    kill.className = "kill";
    kill.textContent = "×";
    kill.title = "Delete";
    kill.addEventListener("click", () => remove(entry.id));
    meta.append(kill);

    el.append(body, meta);
    stream.append(el);
  }
  status.textContent = entries.length ? `${entries.length}` : "";
}

function toBottom() {
  stream.scrollTop = stream.scrollHeight;
}

async function refresh() {
  entries = await invoke("list_entries");
  render();
  toBottom();
}

// --- saving -----------------------------------------------------------------

function say(message) {
  notice.textContent = message;
  notice.hidden = !message;
}

async function save(source) {
  const text = input.value.trim();
  if (!text) return;
  try {
    const entry = await invoke("create_entry", { text, source });
    entries.push(entry);
    input.value = "";
    grow();
    say("");
    render(entry.id);
    toBottom();
  } catch (e) {
    say(String(e));
  }
}

async function remove(id) {
  try {
    await invoke("delete_entry", { id });
    entries = entries.filter((e) => e.id !== id);
    render();
  } catch (e) {
    say(String(e));
  }
}

/** The textarea grows with its content up to the max-height set in CSS. */
function grow() {
  input.style.height = "auto";
  input.style.height = `${input.scrollHeight}px`;
}

// --- dictation --------------------------------------------------------------
//
// Voice does not save on its own: the transcript lands in the input, where it
// can be read and fixed before it becomes an entry. Parakeet is good, and it
// is still not as good as a glance.

async function startDictation() {
  if (recording || transcribing) return;
  say("");
  try {
    await invoke("start_dictation");
  } catch (e) {
    say(String(e));
    return;
  }
  recording = true;
  mic.classList.add("listening");
  inputRow.classList.add("listening");
  input.placeholder = "Listening…";
  levelTimer = setInterval(async () => {
    const state = await invoke("dictation_state");
    if (state.error) {
      say(state.error);
      await stopDictation({ discard: true });
      return;
    }
    // Speech sits low in the linear range; the cube root opens it out so the
    // meter reads as loudness rather than as amplitude.
    const scale = Math.min(1, Math.cbrt(state.level * 12));
    meter.style.transform = `scale(${0.35 + scale * 0.75})`;
  }, 60);
}

function stopMeter() {
  clearInterval(levelTimer);
  levelTimer = null;
  meter.style.transform = "scale(0)";
  mic.classList.remove("listening");
  inputRow.classList.remove("listening");
  input.placeholder = "Write a thought…";
}

async function stopDictation({ discard = false } = {}) {
  if (!recording) return;
  recording = false;
  stopMeter();
  if (discard) {
    await invoke("cancel_dictation");
    return;
  }
  transcribing = true;
  mic.classList.add("busy");
  input.placeholder = "Transcribing…";
  try {
    const text = await invoke("stop_dictation");
    if (text) {
      input.value = input.value ? `${input.value.trimEnd()} ${text}` : text;
      grow();
    } else {
      say("Nothing was heard.");
    }
  } catch (e) {
    say(String(e));
  } finally {
    transcribing = false;
    mic.classList.remove("busy");
    input.placeholder = "Write a thought…";
    input.focus();
  }
}

function toggleDictation() {
  if (recording) stopDictation();
  else startDictation();
}

// --- wiring -----------------------------------------------------------------

input.addEventListener("input", grow);

input.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    save("text");
  }
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    e.preventDefault();
    // Escape during a take abandons it rather than dismissing the panel;
    // otherwise a stray press throws away words you just said.
    if (recording) stopDictation({ discard: true });
    else invoke("hide_panel");
    return;
  }
  if (e.key.toLowerCase() === "d" && e.metaKey) {
    e.preventDefault();
    toggleDictation();
  }
});

mic.addEventListener("click", toggleDictation);

// Clicking anywhere that is not an entry puts the caret back in the input.
document.addEventListener("mousedown", (e) => {
  if (!e.target.closest(".entry") && e.target !== input) {
    setTimeout(() => input.focus(), 0);
  }
});

listen("commons:shown", async () => {
  panel.classList.add("entering");
  requestAnimationFrame(() => panel.classList.remove("entering"));
  await refresh();
  input.focus();
});

listen("commons:hidden", () => {
  // A take must not keep the microphone open behind a hidden panel.
  if (recording) stopDictation({ discard: true });
});

// Timestamps go stale while the panel sits open.
setInterval(() => {
  if (!document.hidden && entries.length) render();
}, 60_000);

refresh();
grow();
