//! Deterministic fault points used by the end-to-end recovery suite.
//!
//! This module is compiled only with the `fault-injection` feature. A fault
//! directory contains `<point>.action` files whose contents are either `pause`
//! or `error`. Reaching a point creates `<point>.reached`. A paused point
//! continues after its action file is removed, which also lets an external test
//! process kill the server at an exact durable boundary.

use parking_lot::RwLock;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use crate::error::StorageError;

const FAULT_DIR_ENV: &str = "STELLARDB_FAULT_DIR";
static OVERRIDE_DIR: LazyLock<RwLock<Option<PathBuf>>> = LazyLock::new(|| RwLock::new(None));

/// Process-local fault directory override for in-process integration scenarios.
///
/// External server tests should set `STELLARDB_FAULT_DIR` on the child process
/// instead. Only one process-local session may be active at a time.
pub struct FaultSession {
    previous: Option<PathBuf>,
}

impl FaultSession {
    pub fn activate(path: impl Into<PathBuf>) -> Self {
        let mut current = OVERRIDE_DIR.write();
        let previous = current.replace(path.into());
        Self { previous }
    }
}

impl Drop for FaultSession {
    fn drop(&mut self) {
        *OVERRIDE_DIR.write() = self.previous.take();
    }
}

fn fault_dir() -> Option<PathBuf> {
    OVERRIDE_DIR
        .read()
        .clone()
        .or_else(|| std::env::var_os(FAULT_DIR_ENV).map(PathBuf::from))
}

fn marker_path(directory: &Path, point: &str, suffix: &str) -> PathBuf {
    directory.join(format!("{point}.{suffix}"))
}

/// Reach a named deterministic fault boundary.
///
/// With no configured action this is a no-op. `error` is one-shot; `pause`
/// blocks until the controller removes the action file.
pub(crate) fn check(point: &str) -> Result<(), StorageError> {
    let Some(directory) = fault_dir() else {
        return Ok(());
    };
    let action_path = marker_path(&directory, point, "action");
    let action = match std::fs::read_to_string(&action_path) {
        Ok(action) => action,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(StorageError::Io(error.to_string())),
    };

    std::fs::create_dir_all(&directory).map_err(|error| StorageError::Io(error.to_string()))?;
    std::fs::write(marker_path(&directory, point, "reached"), [])
        .map_err(|error| StorageError::Io(error.to_string()))?;

    match action.trim() {
        "pause" => {
            while action_path.exists() {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(())
        }
        "error" => {
            match std::fs::remove_file(&action_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(StorageError::Io(error.to_string())),
            }
            Err(StorageError::Other(format!("fault injected at '{point}'")))
        }
        value => Err(StorageError::InvalidConfig(format!(
            "unknown fault action '{value}' for '{point}'"
        ))),
    }
}
