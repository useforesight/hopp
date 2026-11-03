//! Where Hopp keeps its files, following the XDG base directories.
//!
//! The client used to be called `hopp-omarchy`; its configuration directory is moved
//! to the new name the first time it is needed, so a sign-in survives the rename.

use anyhow::{Context, Result};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};

/// Directory name under each XDG base directory, and the runtime socket directory.
pub const APP: &str = "hopp";
const LEGACY: &str = "hopp-omarchy";

fn base(variable: &str, fallback: &str) -> Result<PathBuf> {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::home_dir().map(|home| home.join(fallback)))
        .with_context(|| format!("No {variable} or home directory"))
}

/// `~/.config/hopp`: settings and hooks.
pub fn config_dir() -> Result<PathBuf> {
    let base = base("XDG_CONFIG_HOME", ".config")?;
    migrate(&base);
    Ok(base.join(APP))
}

/// `~/.local/share/hopp`: captures and transcription models.
pub fn data_dir() -> Result<PathBuf> {
    Ok(base("XDG_DATA_HOME", ".local/share")?.join(APP))
}

/// Create `path` and any missing parents, private to this user.
pub fn private_dir(path: &Path) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .with_context(|| format!("Creating {}", path.display()))
}

/// Move `<base>/hopp-omarchy` to `<base>/hopp` once. A rename keeps permissions and
/// is atomic; if both exist the new one wins and the old one is left alone.
fn migrate(base: &Path) {
    let (old, new) = (base.join(LEGACY), base.join(APP));
    if fs::symlink_metadata(&new).is_err()
        && fs::symlink_metadata(&old).is_ok_and(|metadata| metadata.is_dir())
        && let Err(error) = fs::rename(&old, &new)
    {
        crate::logger::line(format!(
            "Could not move {} to {}: {error}",
            old.display(),
            new.display()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn moves_the_old_configuration_once_and_keeps_its_permissions() -> Result<()> {
        let base = tempfile::tempdir()?;
        let old = base.path().join(LEGACY);
        private_dir(&old)?;
        fs::write(old.join("settings.json"), "{}")?;
        fs::set_permissions(old.join("settings.json"), fs::Permissions::from_mode(0o600))?;
        migrate(base.path());
        let new = base.path().join(APP);
        assert!(!old.exists());
        assert_eq!(fs::read_to_string(new.join("settings.json"))?, "{}");
        assert_eq!(fs::metadata(&new)?.permissions().mode() & 0o777, 0o700);
        assert_eq!(
            fs::metadata(new.join("settings.json"))?
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        Ok(())
    }

    #[test]
    fn never_replaces_an_existing_configuration() -> Result<()> {
        let base = tempfile::tempdir()?;
        private_dir(&base.path().join(LEGACY))?;
        fs::write(base.path().join(LEGACY).join("settings.json"), "old")?;
        private_dir(&base.path().join(APP))?;
        fs::write(base.path().join(APP).join("settings.json"), "new")?;
        migrate(base.path());
        assert_eq!(
            fs::read_to_string(base.path().join(APP).join("settings.json"))?,
            "new"
        );
        assert!(base.path().join(LEGACY).exists());
        Ok(())
    }
}
