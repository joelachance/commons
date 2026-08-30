//! Entries on disk.
//!
//! One entry is one Markdown file with a small YAML front matter block, so the
//! corpus stays readable by every other tool the user owns — `grep`, an editor,
//! Obsidian — and survives this app. Nothing here is a database: the front
//! matter is the record, and the index is rebuilt by reading the directory.
//!
//! Organisation is metadata, never path. When the sorting pass lands it writes
//! `topic:` and `tags:` back into the front matter; it must never move a file,
//! because a moved file breaks every link the user made to it by hand.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    /// Unix seconds. The id sorts the same way; this is what the UI formats.
    pub created_at: u64,
    /// "text" or "voice" - where the words came from.
    pub source: String,
    /// Assigned by the sorting pass. Empty until it has run.
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub text: String,
}

pub fn root() -> PathBuf {
    if let Ok(dir) = std::env::var("COMMONS_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".commons")
}

fn entries_dir() -> PathBuf {
    root().join("entries")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Sortable, collision-free, and legible in a directory listing:
/// `20250829T204233-3f2a`. Two entries in the same second get different
/// suffixes, which matters because dictation and typing can land together.
///
/// The stamp is UTC. `created_at` is what the UI formats, so nothing the user
/// reads depends on this; making it local would only mean ids that stop
/// sorting chronologically twice a year.
fn new_id(created: u64) -> String {
    let (y, mo, d, h, mi, s) = civil_from_unix(created);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{y:04}{mo:02}{d:02}T{h:02}{mi:02}{s:02}-{:04x}", nanos & 0xffff)
}

/// Days-from-civil, in reverse: the standard Howard Hinnant algorithm. Avoids
/// pulling in a date crate for the one thing we need it for.
fn civil_from_unix(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (
        y,
        m,
        d,
        (rem / 3600) as u32,
        (rem % 3600 / 60) as u32,
        (rem % 60) as u32,
    )
}

fn escape(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', r"\\").replace('"', "\\\""))
}

fn render(entry: &Entry) -> String {
    let tags = entry
        .tags
        .iter()
        .map(|t| escape(t))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "---\nid: {}\ncreated_at: {}\nsource: {}\ntopic: {}\ntags: [{}]\n---\n\n{}\n",
        entry.id,
        entry.created_at,
        entry.source,
        escape(&entry.topic),
        tags,
        entry.text.trim_end()
    )
}

/// Reads back what `render` wrote. Unknown keys are ignored so a future field
/// never makes an older entry unreadable, and a file without front matter is
/// still an entry - its whole body is the text.
fn parse(id: &str, raw: &str) -> Entry {
    let mut entry = Entry {
        id: id.to_string(),
        created_at: 0,
        source: "text".into(),
        topic: String::new(),
        tags: Vec::new(),
        text: String::new(),
    };
    let Some(rest) = raw.strip_prefix("---\n") else {
        entry.text = raw.trim().to_string();
        return entry;
    };
    let Some((front, body)) = rest.split_once("\n---\n") else {
        entry.text = raw.trim().to_string();
        return entry;
    };
    for line in front.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = unquote(value.trim());
        match key.trim() {
            "id" => entry.id = value,
            "created_at" => entry.created_at = value.parse().unwrap_or(0),
            "source" => entry.source = value,
            "topic" => entry.topic = value,
            "tags" => {
                entry.tags = value
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .split(',')
                    .map(|t| unquote(t.trim()))
                    .filter(|t| !t.is_empty())
                    .collect()
            }
            _ => {}
        }
    }
    entry.text = body.trim().to_string();
    entry
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        v[1..v.len() - 1].replace("\\\"", "\"").replace(r"\\", "\\")
    } else {
        v.to_string()
    }
}

pub fn create(text: &str, source: &str) -> Result<Entry, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("entry is empty".into());
    }
    let created_at = now_secs();
    let entry = Entry {
        id: new_id(created_at),
        created_at,
        source: source.to_string(),
        topic: String::new(),
        tags: Vec::new(),
        text: text.to_string(),
    };
    let dir = entries_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(dir.join(format!("{}.md", entry.id)), render(&entry)).map_err(|e| e.to_string())?;
    Ok(entry)
}

/// Every entry, oldest first - the order the UI stacks them in, so the newest
/// sits closest to the input box.
pub fn list() -> Vec<Entry> {
    let mut entries: Vec<Entry> = match fs::read_dir(entries_dir()) {
        Ok(rd) => rd
            .filter_map(|e| {
                let path = e.ok()?.path();
                if path.extension()? != "md" {
                    return None;
                }
                let id = path.file_stem()?.to_string_lossy().into_owned();
                Some(parse(&id, &fs::read_to_string(&path).ok()?))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    entries
}

/// Ids are generated, never user input, but they still reach the filesystem;
/// keep the check next to the path join so it cannot be forgotten.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() < 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub fn delete(id: &str) -> Result<(), String> {
    if !valid_id(id) {
        return Err("invalid entry id".into());
    }
    fs::remove_file(entries_dir().join(format!("{id}.md"))).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_front_matter() {
        let entry = Entry {
            id: "20250829T204233-3f2a".into(),
            created_at: 1_756_500_153,
            source: "voice".into(),
            topic: "kitchen \"remodel\"".into(),
            tags: vec!["house".into(), "budget".into()],
            text: "Get a second quote.\n\nAsk about tile.".into(),
        };
        let back = parse(&entry.id, &render(&entry));
        assert_eq!(back.id, entry.id);
        assert_eq!(back.created_at, entry.created_at);
        assert_eq!(back.source, "voice");
        assert_eq!(back.topic, "kitchen \"remodel\"");
        assert_eq!(back.tags, vec!["house", "budget"]);
        assert_eq!(back.text, entry.text);
    }

    #[test]
    fn a_plain_markdown_file_is_still_an_entry() {
        let entry = parse("hand-written", "  just some notes\n");
        assert_eq!(entry.text, "just some notes");
        assert_eq!(entry.source, "text");
    }

    #[test]
    fn ids_sort_chronologically_and_stay_unique() {
        // `date -u -r 1756500153` is 2025-08-29T20:42:33.
        let a = new_id(1_756_500_153);
        let b = new_id(1_756_500_154);
        assert!(a.starts_with("20250829T204233-"), "unexpected id: {a}");
        assert!(a < b, "{a} should sort before {b}");
        assert!(valid_id(&a));
    }
}
