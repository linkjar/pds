//! The files the pages link: the stylesheet of `build.rs` and the browser
//! runtime of Topcoat, served from a bundle with content-hashed names.
//!
//! The typeface and the mark are `pds_oauth::theme`'s and need no bundle.
//!
//! The image carries the bundle next to the executable, made in its build
//! stage by `linkjar-pds-console bundle-assets`. A development build or a
//! test has no bundle: the executable then reads its own asset
//! declarations and copies the files into a directory under the temporary
//! directory, which needs the sources of the build on the machine.

use std::{
    env, fs, io,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use topcoat::asset::AssetBundle;
use topcoat_asset::{BundleError, Bundler, BundlerConfig};

/// Why the assets are not there.
#[derive(Debug, thiserror::Error)]
pub enum AssetsError {
    /// The bundle, or the executable, cannot be read.
    #[error("cannot read the assets of the console: {0}")]
    Io(#[from] io::Error),
    /// The executable cannot bundle its own assets.
    #[error("cannot bundle the assets of the console: {0}")]
    Bundle(#[from] BundleError),
}

/// The bundle next to the executable, or the one the executable makes for
/// itself.
pub fn load() -> Result<AssetBundle, AssetsError> {
    match AssetBundle::load() {
        Ok(bundle) => Ok(bundle),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let dir = own_dir()?;
            tracing::info!(dir = %dir.display(), "no asset bundle next to the executable; bundling");
            bundle(&dir)
        }
        Err(error) => Err(error.into()),
    }
}

/// A directory of the temporary directory for the bundle of this
/// executable: its name, size and modification time name it, so a rebuilt
/// executable gets a bundle of its own.
fn own_dir() -> Result<PathBuf, AssetsError> {
    let exe = env::current_exe()?;
    let meta = fs::metadata(&exe)?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_secs());
    let name = exe.file_name().map_or_else(
        || "console".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    Ok(env::temp_dir().join(format!(
        "linkjar-pds-console-assets-{name}-{}-{modified}",
        meta.len()
    )))
}

/// Writes the bundle of this executable into `dir` and loads it.
pub fn bundle(dir: &Path) -> Result<AssetBundle, AssetsError> {
    let binary = fs::read(env::current_exe()?)?;
    let cache = env::temp_dir().join("linkjar-pds-console-asset-cache");
    let config = BundlerConfig::new().cache_dir(cache);
    Bundler::new(&config).bundle(&binary, dir)?;
    Ok(AssetBundle::load_dir(dir)?)
}
