//! Tests for the execution chain rules.

use std::sync::{Arc, Mutex};

use muesli_core::{
    App, BuildCtx, BuildError, Contract, Drain, FixedRate, Flow, Handle, Module, Provider,
    RunError, Stage, StageCtx, StageOrder, StartError, Unbounded, Update,
};

type Log = Arc<Mutex<Vec<String>>>;

fn log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

fn entries(log: &Log) -> Vec<String> {
    log.lock().unwrap().clone()
}

// --- Contract and provider -------------------------------------------------

/// Adds up commands and publishes the sum.
struct Sum;

impl Contract for Sum {
    type Command = u32;
    type Snapshot = u32;
}

/// Runs after `Update`, brought along by the contract.
struct Late;

impl Stage for Late {
    fn configure(order: &mut StageOrder<'_>) {
        order.after(Update);
    }
}

/// Runs after `Late`.
struct AfterLate;

impl Stage for AfterLate {
    fn configure(order: &mut StageOrder<'_>) {
        order.after(Late);
    }
}

struct SumProvider {
    log: Option<Log>,
}

struct SumState {
    total: u32,
    log: Option<Log>,
}

impl Module for SumProvider {
    type Deps = ();
    type State = SumState;

    fn build(self, _: (), ctx: &mut BuildCtx<'_, SumState>) -> Result<SumState, BuildError> {
        ctx.provide::<Sum>(Late);
        Ok(SumState {
            total: 0,
            log: self.log,
        })
    }
}

impl Provider<Sum> for SumState {
    fn process(&mut self, commands: Drain<'_, u32>, _: &mut StageCtx<'_>) {
        self.total += commands.sum::<u32>();
    }

    fn publish(&self, snapshot: &mut u32) {
        *snapshot = self.total;
    }
}

impl Drop for SumState {
    fn drop(&mut self) {
        if let Some(log) = &self.log {
            log.lock().unwrap().push("drop provider".into());
        }
    }
}

/// User module: writes in `Update` and `AfterLate`, reads in both.
struct SumUser {
    log: Log,
}

struct SumUserState {
    sum: Handle<Sum>,
    log: Log,
}

impl Module for SumUser {
    type Deps = Handle<Sum>;
    type State = SumUserState;

    fn build(
        self,
        sum: Handle<Sum>,
        ctx: &mut BuildCtx<'_, SumUserState>,
    ) -> Result<SumUserState, BuildError> {
        ctx.run(Update, |s: &mut SumUserState, ctx| {
            let seen = *s.sum.snapshot(ctx);
            s.log
                .lock()
                .unwrap()
                .push(format!("{} update sees {seen}", ctx.frame()));
            s.sum.push(1);
        });
        ctx.run(AfterLate, |s: &mut SumUserState, ctx| {
            let seen = *s.sum.snapshot(ctx);
            s.log
                .lock()
                .unwrap()
                .push(format!("{} after sees {seen}", ctx.frame()));
            s.sum.push(10);
        });
        Ok(SumUserState { sum, log: self.log })
    }
}

impl Drop for SumUserState {
    fn drop(&mut self) {
        self.log.lock().unwrap().push("drop user".into());
    }
}

// --- Queue and snapshot rules ----------------------------------------------

#[test]
fn queue_and_snapshot_rules() {
    let log = log();
    let mut rt = App::new()
        .add(SumUser { log: log.clone() })
        .add(SumProvider { log: None })
        .build()
        .unwrap();

    rt.run_chain().unwrap();
    rt.run_chain().unwrap();

    assert_eq!(
        entries(&log),
        [
            // Before the provider's stage: state of the previous run.
            "0 update sees 0",
            // After the stage: new state, the 1 from Update is already included.
            "0 after sees 1",
            // The 10 arrived after the drain and only takes effect now.
            "1 update sees 1",
            "1 after sees 12",
        ]
    );
}

