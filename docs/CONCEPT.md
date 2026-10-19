# Muesli – Concept

The source of truth is the concept document
["Muesli – Konzept"](https://claude.ai/artifact/GMVChCQnBa4aJmuGN1Pqqu)
(written in German). This file records where the implementation deviates from
it or makes it more precise. Every deviation is added here.

## Status

| Milestone | Status |
| --- | --- |
| M0 Core | done: `crates/muesli-core`, 20 tests |
| M1 Headless | open |
| M2 Window | open |
| M3 Graphics | open |
| M4 E1 facade | open |
| M5 Descent | open |

## Decisions from M0

### Module = configuration, State = private state

The concept specifies `fn build(deps, ctx) -> Result<Self>`. That leaves no
way to pass configuration such as `.add(SortViz::new(200))` into `build`. The
implementation therefore uses the split the "Handle and State" section already
describes:

```rust
pub trait Module: Send + 'static {
    type Deps: DepSet;          // e.g. (Handle<Window>, Handle<Draw2d>)
    type State: Send + 'static; // owned by the scheduler
    fn build(self, deps: Self::Deps, ctx: &mut BuildCtx<'_, Self::State>)
        -> Result<Self::State, BuildError>;
}

pub trait Provider<C: Contract>: Send + 'static {
    fn process(&mut self, cmds: Drain<'_, C::Command>, ctx: &mut StageCtx<'_>);
    fn publish(&self, snapshot: &mut C::Snapshot);
}
```

- The value passed to `App::add` is the configuration. `build` consumes it and
  returns the state.
- `Provider<C>` is implemented on the state.
- `schedule()` is gone. Work is registered in `build`:
  `ctx.run(Update, |state, ctx| ...)`, providers with
  `ctx.provide::<Draw2d>(Render)`.

### Build order and dependencies

- Channels (queue + snapshot) are created on first access, so a module gets
  its handles whether or not the provider has been built yet. Modules are
  therefore built in `add` order.
- The topological order from `Deps` instead determines execution order within
  a stage (providers before users) and teardown order (users before providers).
- Missing providers, duplicate providers and dependency cycles are still
  startup errors that name the module and the contract. A module may use a
  contract it provides itself.

### Snapshot rule clarified

The concept says "snapshot of the last run" in one place and "later stages see
the new state" in another. What is meant, and implemented: reading calls
always see the **most recently published** snapshot. Stages after the provider
see the new state, stages before it see the previous run's.

Within a stage, all provider drains run first, then registered work, then all
publishes.

### Stages

- A stage label can bring its own constraints (`Stage::configure`). This lets
  a contract crate define `Render` together with "after `Update`" without
  containing any logic.
- A module adds further constraints with
  `ctx.stage(Physics).after(Update).before(Render)`.
- On ties the stage registered first wins, so the chain is deterministic.
  Contradictions are a startup error.

### Lifecycle, errors, time

- **Exit:** `StageCtx::request_exit()` completes the current run, `run_chain`
  returns `Flow::Exit`, and the driver stops.
- **Runtime errors:** `StageCtx::fail(err)` aborts the chain immediately.
  `run_chain` returns `RunError::Module` with module and stage.
- **Teardown:** When the `Runtime` is dropped, in reverse dependency order.
- **Time:** `StageCtx::frame()` and `StageCtx::delta()` instead of a dedicated
  time contract. A contract can follow later if modules should manipulate time
  (pause, slow motion).
- **IDs:** `Handle::alloc_id()` immediately returns an id unique per contract.

### Drivers

- `App::driver(d)` sets the driver; without one, `Once` runs.
- `FixedRate(hz)` is already part of M0.
- `Unbounded` runs the chain without pausing until exit (benchmarks, batch
  simulations). A frame-paced loop like Unity's `Update` comes with the
  window driver in M2, paced by VSync.
- Open for M2: whether a module like `muesli-window-winit` may suggest a
  default driver when the app sets none.
- `Runtime::run_stage::<S>()` runs a single stage (E3). It does not count as a
  run.

### Language

Code, comments, error messages and documentation are in English.

## Decisions after M0

### Fixed timestep and stage groups (planned for M2)

Unity-style `FixedUpdate` is modelled inside the single chain, not with a
second driver or a second loop.

- **One chain, one driver.** `run_chain` needs exclusive access to the
  `Runtime`, and the queue and snapshot rules are defined per run and stage.
  Two independent clocks would make "before" and "after" undefined.
- **The accumulator lives in the `Runtime`, not in the driver.** A driver only
  decides *when* a frame happens (with winit, the event loop decides). Every
  driver gets fixed steps for free instead of reimplementing them.
- **Stage groups:** stages can belong to a group (similar to system groups in
  Unity ECS). Groups are ordered among each other like stages, and stages are
  sorted within their group. A constraint between stages of different groups
  is a startup error unless it agrees with the group order. Groups also give
  tools and visualisations a structure to show.
- **Fixed groups:** "fixed" is a property of a group, not of a single stage,
  e.g. a `FixedUpdate` group containing `Physics`.
- **Rate:** `App::fixed_rate(hz)` sets the app-wide default rate. A fixed group
  may set its own rate; without one it uses the app-wide default. Each fixed
  group keeps its own accumulator.
- **Per frame:** `run_chain` adds `delta` to the accumulators and runs the
  groups in order. A fixed group runs once per full step that fits (possibly
  zero times), every other group runs once. The number of steps per frame is
  capped so a slow frame cannot cause a spiral of death.
- **Time in fixed stages:** `StageCtx::delta()` returns the fixed step. A new
  interpolation factor (e.g. `StageCtx::alpha()`) lets render code blend
  between the last two fixed states.
- **Rules unchanged:** commands pushed in `Update` take effect in the next
  fixed step; `Render` sees the snapshot published by the last step.
- **Out of scope for now:** running fixed groups on their own thread. The
  contract model (queues and snapshots only) keeps that option open.

### Snapshot reads are bound to the stage context (done)

`Handle::snapshot` takes the `StageCtx` of the running work:
`handle.snapshot(ctx)`. The returned `SnapshotRef` cannot outlive that context,
which only exists for one task call. A held snapshot reference therefore can no
longer block the next publish, not even from another thread or through a
`'static` handle. The compiler enforces this (see the `compile_fail` doctest on
`Handle::snapshot`).

- Reading outside of stage calls (in `build` or `Drop`) is no longer possible.
  There is no meaningful "current" snapshot there.
- `StageCtx::request_exit` and `StageCtx::fail` now take `&self`, so a module
  can hold a snapshot reference and still use the context.
- Lock-free snapshots (swapping an `Arc` per publish) stay an option for when
  readers on other threads are needed.
