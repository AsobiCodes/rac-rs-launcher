//! The install engine behind a trait, so every system picks its own worker.
//! Windows: the PowerShell worker (`setup.ps1`, see engine.rs). Linux and other Unix systems: `sh setup.sh` (same protocol, same parser).
use crate::engine::{Engine, EngineOpts};
use std::io::{Error, ErrorKind};

pub trait InstallBackend: Send + Sync {
    fn name(&self) -> &'static str;
    /// Can this system set the game up from a disc at all?
    fn supported(&self) -> bool;
    fn start(&self, opts: &EngineOpts) -> std::io::Result<Engine>;
}

/// Windows: `powershell -File setup.ps1 ...` as a hidden child.
pub struct PowerShellBackend;

impl InstallBackend for PowerShellBackend {
    fn name(&self) -> &'static str {
        "PowerShell (setup.ps1)"
    }
    fn supported(&self) -> bool {
        true
    }
    fn start(&self, opts: &EngineOpts) -> std::io::Result<Engine> {
        Engine::start(opts)
    }
}

/// Linux: `sh setup.sh ...` as a child in its own process group.
pub struct ShellBackend;

impl InstallBackend for ShellBackend {
    fn name(&self) -> &'static str {
        "sh (setup.sh)"
    }
    fn supported(&self) -> bool {
        true
    }
    fn start(&self, opts: &EngineOpts) -> std::io::Result<Engine> {
        Engine::start(opts)
    }
}

pub const UNSUPPORTED_TEXT: &str = "Installing on this system comes in a later version of the launcher.";

/// A system with no install worker at all (not used by `backend()` any more; kept for tests and future targets).
pub struct UnsupportedBackend;

impl InstallBackend for UnsupportedBackend {
    fn name(&self) -> &'static str {
        "none"
    }
    fn supported(&self) -> bool {
        false
    }
    fn start(&self, _opts: &EngineOpts) -> std::io::Result<Engine> {
        Err(Error::new(ErrorKind::Unsupported, UNSUPPORTED_TEXT))
    }
}

pub fn backend() -> Box<dyn InstallBackend> {
    if cfg!(windows) {
        Box::new(PowerShellBackend)
    } else {
        Box::new(ShellBackend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_backend_says_so() {
        let o = EngineOpts { root: std::env::temp_dir(), iso: std::path::PathBuf::new(), ..Default::default() };
        let e = UnsupportedBackend.start(&o).err().unwrap();
        assert_eq!(e.kind(), ErrorKind::Unsupported);
        assert!(e.to_string().contains("later version"));
        assert!(!UnsupportedBackend.supported());
    }

    #[test]
    fn every_system_has_a_working_backend() {
        let b = backend();
        assert!(b.supported());
        assert_eq!(b.name(), if cfg!(windows) { "PowerShell (setup.ps1)" } else { "sh (setup.sh)" });
        // it starts through the engine: a folder without the script is the engine's NotFound, not "unsupported"
        let o = EngineOpts { root: std::env::temp_dir().join("racl_no_such_root_2"), iso: std::path::PathBuf::new(), ..Default::default() };
        assert_eq!(b.start(&o).err().unwrap().kind(), ErrorKind::NotFound);
    }
}
