//! Retention and model deletion, against real folders. A binary of its own with one test,
//! because the crate's paths come from an environment variable no other test may see.

use std::fs;

use anyhow::Result;
use hopp_capture::{
    config::{self, Config},
    markdown_path, models, moments, prune_in,
    store::{Store, now_ms},
};

const DAY_MS: i64 = 24 * 3600 * 1000;

#[test]
fn housekeeping_takes_only_what_nothing_needs() -> Result<()> {
    let home = tempfile::tempdir()?;
    // SAFETY: the only test in this binary, before anything reads the environment.
    unsafe { std::env::set_var("HOPP_CAPTURE_HOME", home.path()) };
    let store = Store::open_default()?;
    old_captures_go_with_their_files(&store)?;
    a_model_in_use_stays(&store)?;
    Ok(())
}

fn old_captures_go_with_their_files(store: &Store) -> Result<()> {
    let now = now_ms();
    for (id, started_ms, finished) in [
        ("c_old", now - 40 * DAY_MS, true),
        ("c_recent", now - DAY_MS, true),
        ("c_old_but_running", now - 40 * DAY_MS, false),
    ] {
        store.begin_call(id, "Standup", started_ms, "tiny.en")?;
        if finished {
            store.end_call(id, started_ms + 1000)?;
        }
        fs::write(markdown_path(id)?, "# Standup\n")?;
        let frames = moments::dir(id)?;
        fs::create_dir_all(&frames)?;
        fs::write(frames.join("frame-001.webp"), [0; 4])?;
    }

    assert_eq!(prune_in(store)?, 0, "nothing goes unless it's asked for");
    config::save(&Config {
        keep_days: Some(30),
        ..config::load()
    })?;
    assert_eq!(prune_in(store)?, 1);

    assert!(store.call("c_old")?.is_none());
    assert!(!markdown_path("c_old")?.exists());
    assert!(!moments::dir("c_old")?.exists());
    for id in ["c_recent", "c_old_but_running"] {
        assert!(store.call(id)?.is_some(), "{id}");
        assert!(markdown_path(id)?.exists(), "{id}");
        assert!(moments::dir(id)?.exists(), "{id}");
    }

    // A prune that stopped after the files, or another process's, leaves this to finish.
    store.begin_call("c_half_done", "Standup", now - 40 * DAY_MS, "tiny.en")?;
    store.end_call("c_half_done", now - 40 * DAY_MS + 1000)?;
    assert_eq!(prune_in(store)?, 1);
    assert!(store.call("c_half_done")?.is_none());
    Ok(())
}

fn a_model_in_use_stays(store: &Store) -> Result<()> {
    let model = models::find("base.en")?;
    let file = models::path(model)?;
    fs::create_dir_all(file.parent().expect("a folder"))?;
    fs::write(&file, [0; 4])?;
    let refused = |store| {
        models::delete(store, model)
            .expect_err("refused")
            .to_string()
    };

    let mut chosen = config::load();
    let previous = chosen.model.clone();
    chosen.model = model.name.into();
    config::save(&chosen)?;
    assert!(refused(Some(store)).contains("choose another"));
    chosen.model = previous;
    config::save(&chosen)?;

    store.begin_call("c_transcribing", "Standup", now_ms(), model.name)?;
    assert!(refused(Some(store)).contains("still using"));
    assert!(file.exists());

    store.end_call("c_transcribing", now_ms())?;
    models::delete(Some(store), model)?;
    assert!(!file.exists());

    fs::write(&file, [0; 4])?;
    models::delete(None, model)?;
    assert!(
        !file.exists(),
        "nothing captured yet, so nothing is using it"
    );
    Ok(())
}
