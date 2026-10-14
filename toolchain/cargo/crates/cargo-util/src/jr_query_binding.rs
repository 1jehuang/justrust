//! Private, opt-in compiler observation. Never part of the logical command.
use std::fmt;
use std::path::PathBuf;
use std::process::{Command, ExitStatus};
use std::sync::Arc;

#[cfg(target_os = "linux")]
#[path = "../../../../shared/query_binding.rs"]
#[allow(dead_code, unused_imports)]
mod transport;
#[cfg(target_os = "linux")]
pub use transport::Binding;

/// Real process lifecycle observations, independent of Cargo metadata readiness.
pub trait Observer: Send + Sync {
    fn spawn_attempt(&self);
    fn spawn_result(&self, spawned: bool, retry: bool);
    fn wait(&self, status: ExitStatus);
    fn lost(&self);
}

#[cfg(target_os = "linux")]
#[derive(Clone)]
pub struct Launch {
    pub directory: PathBuf,
    pub binding: Binding,
    pub observer: Arc<dyn Observer>,
}
#[cfg(target_os = "linux")]
impl fmt::Debug for Launch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PrivateCompilerLaunch")
    }
}

#[cfg(target_os = "linux")]
impl Launch {
    pub(crate) fn materialize(&self, command: &mut Command) {
        use std::os::unix::process::CommandExt;
        let prepared = match transport::prepare(&self.directory, &self.binding) {
            Ok(prepared) => Arc::new(prepared),
            Err(_) => {
                self.observer.lost();
                return;
            }
        };
        // No argv or environment additions, including on argfile retry.
        // The closure retains owned descriptors until spawn.
        unsafe {
            command.pre_exec(move || {
                // Collision or mapping failure disables observation only. A missing
                // receipt remains incomplete, without invoking observers after fork.
                prepared.map_child();
                Ok(())
            });
        }
    }
}
