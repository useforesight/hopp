//! Diagnostic lines on stderr (the journal when started from the bar), written by a
//! background thread so a slow journal can never stall the UI or the engine. When the
//! writer falls behind, lines are dropped and counted rather than queued.
use std::{
    io::Write,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
        mpsc::{SyncSender, TrySendError, sync_channel},
    },
};

static WRITER: OnceLock<Option<SyncSender<String>>> = OnceLock::new();
static DROPPED: AtomicUsize = AtomicUsize::new(0);

/// Log one line, prefixed with `hopp:`. Never blocks and never includes tokens.
pub fn line(text: impl Into<String>) {
    let writer = WRITER.get_or_init(|| {
        let (sender, lines) = sync_channel::<String>(64);
        std::thread::Builder::new()
            .name("hopp-log".into())
            .spawn(move || {
                let mut stderr = std::io::stderr();
                for line in lines {
                    let dropped = DROPPED.swap(0, Ordering::Relaxed);
                    if dropped > 0 {
                        let _ = writeln!(stderr, "hopp: ({dropped} log lines dropped)");
                    }
                    let _ = writeln!(stderr, "hopp: {line}");
                }
            })
            .ok()
            .map(|_| sender)
    });
    if let Some(sender) = writer
        && let Err(TrySendError::Full(_)) = sender.try_send(text.into())
    {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}

/// Library warnings and errors (GPUI, wgpu and the rest log through the `log` crate) into
/// the same journal lines, so a GPU error is on record before anything goes wrong.
struct Libraries;

impl log::Log for Libraries {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            line(format!(
                "{} {}: {}",
                record.level(),
                record.target(),
                record.args()
            ));
        }
    }

    fn flush(&self) {}
}

/// Start forwarding library warnings and errors. Call once, early in `main`.
pub fn capture_libraries() {
    if log::set_logger(&Libraries).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
}
