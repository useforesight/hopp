//! The captures window's way into the store of captured calls. The rules are in
//! `hopp_capture::browse`; this keeps the one database connection and the call being read.

use std::sync::{Mutex, MutexGuard, PoisonError};

use hopp_capture::{
    browse::{self, Listed, Reading, Update},
    config, disk_usage, liveness, models,
    paths::{data_dir, private_dir},
    prune_in,
    store::{self, Store},
};
use serde::{Deserialize, Serialize};
use tauri::{ipc::Response, State};
use tauri_plugin_opener::OpenerExt;

/// One connection for the life of the app. Closing any descriptor of a database drops all of
/// this process's locks on it, so it's never opened twice.
#[derive(Default)]
pub struct Captures(Mutex<Open>);

#[derive(Default)]
struct Open {
    store: Option<Store>,
    reading: Option<Reading>,
}

impl Open {
    fn store(&mut self) -> Result<&Store, String> {
        if self.store.is_none() {
            self.store = Some(Store::open_default().map_err(describe)?);
        }
        self.store.as_ref().ok_or_else(String::new)
    }

    /// The store, unless nothing has been captured yet, when there's no reason to create it.
    fn existing_store(&mut self) -> Result<Option<&Store>, String> {
        let exists = self.store.is_some() || store::default_path().map_err(describe)?.exists();
        if exists {
            self.store().map(Some)
        } else {
            Ok(None)
        }
    }
}

impl Captures {
    fn open(&self) -> MutexGuard<'_, Open> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Close what a crash left open and apply `keep_days`, for someone who doesn't capture
    /// often: otherwise old captures only go when another one finishes.
    pub fn tidy(&self) -> Result<(), String> {
        let mut open = self.open();
        let Some(store) = open.existing_store()? else {
            return Ok(());
        };
        liveness::close_orphans(store).map_err(describe)?;
        prune_in(store).map_err(describe)?;
        Ok(())
    }
}

fn describe(error: impl std::fmt::Display) -> String {
    format!("{error:#}")
}

/// What `captures_poll` fails with when the call being read has been deleted. Other errors
/// may pass, and the window carries on.
const CALL_GONE: &str = "gone";

#[tauri::command(async)]
pub fn captures_list(captures: State<'_, Captures>, query: String) -> Result<Vec<Listed>, String> {
    let mut open = captures.open();
    browse::list(open.store()?, &query).map_err(describe)
}

/// Starts reading a call: an id, `last` or `live`. The update has all of it.
#[tauri::command(async)]
pub fn captures_open(captures: State<'_, Captures>, which: String) -> Result<Update, String> {
    let mut open = captures.open();
    let (reading, update) = Reading::open(open.store()?, &which).map_err(describe)?;
    open.reading = Some(reading);
    Ok(update)
}

/// What's changed in the call being read, if anything.
#[tauri::command(async)]
pub fn captures_poll(captures: State<'_, Captures>) -> Result<Option<Update>, String> {
    let mut open = captures.open();
    open.store()?;
    let Open { store, reading } = &mut *open;
    let (Some(store), Some(open_call)) = (store.as_ref(), reading.as_mut()) else {
        return Ok(None);
    };
    if open_call.is_gone(store).map_err(describe)? {
        *reading = None;
        return Err(CALL_GONE.to_string());
    }
    open_call.poll(store).map_err(describe)
}

/// A picture from the call being read, as bytes the window turns into an image.
#[tauri::command(async)]
pub fn captures_frame(
    captures: State<'_, Captures>,
    call_id: String,
    number: u32,
    full: bool,
) -> Result<Response, String> {
    let path = {
        let open = captures.open();
        let reading = open
            .reading
            .as_ref()
            .filter(|reading| reading.call_id() == call_id)
            .ok_or("That call isn't open")?;
        reading.picture_path(number, full).map_err(describe)?
    };
    let bytes = std::fs::read(&path).map_err(|e| format!("Reading frame {number}: {e}"))?;
    Ok(Response::new(bytes))
}

#[tauri::command(async)]
pub fn captures_delete(captures: State<'_, Captures>, call_id: String) -> Result<(), String> {
    let mut open = captures.open();
    browse::delete(open.store()?, &call_id).map_err(describe)?;
    if open
        .reading
        .as_ref()
        .is_some_and(|reading| reading.call_id() == call_id)
    {
        open.reading = None;
    }
    Ok(())
}

