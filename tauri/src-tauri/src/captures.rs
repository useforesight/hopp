//! The captures window's way into the store of captured calls. The rules are in
//! `hopp_capture::browse`; this keeps the one database connection and the call being read.

use std::sync::{Mutex, MutexGuard, PoisonError};

use hopp_capture::{
    browse::{self, Listed, Reading, Update},
    store::Store,
};
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
}

impl Captures {
    fn open(&self) -> MutexGuard<'_, Open> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
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
