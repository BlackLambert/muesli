//! App assembly, runtime and the context passed at runtime.

use std::any::{Any, TypeId, type_name};
use std::cell::Cell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::time::{Duration, Instant};

use crate::contract::ContractId;
use crate::driver::{Driver, Once};
use crate::error::{BoxError, Error, RunError, StartError};
use crate::graph::topo_sort;
use crate::module::{BuildCtx, DepCtx, DepSet, Module, Plan, Task};
use crate::stage::{Stage, StageId, Update};

type AddFn = Box<dyn FnOnce(&mut Plan, usize) -> Result<Built, StartError>>;

struct Built {
    name: &'static str,
    state: Box<dyn Any + Send>,
    needs: Vec<ContractId>,
}

/// Composition root: the only place that chooses implementations.
///
/// ```
/// # use muesli_core::{App, BuildCtx, BuildError, Module, Update};
/// # struct Hello;
/// # impl Module for Hello {
/// #     type Deps = ();
/// #     type State = ();
/// #     fn build(self, _: (), ctx: &mut BuildCtx<'_, ()>) -> Result<(), BuildError> {
/// #         ctx.run(Update, |_, _| {});
/// #         Ok(())
/// #     }
/// # }
/// App::new().add(Hello).run().unwrap();
/// ```
#[must_use]
pub struct App {
    modules: Vec<AddFn>,
    driver: Option<Box<dyn Driver>>,
}

