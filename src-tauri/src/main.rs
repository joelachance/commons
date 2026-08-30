#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Commons: a capture surface for notes.
//!
//! One hotkey summons a panel, you say or type a thought, and it lands in a
//! Markdown file. Sorting the corpus is a later pass over that same directory;
//! nothing in the capture path waits on a model, because a note you had to
//! wait to write is a note you lost.

mod audio;
mod overlay;
mod sidecar;
mod store;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::ShortcutState;

const TOGGLE_SHORTCUT: &str = "alt+cmd+space";

struct Dictation {
    recorder: audio::Recorder,
    transcriber: sidecar::Sidecar,
}

#[derive(serde::Serialize)]
struct DictationState {
    recording: bool,
    level: f32,
    error: Option<String>,
}

// --- panel ------------------------------------------------------------------

#[tauri::command]
fn hide_panel(app: AppHandle) {
    overlay::hide(&app);
}

// --- entries ----------------------------------------------------------------

#[tauri::command]
fn create_entry(text: String, source: String) -> Result<store::Entry, String> {
    store::create(&text, &source)
}

#[tauri::command]
fn list_entries() -> Vec<store::Entry> {
    store::list()
}

#[tauri::command]
fn delete_entry(id: String) -> Result<(), String> {
    store::delete(&id)
}

#[tauri::command]
fn store_path() -> String {
    store::root().to_string_lossy().into_owned()
}

// --- dictation --------------------------------------------------------------

/// Opens the microphone now and warms the model in the background. The user is
/// already talking by the time the 2.3GB of weights are resident, which is the
/// whole reason the two are not done in sequence.
#[tauri::command]
fn start_dictation(app: AppHandle) -> Result<(), String> {
    let state = app.state::<Dictation>();
    state.recorder.start()?;
    if !state.transcriber.is_running() {
        let app = app.clone();
        std::thread::spawn(move || {
            if let Err(e) = app.state::<Dictation>().transcriber.spawn() {
                eprintln!("[sidecar] {e}");
            }
        });
    }
    Ok(())
}

/// Closes the microphone and returns the transcript. Blocking work runs off the
/// main thread so the panel keeps painting its "transcribing" state.
#[tauri::command]
async fn stop_dictation(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Dictation>();
        let Some((samples, rate)) = state.recorder.stop() else {
            return Err(state
                .recorder
                .error()
                .unwrap_or_else(|| "nothing was recorded".into()));
        };
        let wav = std::env::temp_dir().join(format!("commons-{}.wav", std::process::id()));
        audio::write_wav(&wav, &samples, rate)?;
        state.transcriber.spawn()?;
        let text = state.transcriber.transcribe(&wav);
        let _ = std::fs::remove_file(&wav);
        text
    })
    .await
    .map_err(|e| format!("transcription task failed: {e}"))?
}

#[tauri::command]
fn cancel_dictation(app: AppHandle) {
    let _ = app.state::<Dictation>().recorder.stop();
}

#[tauri::command]
fn dictation_state(app: AppHandle) -> DictationState {
    let state = app.state::<Dictation>();
    DictationState {
        recording: state.recorder.is_recording(),
        level: state.recorder.level(),
        error: state.recorder.error(),
    }
}

// ---------------------------------------------------------------------------

fn main() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts([TOGGLE_SHORTCUT])
                .expect("invalid shortcut")
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        overlay::toggle(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            hide_panel,
            create_entry,
            list_entries,
            delete_entry,
            store_path,
            start_dictation,
            stop_dictation,
            cancel_dictation,
            dictation_state,
        ])
        .setup(|app| {
            // A panel, not an app: no Dock icon, no Cmd-Tab entry.
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            app.manage(Dictation {
                recorder: audio::Recorder::new(),
                transcriber: sidecar::Sidecar::new(),
            });

            let win = app.get_webview_window("main").expect("main window missing");
            overlay::install(&win)?;

            let toggle_item = tauri::menu::MenuItemBuilder::with_id("toggle", "Show/Hide Commons")
                .accelerator("Alt+Cmd+Space")
                .build(app)?;
            let quit_item =
                tauri::menu::MenuItemBuilder::with_id("quit", "Quit Commons").build(app)?;
            let menu = tauri::menu::MenuBuilder::new(app)
                .items(&[&toggle_item, &quit_item])
                .build()?;
            tauri::tray::TrayIconBuilder::with_id("commons-tray")
                .icon(tauri::image::Image::from_bytes(include_bytes!(
                    "../icons/tray.png"
                ))?)
                .menu(&menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "toggle" => overlay::toggle(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // The panel starts hidden; the hotkey is the way in.
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running commons");
}
