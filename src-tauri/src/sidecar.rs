//! Parakeet ASR sidecar.
//!
//! The model is 2.3GB and takes seconds to load, so it lives in a long-running
//! Python process and is loaded once per app launch rather than once per take.
//! The contract is newline-delimited JSON over stdin/stdout, which keeps the
//! model swappable (parakeet-mlx today, NeMo or ONNX later) without touching
//! Rust.
//!
//! Nothing here reaches the network. The model comes from the local Hugging
//! Face cache and the audio never leaves the machine.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use serde::Deserialize;

#[derive(Deserialize)]
struct Response {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    text: String,
}

struct Proc {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

pub struct Sidecar {
    proc: Mutex<Option<Proc>>,
    dir: PathBuf,
}

impl Sidecar {
    pub fn new() -> Self {
        let dir = std::env::var("COMMONS_SIDECAR_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .unwrap()
                    .join("sidecar")
            });
        Sidecar {
            proc: Mutex::new(None),
            dir,
        }
    }

    fn python(&self) -> PathBuf {
        let venv = self.dir.join(".venv/bin/python");
        if venv.exists() {
            venv
        } else {
            PathBuf::from("python3")
        }
    }

    /// Starts the sidecar and blocks until the model reports ready. Idempotent,
    /// so the first dictation can call it and pay the load cost once.
    pub fn spawn(&self) -> Result<(), String> {
        let mut slot = self.proc.lock().unwrap();
        if slot.is_some() {
            return Ok(());
        }
        let script = self.dir.join("transcribe.py");
        if !script.exists() {
            return Err(format!("sidecar script not found at {}", script.display()));
        }
        let mut child = Command::new(self.python())
            .arg(&script)
            .current_dir(&self.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("failed to spawn the transcriber: {e}"))?;

        let stdin = child.stdin.take().ok_or("no sidecar stdin")?;
        let mut stdout = BufReader::new(child.stdout.take().ok_or("no sidecar stdout")?);

        let mut line = String::new();
        stdout
            .read_line(&mut line)
            .map_err(|e| format!("transcriber handshake failed: {e}"))?;
        if !line.contains("\"ready\"") {
            let _ = child.kill();
            return Err(format!(
                "transcriber did not start (run `make sidecar`): {}",
                line.trim()
            ));
        }

        *slot = Some(Proc {
            child,
            stdin,
            stdout,
        });
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.proc.lock().unwrap().is_some()
    }

    /// Transcribes a WAV file to text.
    pub fn transcribe(&self, wav: &Path) -> Result<String, String> {
        let mut slot = self.proc.lock().unwrap();
        let proc = slot.as_mut().ok_or("transcriber is not running")?;

        let req = serde_json::json!({ "path": wav.to_string_lossy() });
        writeln!(proc.stdin, "{req}").map_err(|e| format!("transcriber write failed: {e}"))?;
        proc.stdin
            .flush()
            .map_err(|e| format!("transcriber flush failed: {e}"))?;

        let mut line = String::new();
        let n = proc
            .stdout
            .read_line(&mut line)
            .map_err(|e| format!("transcriber read failed: {e}"))?;
        if n == 0 {
            // The child died; drop it so the next take can respawn it.
            if let Some(mut p) = slot.take() {
                let _ = p.child.kill();
            }
            return Err("transcriber closed unexpectedly".into());
        }

        let resp: Response =
            serde_json::from_str(&line).map_err(|e| format!("bad transcriber response: {e}"))?;
        if !resp.ok {
            return Err(resp.error.unwrap_or_else(|| "transcription failed".into()));
        }
        Ok(resp.text.trim().to_string())
    }
}
