#[path = "../../../../src/capture/config.rs"]
pub mod config;
#[path = "../../../../src/capture/frame_files.rs"]
pub mod frame_files;
#[path = "../../../../src/capture/models.rs"]
pub mod models;
#[path = "../../../../src/capture/store.rs"]
pub mod store;
#[path = "../../../../src/capture/timeline.rs"]
pub mod timeline;

pub fn prune() -> anyhow::Result<usize> {
    Ok(0)
}

pub fn disk_usage() -> (u64, u64) {
    (0, 0)
}

pub fn markdown_path(call_id: &str) -> anyhow::Result<std::path::PathBuf> {
    Ok(crate::paths::data_dir()?.join(format!("capture/{call_id}.md")))
}