impl App {
    /// An empty app.
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
            driver: None,
        }
    }

    /// Adds a module. Modules are built in this order.
    // `add` is the name from the concept; a `std::ops::Add` makes no sense here.
    #[allow(clippy::should_implement_trait)]
    pub fn add<M: Module>(mut self, module: M) -> Self {
        self.modules.push(Box::new(move |plan, index| {
            let name = type_name::<M>();
            let mut needs = Vec::new();
            M::Deps::contracts(&mut needs);
            let deps = M::Deps::fetch(&mut DepCtx {
                registry: &mut plan.registry,
            });
            let mut ctx = BuildCtx {
                plan,
                module: index,
                name,
                _state: PhantomData,
            };
            let state = module
                .build(deps, &mut ctx)
                .map_err(|source| StartError::Build {
                    module: name,
                    source,
                })?;
            Ok(Built {
                name,
                state: Box::new(state),
                needs,
            })
        }));
        self
    }

    /// Sets the driver. Without one the chain runs exactly once ([`Once`]).
    pub fn driver(mut self, driver: impl Driver) -> Self {
        self.driver = Some(Box::new(driver));
        self
    }

    /// Builds all modules, checks providers and dependencies and sorts the chain.
    /// The driver is ignored.
    pub fn build(self) -> Result<Runtime, StartError> {
        let mut plan = Plan::default();
        plan.stages.register(StageId::of::<Update>());

        let mut slots = Vec::with_capacity(self.modules.len());
        let mut needs = Vec::with_capacity(self.modules.len());
        for (index, add) in self.modules.into_iter().enumerate() {
            let built = add(&mut plan, index)?;
            slots.push(Slot {
                name: built.name,
                state: Some(built.state),
            });
            needs.push(built.needs);
        }

        let providers = providers_by_contract(&plan, &slots)?;
        let module_order = module_order(&slots, &needs, &providers)?;
        let stage_order = plan.stages.sort()?;

        let mut topo_pos = vec![0; slots.len()];
        for (pos, &m) in module_order.iter().enumerate() {
            topo_pos[m] = pos;
        }
        let mut by_stage: Vec<Vec<Task>> =
            (0..plan.stages.nodes.len()).map(|_| Vec::new()).collect();
        for task in plan.tasks {
            by_stage[task.stage].push(task);
        }
        let stages = stage_order
            .into_iter()
            .map(|s| {
                let mut tasks = std::mem::take(&mut by_stage[s]);
                tasks.sort_by_key(|t| (t.phase, topo_pos[t.module]));
                StageRun {
                    id: plan.stages.nodes[s],
                    tasks,
                }
            })
            .collect();

        Ok(Runtime {
            slots,
            stages,
            module_order,
            frame: 0,
            last_chain: None,
        })
    }

    /// Builds the app and hands it to the driver.
    pub fn run(mut self) -> Result<(), Error> {
        let driver = self.driver.take().unwrap_or_else(|| Box::new(Once));
        let runtime = self.build()?;
        driver.drive(runtime)?;
        Ok(())
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

/// Groups providers by contract and rejects duplicates.
fn providers_by_contract(
    plan: &Plan,
    slots: &[Slot],
) -> Result<HashMap<ContractId, usize>, StartError> {
    let mut grouped: Vec<(ContractId, Vec<usize>)> = Vec::new();
    for &(contract, module) in &plan.provides {
        match grouped.iter_mut().find(|(c, _)| *c == contract) {
            Some((_, modules)) => modules.push(module),
            None => grouped.push((contract, vec![module])),
        }
    }
    let mut providers = HashMap::new();
    for (contract, modules) in grouped {
        if modules.len() > 1 {
            return Err(StartError::DuplicateProvider {
                contract: contract.name(),
                providers: modules.iter().map(|&m| slots[m].name).collect(),
            });
        }
        providers.insert(contract, modules[0]);
    }
    Ok(providers)
}

/// Topological order of modules: providers before their users.
fn module_order(
    slots: &[Slot],
    needs: &[Vec<ContractId>],
    providers: &HashMap<ContractId, usize>,
) -> Result<Vec<usize>, StartError> {
    let n = slots.len();
    let mut missing: Vec<(ContractId, Vec<&'static str>)> = Vec::new();
    let mut next = vec![Vec::new(); n];
    for (user, contracts) in needs.iter().enumerate() {
        for contract in contracts {
            match providers.get(contract) {
                Some(&provider) if provider == user => {}
                Some(&provider) => next[provider].push(user),
                None => match missing.iter_mut().find(|(c, _)| c == contract) {
                    Some((_, users)) => users.push(slots[user].name),
                    None => missing.push((*contract, vec![slots[user].name])),
                },
            }
        }
    }
    if let Some((contract, needed_by)) = missing.into_iter().next() {
        return Err(StartError::MissingProvider {
            contract: contract.name(),
            needed_by,
        });
    }

    topo_sort(&next).map_err(|stuck| StartError::DependencyCycle {
        modules: stuck.into_iter().map(|i| slots[i].name).collect(),
    })
}

struct Slot {
    name: &'static str,
    state: Option<Box<dyn Any + Send>>,
}

struct StageRun {
    id: StageId,
    tasks: Vec<Task>,
}

/// Whether the chain should keep running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// Keep going.
    Continue,
    /// A module requested exit.
    Exit,
}

/// A fully built app. It owns no loop; a [`Driver`] drives it through
/// [`Runtime::run_chain`].
///
/// On drop, modules are torn down in reverse dependency order: users before
/// their providers.
pub struct Runtime {
    slots: Vec<Slot>,
    stages: Vec<StageRun>,
    module_order: Vec<usize>,
    frame: u64,
    last_chain: Option<Instant>,
}

impl Runtime {
    /// Runs all stages once, in order.
    pub fn run_chain(&mut self) -> Result<Flow, RunError> {
        let now = Instant::now();
        let delta = self.last_chain.map_or(Duration::ZERO, |last| now - last);
        self.last_chain = Some(now);

        let exit = Cell::new(false);
        for stage in 0..self.stages.len() {
            self.run_stage_at(stage, delta, &exit)?;
        }
        self.frame += 1;
        Ok(if exit.get() {
            Flow::Exit
        } else {
            Flow::Continue
        })
    }

    /// Runs only stage `S`, with the same queue and snapshot rules.
    /// Does not count as a run: `frame` stays, `delta` is zero. If `S` is not
    /// part of the chain, nothing happens.
    pub fn run_stage<S: Stage>(&mut self) -> Result<Flow, RunError> {
        let exit = Cell::new(false);
        if let Some(stage) = self
            .stages
            .iter()
            .position(|s| s.id.label_type() == TypeId::of::<S>())
        {
            self.run_stage_at(stage, Duration::ZERO, &exit)?;
        }
        Ok(if exit.get() {
            Flow::Exit
        } else {
            Flow::Continue
        })
    }

    /// Stage names in execution order.
    pub fn stage_names(&self) -> Vec<&'static str> {
        self.stages.iter().map(|s| s.id.name).collect()
    }

    /// Module names in execution order within a stage.
    pub fn module_names(&self) -> Vec<&'static str> {
        self.module_order
            .iter()
            .map(|&m| self.slots[m].name)
            .collect()
    }

    /// Number of completed runs.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    fn run_stage_at(
        &mut self,
        stage: usize,
        delta: Duration,
        exit: &Cell<bool>,
    ) -> Result<(), RunError> {
        let run = &mut self.stages[stage];
        for task in &mut run.tasks {
            let slot = &mut self.slots[task.module];
            let failure = Cell::new(None);
            let mut ctx = StageCtx {
                module: slot.name,
                stage: run.id.name,
                frame: self.frame,
                delta,
                exit,
                failure: &failure,
            };
            let state = slot
                .state
                .as_deref_mut()
                .expect("state lives as long as the runtime");
            (task.f)(state, &mut ctx);
            if let Some(source) = failure.take() {
                return Err(RunError::Module {
                    module: slot.name,
                    stage: run.id.name,
                    source,
                });
            }
        }
        Ok(())
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        for &m in self.module_order.iter().rev() {
            self.slots[m].state = None;
        }
    }
}

/// Context for work running in a stage.
pub struct StageCtx<'a> {
    module: &'static str,
    stage: &'static str,
    frame: u64,
    delta: Duration,
    exit: &'a Cell<bool>,
    failure: &'a Cell<Option<BoxError>>,
}

impl StageCtx<'_> {
    /// The module whose work is running.
    pub fn module_name(&self) -> &'static str {
        self.module
    }

    /// The running stage.
    pub fn stage_name(&self) -> &'static str {
        self.stage
    }

    /// Number of the current run, starting at 0.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Time since the start of the previous run; zero in the first run.
    pub fn delta(&self) -> Duration {
        self.delta
    }

    /// Requests exit. The current run completes, then the driver stops.
    pub fn request_exit(&self) {
        self.exit.set(true);
    }

    /// Aborts the chain with an error. The remaining work of this stage and all
    /// later stages no longer run.
    pub fn fail(&self, error: impl Into<BoxError>) {
        self.failure.set(Some(error.into()));
    }
}
