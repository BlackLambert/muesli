//! Startup and runtime errors.

use std::error::Error as StdError;
use std::fmt;

/// An error a module reports while building or at runtime.
pub type BoxError = Box<dyn StdError + Send + Sync + 'static>;

/// Error from [`Module::build`](crate::Module::build).
pub type BuildError = BoxError;

/// The app could not be started.
#[derive(Debug)]
#[non_exhaustive]
pub enum StartError {
    /// A module failed to build.
    Build {
        /// The module.
        module: &'static str,
        /// The module's error.
        source: BuildError,
    },
    /// A contract is needed, but no module provides it.
    MissingProvider {
        /// The contract.
        contract: &'static str,
        /// Modules that need it.
        needed_by: Vec<&'static str>,
    },
    /// More than one module provides the same contract.
    DuplicateProvider {
        /// The contract.
        contract: &'static str,
        /// All modules that provide it.
        providers: Vec<&'static str>,
    },
    /// Modules depend on each other in a cycle.
    DependencyCycle {
        /// The modules involved.
        modules: Vec<&'static str>,
    },
    /// The stage constraints contradict each other.
    StageCycle {
        /// The stages involved.
        stages: Vec<&'static str>,
    },
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Build { module, source } => {
                write!(f, "module `{module}` failed to build: {source}")
            }
            Self::MissingProvider {
                contract,
                needed_by,
            } => write!(
                f,
                "no provider for contract `{contract}`, needed by {}",
                list(needed_by)
            ),
            Self::DuplicateProvider {
                contract,
                providers,
            } => write!(
                f,
                "contract `{contract}` has multiple providers: {}",
                list(providers)
            ),
            Self::DependencyCycle { modules } => {
                write!(f, "dependency cycle between {}", list(modules))
            }
            Self::StageCycle { stages } => {
                write!(f, "contradicting stage order at {}", list(stages))
            }
        }
    }
}

impl StdError for StartError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Build { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

/// An error while the chain is running.
#[derive(Debug)]
#[non_exhaustive]
pub enum RunError {
    /// A module aborted via [`StageCtx::fail`](crate::StageCtx::fail).
    Module {
        /// The module.
        module: &'static str,
        /// The stage it happened in.
        stage: &'static str,
        /// The module's error.
        source: BoxError,
    },
    /// The driver itself failed.
    Driver(BoxError),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Module {
                module,
                stage,
                source,
            } => write!(f, "module `{module}` failed in stage `{stage}`: {source}"),
            Self::Driver(source) => write!(f, "driver failed: {source}"),
        }
    }
}

impl StdError for RunError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Module { source, .. } | Self::Driver(source) => Some(source.as_ref()),
        }
    }
}

/// Startup or runtime error, as returned by [`App::run`](crate::App::run).
#[derive(Debug)]
pub enum Error {
    /// Error at startup.
    Start(StartError),
    /// Error at runtime.
    Run(RunError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start(e) => e.fmt(f),
            Self::Run(e) => e.fmt(f),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Start(e) => Some(e),
            Self::Run(e) => Some(e),
        }
    }
}

impl From<StartError> for Error {
    fn from(e: StartError) -> Self {
        Self::Start(e)
    }
}

impl From<RunError> for Error {
    fn from(e: RunError) -> Self {
        Self::Run(e)
    }
}

fn list(names: &[&str]) -> String {
    names
        .iter()
        .map(|n| format!("`{n}`"))
        .collect::<Vec<_>>()
        .join(", ")
}
