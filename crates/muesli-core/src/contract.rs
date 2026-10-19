//! Contracts, handles and the channels behind them.

use std::any::{Any, TypeId, type_name};
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::runtime::StageCtx;

/// A contract describes what an area offers: which commands it accepts and
/// which snapshot it publishes. It contains no logic.
///
/// ```
/// use muesli_core::Contract;
///
/// pub struct Counter; // marker
///
/// impl Contract for Counter {
///     type Command = u32;
///     type Snapshot = u32;
/// }
/// ```
pub trait Contract: 'static {
    /// What users of the contract send to the provider.
    type Command: Send + 'static;
    /// What the provider publishes after its stage.
    type Snapshot: Default + Send + Sync + 'static;
}

/// Runtime id of a contract, with its name for error messages.
#[derive(Clone, Copy)]
pub struct ContractId {
    type_id: TypeId,
    name: &'static str,
}

impl ContractId {
    /// The id of contract `C`.
    pub fn of<C: Contract>() -> Self {
        Self {
            type_id: TypeId::of::<C>(),
            name: type_name::<C>(),
        }
    }

    /// Full type name of the contract.
    pub fn name(&self) -> &'static str {
        self.name
    }
}

impl PartialEq for ContractId {
    fn eq(&self, other: &Self) -> bool {
        self.type_id == other.type_id
    }
}

impl Eq for ContractId {}

impl Hash for ContractId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.type_id.hash(state);
    }
}

impl fmt::Debug for ContractId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name)
    }
}

/// Queue and snapshot of a contract. There is exactly one channel per contract and app.
pub(crate) struct Channel<C: Contract> {
    queue: Mutex<Vec<C::Command>>,
    snapshot: RwLock<C::Snapshot>,
    next_id: AtomicU64,
}

impl<C: Contract> Channel<C> {
    fn new() -> Self {
        Self {
            queue: Mutex::new(Vec::new()),
            snapshot: RwLock::new(C::Snapshot::default()),
            next_id: AtomicU64::new(0),
        }
    }

    /// Swaps the queue with `scratch`. Afterwards all pending commands are in
    /// `scratch`, and the queue takes over its (empty) allocation.
    pub(crate) fn swap_queue(&self, scratch: &mut Vec<C::Command>) {
        debug_assert!(scratch.is_empty());
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::swap(&mut *queue, scratch);
    }

    pub(crate) fn write_snapshot(&self) -> RwLockWriteGuard<'_, C::Snapshot> {
        self.snapshot
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Public facade of a contract.
///
/// Writing calls are queued as commands and only executed in the provider's
/// stage. Reading calls see the most recently published snapshot and are only
/// possible inside a stage call. A handle is cheap to clone and `Send`.
pub struct Handle<C: Contract> {
    channel: Arc<Channel<C>>,
}

impl<C: Contract> Handle<C> {
    pub(crate) fn new(channel: Arc<Channel<C>>) -> Self {
        Self { channel }
    }

    /// Queues a command. The provider processes it at the start of its next stage.
    pub fn push(&self, command: C::Command) {
        self.channel
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(command);
    }

    /// The most recently published snapshot.
    ///
    /// Reading needs the [`StageCtx`] of the running work, and the returned
    /// reference cannot outlive it. A snapshot reference therefore always ends
    /// with the stage call that took it and can never block the next publish:
    ///
    /// ```compile_fail
    /// use muesli_core::{Contract, Handle, SnapshotRef, StageCtx};
    ///
    /// struct Sum;
    /// impl Contract for Sum {
    ///     type Command = u32;
    ///     type Snapshot = u32;
    /// }
    ///
    /// // Error: the reference may not outlive `ctx`.
    /// fn keep<'h>(sum: &'h Handle<Sum>, ctx: &StageCtx<'_>) -> SnapshotRef<'h, Sum> {
    ///     sum.snapshot(ctx)
    /// }
    /// ```
    pub fn snapshot<'a>(&'a self, _ctx: &'a StageCtx<'_>) -> SnapshotRef<'a, C> {
        SnapshotRef {
            guard: self
                .channel
                .snapshot
                .read()
                .unwrap_or_else(PoisonError::into_inner),
        }
    }

    /// Immediately allocates a new id that is unique per contract, e.g. for a
    /// texture that is only created on the next drain.
    pub fn alloc_id(&self) -> u64 {
        self.channel.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

impl<C: Contract> Clone for Handle<C> {
    fn clone(&self) -> Self {
        Self {
            channel: Arc::clone(&self.channel),
        }
    }
}

impl<C: Contract> fmt::Debug for Handle<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Handle<{}>", type_name::<C>())
    }
}

/// Read access to a snapshot, valid for the stage call it was taken in.
pub struct SnapshotRef<'a, C: Contract> {
    guard: RwLockReadGuard<'a, C::Snapshot>,
}

impl<C: Contract> Deref for SnapshotRef<'_, C> {
    type Target = C::Snapshot;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

/// The commands a provider works through at the start of its stage.
/// Commands that are not consumed are dropped.
pub struct Drain<'a, T> {
    pub(crate) inner: std::vec::Drain<'a, T>,
}

impl<T> Iterator for Drain<'_, T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<T> ExactSizeIterator for Drain<'_, T> {}

/// All channels of an app, keyed by contract.
#[derive(Default)]
pub(crate) struct Registry {
    channels: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Registry {
    /// Returns the channel of `C`, creating it on first access.
    pub(crate) fn channel<C: Contract>(&mut self) -> Arc<Channel<C>> {
        self.channels
            .entry(TypeId::of::<C>())
            .or_insert_with(|| Box::new(Arc::new(Channel::<C>::new())))
            .downcast_ref::<Arc<Channel<C>>>()
            .map(Arc::clone)
            .expect("channel is stored under the TypeId of its contract")
    }
}
