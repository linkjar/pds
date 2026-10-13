//! Fail points (`docs/SPEC.md` §19.2).
//!
//! A fail point is a named place between two steps of a write. A test or a
//! simulation arms one, and the next time the server reaches it the process
//! stops there, or the step fails. A point that is not armed costs one
//! atomic load.
//!
//! The server arms fail points from `PDS_FAIL_POINTS` in dev mode only
//! (SPEC §3.3). A production start ignores the variable.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

/// What an armed fail point does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The process aborts: no destructor runs and no buffer is flushed, as
    /// when the process is killed.
    Abort,
    /// The process aborts the first time only. The marker file records that
    /// it did, so a restart with the same setting passes the point.
    AbortOnce(PathBuf),
    /// The step fails with [`Injected`].
    Error,
}

/// The error of a step that a fail point failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("fail point {0}")]
pub struct Injected(pub &'static str);

static ARMED: AtomicBool = AtomicBool::new(false);
static POINTS: Mutex<Option<HashMap<String, Action>>> = Mutex::new(None);

/// Arms the fail point `name`.
pub fn arm(name: &str, action: Action) {
    let mut points = POINTS.lock().unwrap_or_else(PoisonError::into_inner);
    points
        .get_or_insert_with(HashMap::new)
        .insert(name.to_owned(), action);
    drop(points);
    ARMED.store(true, Ordering::Release);
}

/// Disarms every fail point.
pub fn disarm_all() {
    let mut points = POINTS.lock().unwrap_or_else(PoisonError::into_inner);
    *points = None;
    drop(points);
    ARMED.store(false, Ordering::Release);
}

/// A setting that [`arm_from`] cannot read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a fail point setting: {0}")]
pub struct BadSetting(pub String);

/// Arms fail points from a setting: `name=action` pairs separated by
/// commas, where an action is `abort`, `abort-once:<marker file>` or
/// `error`.
pub fn arm_from(setting: &str) -> Result<(), BadSetting> {
    for pair in setting.split(',').filter(|pair| !pair.is_empty()) {
        let bad = || BadSetting(pair.to_owned());
        let (name, action) = pair.split_once('=').ok_or_else(bad)?;
        let action = match action {
            "abort" => Action::Abort,
            "error" => Action::Error,
            other => match other.strip_prefix("abort-once:") {
                Some(marker) if !marker.is_empty() => Action::AbortOnce(PathBuf::from(marker)),
                _ => return Err(bad()),
            },
        };
        arm(name, action);
    }
    Ok(())
}

/// Passes the fail point `name`. It returns at once when no point is armed.
pub fn point(name: &'static str) -> Result<(), Injected> {
    if !ARMED.load(Ordering::Acquire) {
        return Ok(());
    }
    let action = {
        let points = POINTS.lock().unwrap_or_else(PoisonError::into_inner);
        points.as_ref().and_then(|points| points.get(name).cloned())
    };
    match action {
        None => Ok(()),
        Some(Action::Error) => Err(Injected(name)),
        Some(Action::Abort) => {
            tracing_abort(name);
            std::process::abort()
        }
        Some(Action::AbortOnce(marker)) => {
            if marker.exists() {
                return Ok(());
            }
            // The marker is written first. When it cannot be written the
            // point does nothing, so that a restart cannot loop.
            if std::fs::write(&marker, name).is_err() {
                return Ok(());
            }
            tracing_abort(name);
            std::process::abort()
        }
    }
}

#[allow(clippy::print_stderr)] // the last words of the process; no logger may run after
fn tracing_abort(name: &str) {
    eprintln!("fail point {name}: aborting");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setting_arms_points_and_a_bad_one_is_refused() {
        assert!(arm_from("a=explode").is_err());
        assert!(arm_from("a").is_err());
        assert!(arm_from("a=abort-once:").is_err());
        // Only the `error` action can be passed in a test.
        assert_eq!(point("storage.test.unarmed"), Ok(()));
        arm_from("storage.test.armed=error").unwrap();
        assert_eq!(
            point("storage.test.armed"),
            Err(Injected("storage.test.armed"))
        );
        assert_eq!(point("storage.test.unarmed"), Ok(()));
    }
}