#[test]
fn providers_run_before_their_users_regardless_of_add_order() {
    let rt = App::new()
        .add(SumUser { log: log() })
        .add(SumProvider { log: None })
        .build()
        .unwrap();
    let names = rt.module_names();
    assert!(names[0].ends_with("SumProvider"), "{names:?}");
    assert!(names[1].ends_with("SumUser"), "{names:?}");
}

#[test]
fn modules_drop_in_reverse_dependency_order() {
    let log = log();
    let rt = App::new()
        .add(SumProvider {
            log: Some(log.clone()),
        })
        .add(SumUser { log: log.clone() })
        .build()
        .unwrap();
    drop(rt);
    assert_eq!(entries(&log), ["drop user", "drop provider"]);
}

#[test]
fn handles_allocate_unique_ids_immediately() {
    struct Grab(Arc<Mutex<Option<Handle<Sum>>>>);
    impl Module for Grab {
        type Deps = Handle<Sum>;
        type State = ();
        fn build(self, h: Handle<Sum>, _: &mut BuildCtx<'_, ()>) -> Result<(), BuildError> {
            *self.0.lock().unwrap() = Some(h);
            Ok(())
        }
    }

    let slot = Arc::new(Mutex::new(None));
    let _rt = App::new()
        .add(SumProvider { log: None })
        .add(Grab(slot.clone()))
        .build()
        .unwrap();
    let handle = slot.lock().unwrap().clone().unwrap();
    let other = handle.clone();
    assert_eq!(handle.alloc_id(), 0);
    assert_eq!(other.alloc_id(), 1);

    fn assert_send<T: Send + Clone>() {}
    assert_send::<Handle<Sum>>();
}

#[test]
fn snapshot_can_be_held_while_using_the_context() {
    struct Reader;
    impl Module for Reader {
        type Deps = Handle<Sum>;
        type State = Handle<Sum>;
        fn build(
            self,
            sum: Handle<Sum>,
            ctx: &mut BuildCtx<'_, Handle<Sum>>,
        ) -> Result<Handle<Sum>, BuildError> {
            ctx.run(Update, |sum: &mut Handle<Sum>, ctx| {
                let seen = sum.snapshot(ctx);
                if *seen == 0 {
                    ctx.request_exit();
                }
            });
            Ok(sum)
        }
    }

    let mut rt = App::new()
        .add(Reader)
        .add(SumProvider { log: None })
        .build()
        .unwrap();
    assert_eq!(rt.run_chain().unwrap(), Flow::Exit);
}

// --- Stage ordering --------------------------------------------------------

struct Input;
impl Stage for Input {
    fn configure(order: &mut StageOrder<'_>) {
        order.before(Update);
    }
}

struct Render;
impl Stage for Render {
    fn configure(order: &mut StageOrder<'_>) {
        order.after(Update);
    }
}

struct Physics;
impl Stage for Physics {}

/// Registers work in arbitrary stages and places `Physics` via constraints.
struct Staged;

impl Module for Staged {
    type Deps = ();
    type State = ();

    fn build(self, _: (), ctx: &mut BuildCtx<'_, ()>) -> Result<(), BuildError> {
        ctx.run(Render, |_, _| {});
        ctx.stage(Physics).after(Update).before(Render);
        ctx.run(Physics, |_, _| {});
        ctx.run(Input, |_, _| {});
        Ok(())
    }
}

