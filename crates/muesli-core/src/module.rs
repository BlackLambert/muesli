//! Modules, providers and everything needed while building them.

use std::any::Any;

use crate::contract::{Contract, ContractId, Drain, Handle, Registry};
use crate::error::BuildError;
use crate::runtime::StageCtx;
use crate::stage::{Stage, StageGraph, StageId, StageOrder};

/// A module is what you pass to [`App::add`](crate::App::add): its
/// configuration. At startup the core builds the private [`State`](Module::State)
/// from it, which is owned by the scheduler.
///
/// ```
/// use muesli_core::{BuildCtx, BuildError, Module, Update};
///
/// struct Hello;
///
/// impl Module for Hello {
///     type Deps = ();
///     type State = ();
///
///     fn build(self, _: (), ctx: &mut BuildCtx<'_, ()>) -> Result<(), BuildError> {
///         ctx.run(Update, |_, _| println!("Hello"));
///         Ok(())
///     }
/// }
/// ```
pub trait Module: Send + 'static {
    /// The contracts the module needs, e.g. `(Handle<Window>, Handle<Draw2d>)`.
    type Deps: DepSet;
    /// The private state. It only gets `&mut` in the stages the module
    /// registers work for.
    type State: Send + 'static;

    /// Builds the state, announces provided contracts with
    /// [`BuildCtx::provide`] and registers work with [`BuildCtx::run`].
    fn build(
        self,
        deps: Self::Deps,
        ctx: &mut BuildCtx<'_, Self::State>,
    ) -> Result<Self::State, BuildError>;
}

/// A state that serves a contract.
pub trait Provider<C: Contract>: Send + 'static {
    /// Processes all commands queued so far, at the start of its own stage.
    fn process(&mut self, commands: Drain<'_, C::Command>, ctx: &mut StageCtx<'_>);

    /// Publishes the new snapshot after its own stage.
    fn publish(&self, snapshot: &mut C::Snapshot);
}

/// A set of handles a module needs: `()`, a single `Handle<C>` or a tuple of
/// up to eight handles.
pub trait DepSet: Sized + Send + 'static {
    /// The contracts in this set.
    fn contracts(out: &mut Vec<ContractId>);

    /// Fetches the handles.
    fn fetch(ctx: &mut DepCtx<'_>) -> Self;
}

/// Access to the channels while dependencies are resolved.
pub struct DepCtx<'a> {
    pub(crate) registry: &'a mut Registry,
}

impl DepCtx<'_> {
    /// A handle to contract `C`.
    pub fn handle<C: Contract>(&mut self) -> Handle<C> {
        Handle::new(self.registry.channel::<C>())
    }
}

impl DepSet for () {
    fn contracts(_: &mut Vec<ContractId>) {}
    fn fetch(_: &mut DepCtx<'_>) -> Self {}
}

impl<C: Contract> DepSet for Handle<C> {
    fn contracts(out: &mut Vec<ContractId>) {
        out.push(ContractId::of::<C>());
    }

    fn fetch(ctx: &mut DepCtx<'_>) -> Self {
        ctx.handle::<C>()
    }
}

macro_rules! dep_tuple {
    ($($t:ident),+) => {
        impl<$($t: DepSet),+> DepSet for ($($t,)+) {
            fn contracts(out: &mut Vec<ContractId>) {
                $($t::contracts(out);)+
            }

            fn fetch(ctx: &mut DepCtx<'_>) -> Self {
                ($($t::fetch(ctx),)+)
            }
        }
    };
}

dep_tuple!(A);
dep_tuple!(A, B);
dep_tuple!(A, B, C);
dep_tuple!(A, B, C, D);
dep_tuple!(A, B, C, D, E);
dep_tuple!(A, B, C, D, E, F);
dep_tuple!(A, B, C, D, E, F, G);
dep_tuple!(A, B, C, D, E, F, G, H);

pub(crate) type TaskFn = Box<dyn FnMut(&mut (dyn Any + Send), &mut StageCtx<'_>) + Send>;

/// The phase of a stage a task runs in.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Phase {
    /// Providers drain their queue.
    Process,
    /// Work registered by modules.
    Run,
    /// Providers publish their snapshot.
    Publish,
}

pub(crate) struct Task {
    pub(crate) module: usize,
    pub(crate) stage: usize,
    pub(crate) phase: Phase,
    pub(crate) f: TaskFn,
}

/// What is collected while building all modules.
#[derive(Default)]
pub(crate) struct Plan {
    pub(crate) registry: Registry,
    pub(crate) stages: StageGraph,
    pub(crate) tasks: Vec<Task>,
    /// `(contract, module index)`
    pub(crate) provides: Vec<(ContractId, usize)>,
}

/// Context while building a module with state `S`.
pub struct BuildCtx<'a, S> {
    pub(crate) plan: &'a mut Plan,
    pub(crate) module: usize,
    pub(crate) name: &'static str,
    pub(crate) _state: std::marker::PhantomData<fn(&mut S)>,
}

impl<S: Send + 'static> BuildCtx<'_, S> {
    /// Name of the module being built.
    pub fn module_name(&self) -> &'static str {
        self.name
    }

    /// Announces the state as provider of `C`. It drains the queue at the start
    /// of `stage` and publishes its snapshot afterwards.
    pub fn provide<C: Contract>(&mut self, stage: impl Stage) -> &mut Self
    where
        S: Provider<C>,
    {
        let stage = self.register_stage(stage);
        let channel = self.plan.registry.channel::<C>();
        self.plan
            .provides
            .push((ContractId::of::<C>(), self.module));

        let queue = std::sync::Arc::clone(&channel);
        let mut scratch: Vec<C::Command> = Vec::new();
        self.push_task(
            stage,
            Phase::Process,
            Box::new(move |state, ctx| {
                queue.swap_queue(&mut scratch);
                let commands = Drain {
                    inner: scratch.drain(..),
                };
                Provider::<C>::process(downcast::<S>(state), commands, ctx);
            }),
        );
        self.push_task(
            stage,
            Phase::Publish,
            Box::new(move |state, _| {
                let state: &S = downcast::<S>(state);
                Provider::<C>::publish(state, &mut channel.write_snapshot());
            }),
        );
        self
    }

    /// Registers work in `stage`.
    pub fn run<F>(&mut self, stage: impl Stage, mut f: F) -> &mut Self
    where
        F: FnMut(&mut S, &mut StageCtx<'_>) + Send + 'static,
    {
        let stage = self.register_stage(stage);
        self.push_task(
            stage,
            Phase::Run,
            Box::new(move |state, ctx| f(downcast::<S>(state), ctx)),
        );
        self
    }

    /// Adds constraints for a stage, e.g.
    /// `ctx.stage(Physics).after(Update).before(Render)`.
    pub fn stage<T: Stage>(&mut self, _stage: T) -> StageOrder<'_> {
        self.plan.stages.order_of(StageId::of::<T>())
    }

    fn register_stage<T: Stage>(&mut self, _stage: T) -> usize {
        self.plan.stages.register(StageId::of::<T>())
    }

    fn push_task(&mut self, stage: usize, phase: Phase, f: TaskFn) {
        self.plan.tasks.push(Task {
            module: self.module,
            stage,
            phase,
            f,
        });
    }
}

fn downcast<S: 'static>(state: &mut (dyn Any + Send)) -> &mut S {
    state
        .downcast_mut::<S>()
        .expect("task belongs to the state of its module")
}