#[tauri::command(async)]
pub fn captures_open_markdown(
    app: tauri::AppHandle,
    captures: State<'_, Captures>,
    call_id: String,
) -> Result<(), String> {
    let path = {
        let mut open = captures.open();
        browse::markdown_file(open.store()?, &call_id).map_err(describe)?
    };
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// What the settings window shows of capture.
#[derive(Serialize)]
pub struct CaptureSettings {
    model: String,
    gpu: bool,
    drafts: bool,
    auto: bool,
    keep_days: Option<u32>,
    models: Vec<CaptureModel>,
    captures_bytes: u64,
    models_bytes: u64,
}

#[derive(Serialize)]
pub struct CaptureModel {
    name: &'static str,
    bytes: u64,
    note: &'static str,
    installed: bool,
    /// Hex's copy, which isn't Hopp's to delete.
    shared: bool,
    /// How far along its download is, as a percentage.
    downloading: Option<u8>,
    failed: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureChange {
    Model(String),
    Gpu(bool),
    Drafts(bool),
    Auto(bool),
    KeepDays(Option<u32>),
}

/// The model captures use. A name nothing knows, from a hand-edited file or an older version,
/// means the default, as it does when a capture starts.
fn chosen_model(config: &config::Config) -> Result<&'static models::Model, String> {
    models::find(&config.model)
        .or_else(|_| models::find(models::DEFAULT))
        .map_err(describe)
}

#[tauri::command(async)]
pub fn capture_settings() -> Result<CaptureSettings, String> {
    let config = config::load();
    let (captures_bytes, models_bytes) = disk_usage();
    Ok(CaptureSettings {
        model: chosen_model(&config)?.name.to_string(),
        gpu: config.gpu,
        drafts: config.drafts,
        auto: config.auto,
        keep_days: config.keep_days,
        models: models::MODELS
            .iter()
            .map(|model| {
                let state = models::download_state(model.name);
                CaptureModel {
                    name: model.name,
                    bytes: model.bytes,
                    note: model.note,
                    installed: models::installed(model),
                    shared: models::shared(model),
                    downloading: state.clone().and_then(Result::ok),
                    failed: state.and_then(Result::err),
                }
            })
            .collect(),
        captures_bytes,
        models_bytes,
    })
}

/// Saves one capture setting. Read afresh when a capture starts, so a capture already running
/// isn't touched.
#[tauri::command(async)]
pub fn capture_settings_change(
    captures: State<'_, Captures>,
    change: CaptureChange,
) -> Result<(), String> {
    let mut chosen = config::load();
    let needs_the_model = match &change {
        CaptureChange::Model(_) => true,
        CaptureChange::Auto(auto) => *auto,
        _ => false,
    };
    let prune_now = matches!(change, CaptureChange::KeepDays(Some(_)));
    match change {
        CaptureChange::Model(name) => {
            chosen.model = models::find(&name).map_err(describe)?.name.to_string()
        }
        CaptureChange::Gpu(gpu) => chosen.gpu = gpu,
        CaptureChange::Drafts(drafts) => chosen.drafts = drafts,
        CaptureChange::Auto(auto) => chosen.auto = auto,
        CaptureChange::KeepDays(days) => chosen.keep_days = days,
    }
    config::save(&chosen).map_err(describe)?;
    if needs_the_model {
        // Fetched now, so the first capture has its words from the start.
        let model = chosen_model(&chosen)?;
        if !models::installed(model) {
            models::download_in_background(model);
        }
    }
    if prune_now {
        let mut open = captures.open();
        prune_in(open.store()?).map_err(describe)?;
    }
    Ok(())
}

#[tauri::command(async)]
pub fn capture_model_download(name: String) -> Result<(), String> {
    models::download_in_background(models::find(&name).map_err(describe)?);
    Ok(())
}

#[tauri::command(async)]
pub fn capture_model_delete(captures: State<'_, Captures>, name: String) -> Result<(), String> {
    let model = models::find(&name).map_err(describe)?;
    let mut open = captures.open();
    models::delete(open.existing_store()?, model).map_err(describe)
}

#[tauri::command(async)]
pub fn capture_open_folder(app: tauri::AppHandle) -> Result<(), String> {
    let folder = data_dir().map_err(describe)?.join("capture");
    private_dir(&folder).map_err(describe)?;
    app.opener()
        .open_path(folder.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}