fn short(names: Vec<&'static str>) -> Vec<&'static str> {
    names
        .into_iter()
        .map(|n| n.rsplit("::").next().unwrap())
        .collect()
}

#[test]
fn stages_are_sorted_by_constraints() {
    let rt = App::new().add(Staged).build().unwrap();
    assert_eq!(
        short(rt.stage_names()),
        ["Input", "Update", "Physics", "Render"]
    );
}

#[test]
fn headless_app_has_only_update() {
    struct Tool;
    impl Module for Tool {
        type Deps = ();
        type State = ();
        fn build(self, _: (), ctx: &mut BuildCtx<'_, ()>) -> Result<(), BuildError> {
            ctx.run(Update, |_, _| {});
            Ok(())
        }
    }
    let rt = App::new().add(Tool).build().unwrap();
    assert_eq!(short(rt.stage_names()), ["Update"]);
}

#[test]
fn contradicting_stage_constraints_are_a_start_error() {
    struct Contradiction;
    impl Module for Contradiction {
        type Deps = ();
        type State = ();
        fn build(self, _: (), ctx: &mut BuildCtx<'_, ()>) -> Result<(), BuildError> {
            ctx.stage(Render).before(Update);
            Ok(())
        }
    }
    let err = App::new().add(Contradiction).build().err().unwrap();
    assert!(matches!(err, StartError::StageCycle { .. }), "{err}");
}

#[test]
fn run_stage_runs_a_single_stage() {
    let log = log();
    let mut rt = App::new()
        .add(SumUser { log: log.clone() })
        .add(SumProvider { log: None })
        .build()
        .unwrap();
    rt.run_stage::<AfterLate>().unwrap();
    rt.run_stage::<Physics>().unwrap(); // not part of the chain: nothing happens
    assert_eq!(entries(&log), ["0 after sees 0"]);
    assert_eq!(rt.frame(), 0);
}

// --- Provider validation ---------------------------------------------------

#[test]
fn missing_provider_names_contract_and_module() {
    let err = App::new()
        .add(SumUser { log: log() })
        .build()
        .err()
        .unwrap();
    match &err {
        StartError::MissingProvider {
            contract,
            needed_by,
        } => {
            assert!(contract.ends_with("Sum"));
            assert!(needed_by[0].ends_with("SumUser"));
        }
        other => panic!("wrong error: {other}"),
    }
    let message = err.to_string();
    assert!(
        message.contains("Sum") && message.contains("SumUser"),
        "{message}"
    );
}

#[test]
fn duplicate_provider_is_a_start_error() {
    let err = App::new()
        .add(SumProvider { log: None })
        .add(SumProvider { log: None })
        .build()
        .err()
        .unwrap();
    assert!(
        matches!(&err, StartError::DuplicateProvider { providers, .. } if providers.len() == 2),
        "{err}"
    );
}

#[test]
fn dependency_cycle_is_a_start_error() {
    struct Ping;
    impl Contract for Ping {
        type Command = ();
        type Snapshot = ();
    }
    struct Pong;
    impl Contract for Pong {
        type Command = ();
        type Snapshot = ();
    }

    struct Both;
    impl Provider<Ping> for Both {
        fn process(&mut self, _: Drain<'_, ()>, _: &mut StageCtx<'_>) {}
        fn publish(&self, _: &mut ()) {}
    }
    impl Provider<Pong> for Both {
        fn process(&mut self, _: Drain<'_, ()>, _: &mut StageCtx<'_>) {}
        fn publish(&self, _: &mut ()) {}
    }

    struct A;
    impl Module for A {
        type Deps = Handle<Pong>;
        type State = Both;
        fn build(self, _: Handle<Pong>, ctx: &mut BuildCtx<'_, Both>) -> Result<Both, BuildError> {
            ctx.provide::<Ping>(Update);
            Ok(Both)
        }
    }
    struct B;
    impl Module for B {
        type Deps = Handle<Ping>;
        type State = Both;
        fn build(self, _: Handle<Ping>, ctx: &mut BuildCtx<'_, Both>) -> Result<Both, BuildError> {
            ctx.provide::<Pong>(Update);
            Ok(Both)
        }
    }

    let err = App::new().add(A).add(B).build().err().unwrap();
    assert!(
        matches!(&err, StartError::DependencyCycle { modules } if modules.len() == 2),
        "{err}"
    );
}

#[test]
fn a_module_may_use_the_contract_it_provides() {
    struct SelfUser;
    impl Module for SelfUser {
        type Deps = Handle<Sum>;
        type State = SumState;
        fn build(
            self,
            _: Handle<Sum>,
            ctx: &mut BuildCtx<'_, SumState>,
        ) -> Result<SumState, BuildError> {
            ctx.provide::<Sum>(Update);
            Ok(SumState {
                total: 0,
                log: None,
            })
        }
    }
    App::new().add(SelfUser).build().unwrap();
}

#[test]
fn build_errors_name_the_module() {
    struct Broken;
    impl Module for Broken {
        type Deps = ();
        type State = ();
        fn build(self, _: (), _: &mut BuildCtx<'_, ()>) -> Result<(), BuildError> {
            Err("no GPU".into())
        }
    }
    let err = App::new().add(Broken).build().err().unwrap();
    assert!(matches!(err, StartError::Build { .. }));
    assert!(err.to_string().contains("Broken"), "{err}");
}

// --- Lifecycle and drivers -------------------------------------------------

/// Counts runs and requests exit after `limit`.
struct Counter {
    limit: u64,
    seen: Arc<Mutex<u64>>,
}

impl Module for Counter {
    type Deps = ();
    type State = Counter;

    fn build(self, _: (), ctx: &mut BuildCtx<'_, Counter>) -> Result<Counter, BuildError> {
        ctx.run(Update, |c: &mut Counter, ctx| {
            *c.seen.lock().unwrap() += 1;
            if ctx.frame() + 1 == c.limit {
                ctx.request_exit();
            }
        });
        Ok(self)
    }
}

#[test]
fn run_chain_reports_exit() {
    let seen = Arc::new(Mutex::new(0));
    let mut rt = App::new()
        .add(Counter {
            limit: 2,
            seen: seen.clone(),
        })
        .build()
        .unwrap();
    assert_eq!(rt.run_chain().unwrap(), Flow::Continue);
    assert_eq!(rt.run_chain().unwrap(), Flow::Exit);
}

#[test]
fn once_driver_runs_the_chain_exactly_once() {
    let seen = Arc::new(Mutex::new(0));
    App::new()
        .add(Counter {
            limit: u64::MAX,
            seen: seen.clone(),
        })
        .run()
        .unwrap();
    assert_eq!(*seen.lock().unwrap(), 1);
}

#[test]
fn fixed_rate_runs_until_exit() {
    let seen = Arc::new(Mutex::new(0));
    App::new()
        .add(Counter {
            limit: 3,
            seen: seen.clone(),
        })
        .driver(FixedRate(1000.0))
        .run()
        .unwrap();
    assert_eq!(*seen.lock().unwrap(), 3);
}

#[test]
fn unbounded_runs_until_exit() {
    let seen = Arc::new(Mutex::new(0));
    App::new()
        .add(Counter {
            limit: 5,
            seen: seen.clone(),
        })
        .driver(Unbounded)
        .run()
        .unwrap();
    assert_eq!(*seen.lock().unwrap(), 5);
}

#[test]
fn fixed_rate_rejects_invalid_frequency() {
    let err = App::new()
        .add(Counter {
            limit: 1,
            seen: Arc::new(Mutex::new(0)),
        })
        .driver(FixedRate(0.0))
        .run()
        .err()
        .unwrap();
    assert!(
        matches!(err, muesli_core::Error::Run(RunError::Driver(_))),
        "{err}"
    );
}

#[test]
fn fail_aborts_the_chain_with_module_and_stage() {
    let log = log();
    struct Failing;
    impl Module for Failing {
        type Deps = ();
        type State = ();
        fn build(self, _: (), ctx: &mut BuildCtx<'_, ()>) -> Result<(), BuildError> {
            ctx.run(Update, |_, ctx| ctx.fail("broken"));
            Ok(())
        }
    }
    let mut rt = App::new()
        .add(Failing)
        .add(SumUser { log: log.clone() })
        .add(SumProvider { log: None })
        .build()
        .unwrap();
    let err = rt.run_chain().err().unwrap();
    match &err {
        RunError::Module { module, stage, .. } => {
            assert!(module.ends_with("Failing"));
            assert!(stage.ends_with("Update"));
        }
        other => panic!("wrong error: {other}"),
    }
    // Later stages no longer run.
    assert!(
        entries(&log).iter().all(|e| !e.contains("after")),
        "{:?}",
        entries(&log)
    );
}
