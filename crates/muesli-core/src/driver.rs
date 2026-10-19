//! Drivers that run the chain.

use std::thread;
use std::time::{Duration, Instant};

use crate::error::RunError;
use crate::runtime::{Flow, Runtime};

/// Decides when and how often the chain runs. Exactly one per app.
///
/// The core does not own the loop, because window backends like winit own the
/// main-thread loop. A driver therefore calls [`Runtime::run_chain`] from the
/// outside.
pub trait Driver: 'static {
    /// Drives the runtime until a module requests exit or an error occurs.
    fn drive(self: Box<Self>, runtime: Runtime) -> Result<(), RunError>;
}

/// Runs the chain exactly once. For CLI tools and algorithm tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct Once;

impl Driver for Once {
    fn drive(self: Box<Self>, mut runtime: Runtime) -> Result<(), RunError> {
        runtime.run_chain().map(|_| ())
    }
}

/// Runs the chain as often as possible, without pausing, until a module
/// requests exit. For benchmarks and batch simulations that only need the
/// result.
///
/// Keeps one CPU core busy. Use [`FixedRate`] for a steady pace instead.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unbounded;

impl Driver for Unbounded {
    fn drive(self: Box<Self>, mut runtime: Runtime) -> Result<(), RunError> {
        while runtime.run_chain()? == Flow::Continue {}
        Ok(())
    }
}

/// Runs the chain at a fixed rate until a module requests exit.
/// For simulations without a window.
///
/// If the chain falls behind, the driver does not catch up but restarts the
/// schedule from now.
#[derive(Debug, Clone, Copy)]
pub struct FixedRate(pub f64);

impl Driver for FixedRate {
    fn drive(self: Box<Self>, mut runtime: Runtime) -> Result<(), RunError> {
        let FixedRate(hz) = *self;
        if !(hz.is_finite() && hz > 0.0) {
            return Err(RunError::Driver(
                format!("FixedRate needs a positive frequency, not {hz}").into(),
            ));
        }
        let period = Duration::from_secs_f64(1.0 / hz);
        let mut next = Instant::now();
        loop {
            if runtime.run_chain()? == Flow::Exit {
                return Ok(());
            }
            next += period;
            let now = Instant::now();
            if next > now {
                thread::sleep(next - now);
            } else {
                next = now;
            }
        }
    }
}
