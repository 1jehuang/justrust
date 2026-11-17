//! Request-owned subprocesses. Intended as a private child module of `resident`.
//!
//! A child borrows the parent's delegated execution slot. This API does not mint
//! capacity: the caller must suspend its CPU work while the child uses that slot.
//! Additional parallel work in the child must acquire inherited jobserver tokens.
//! No process group is changed, so worker cancellation still reaches descendants.

use std::io;
use std::process::{Child, Command, ExitStatus};
use std::sync::Arc;

use super::{RequestClient, RequestInner};

/// Spawn with the request's inherited jobserver and tracked lifetime.
///
/// Consume the command: jobserver configuration installs inheritable descriptor
/// hooks, so returning a configured Command would permit untracked later spawns.
/// Environment entries are preserved except CARGO_MAKEFLAGS, which must describe
/// this request's actual jobserver. No descriptors or stdio handles escape.
/// Commands must not require the parent to drain piped stdout/stderr, since this
/// deliberately narrow wrapper provides no access to those handles.
pub(super) fn spawn(client: &RequestClient, mut command: Command) -> io::Result<RequestChild> {
    client.check_active()?;
    let owner = Arc::clone(&client.inner);
    owner.client.configure(&mut command);
    // Check again after configuration. Concurrent request termination still sees
    // our tracked ownership and permanently closes admission, never reuses it.
    client.check_active()?;
    let child = command.spawn()?;
    Ok(RequestChild { child, owner: Some(owner) })
}

/// Owns both the OS child and the request until successful reaping.
///
/// There is no raw Child, stdio, jobserver client, or token conversion. Forgetting
/// this object keeps request ownership outstanding and therefore fails closed.
/// Descendants must obey the same structured lifetime contract as the command:
/// the immediate child must not daemonize or leave jobserver users behind.
#[must_use]
pub struct RequestChild {
    child: Child,
    owner: Option<Arc<RequestInner>>,
}

impl RequestChild {
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Release request ownership only after the OS confirms child termination.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = loop {
            match self.child.wait() {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => break result?,
            }
        };
        self.owner.take();
        Ok(status)
    }

    /// A pending child or failed wait retains request ownership.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        if status.is_some() {
            self.owner.take();
        }
        Ok(status)
    }
}

impl Drop for RequestChild {
    fn drop(&mut self) {
        if self.owner.is_none() {
            return;
        }
        // Even if kill fails (for example the child already exited), wait must
        // still reap. Never interpret a kill result as evidence of termination.
        let _ = self.child.kill();
        if self.wait().is_err() {
            // Reaping failure gives no safe proof that jobserver users stopped.
            // Retain the request forever rather than return its reserved token
            // and admit another compiler while the child might still be alive.
            if let Some(owner) = self.owner.take() {
                std::mem::forget(owner);
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::jobserver::resident::{Admission, BeginError};
    use jobserver_crate::Client;

    fn admission() -> &'static Admission {
        Box::leak(Box::new(Admission::new()))
    }

    fn shell(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }

    #[test]
    fn live_child_prevents_finish_and_drop_reaps() {
        let admission = admission();
        let supplied = Client::new(1).unwrap();
        let lease = admission.begin(supplied.clone()).unwrap();
        let mut child = spawn(&lease.client(), shell("exec sleep 60")).unwrap();
        assert!(child.id() > 0);
        assert!(child.try_wait().unwrap().is_none());
        assert!(matches!(lease.finish(), Err(BeginError::OutstandingHandles)));
        assert_eq!(supplied.available().unwrap(), 0);
        drop(child);
        assert_eq!(supplied.available().unwrap(), 1);
        assert!(matches!(admission.begin(supplied), Err(BeginError::OutstandingHandles)));
    }

    #[test]
    fn explicit_wait_releases_owner_and_preserves_exit_status() {
        let admission = admission();
        let lease = admission.begin(Client::new(1).unwrap()).unwrap();
        let mut child = spawn(&lease.client(), shell("exit 7")).unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(7));
        // The wrapper itself may remain alive after successful reaping.
        lease.finish().unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(7));
    }

    #[test]
    fn drop_reaps_before_releasing_owner() {
        let admission = admission();
        let lease = admission.begin(Client::new(1).unwrap()).unwrap();
        let child = spawn(&lease.client(), shell("exec sleep 60")).unwrap();
        drop(child);
        lease.finish().unwrap();
        admission.begin(Client::new(1).unwrap()).unwrap().finish().unwrap();
    }

    #[test]
    fn failed_spawn_and_inactive_request_do_not_escape_ownership() {
        let admission = admission();
        let lease = admission.begin(Client::new(1).unwrap()).unwrap();
        assert!(spawn(&lease.client(), Command::new("/justrust-missing-child-command")).is_err());
        let client = lease.client();
        assert!(matches!(lease.finish(), Err(BeginError::OutstandingHandles)));
        assert!(spawn(&client, shell("exit 0")).is_err());
    }

    const PROBE: &str = "JR_RESIDENT_CHILD_ENV_PROBE";

    #[test]
    fn inherited_environment_probe() {
        if std::env::var_os(PROBE).is_none() {
            return;
        }
        assert_eq!(std::env::var(PROBE).unwrap(), "preserved");
        assert_eq!(std::env::var("MAKEFLAGS").unwrap(), "preserved-makeflags");
        assert_eq!(std::env::var("MFLAGS").unwrap(), "preserved-mflags");
        assert_ne!(std::env::var("CARGO_MAKEFLAGS").unwrap(), "invalid-parent-value");
        // SAFETY: this isolated subprocess was launched with the request client's
        // configure hook, and no other test is running here to recycle its fds.
        let inherited = unsafe { Client::from_env_ext(true) }.client.expect("valid inherited fds");
        let token = inherited.acquire().unwrap();
        drop(token);
    }

    #[test]
    fn child_inherits_valid_jobserver_and_other_environment() {
        let admission = admission();
        let lease = admission.begin(Client::new(2).unwrap()).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        let module = module_path!().split_once("::").unwrap().1;
        let probe = format!("{module}::inherited_environment_probe");
        command
            .args(["--exact", &probe, "--nocapture"])
            .env(PROBE, "preserved")
            .env("MAKEFLAGS", "preserved-makeflags")
            .env("MFLAGS", "preserved-mflags")
            .env("CARGO_MAKEFLAGS", "invalid-parent-value");
        let mut child = spawn(&lease.client(), command).unwrap();
        assert!(child.wait().unwrap().success());
        lease.finish().unwrap();
    }
}
