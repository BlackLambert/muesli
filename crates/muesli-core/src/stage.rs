//! Stage labels and their ordering.

use std::any::{TypeId, type_name};
use std::collections::HashMap;

use crate::error::StartError;
use crate::graph::topo_sort;

/// A stage label. Stages are empty types; their order follows from
/// constraints that the core sorts topologically.
///
/// A label can bring its own constraints. This lets a contract crate define a
/// stage together with its position without containing any logic:
///
/// ```
/// use muesli_core::{Stage, StageOrder, Update};
///
/// pub struct Render;
///
/// impl Stage for Render {
///     fn configure(order: &mut StageOrder<'_>) {
///         order.after(Update);
///     }
/// }
/// ```
pub trait Stage: Send + Sync + 'static {
    /// Constraints that always apply as soon as the stage is used.
    fn configure(_order: &mut StageOrder<'_>) {}
}

/// The only stage the core defines: this is where user code runs.
#[derive(Debug, Clone, Copy, Default)]
pub struct Update;

impl Stage for Update {}

/// Runtime id of a stage.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StageId {
    type_id: TypeId,
    pub(crate) name: &'static str,
    configure: fn(&mut StageOrder<'_>),
}

impl StageId {
    pub(crate) fn of<S: Stage>() -> Self {
        Self {
            type_id: TypeId::of::<S>(),
            name: type_name::<S>(),
            configure: S::configure,
        }
    }

    /// The `TypeId` of the label (not of `StageId`).
    pub(crate) fn label_type(&self) -> TypeId {
        self.type_id
    }
}

/// All known stages and their constraints.
#[derive(Default)]
pub(crate) struct StageGraph {
    pub(crate) nodes: Vec<StageId>,
    index: HashMap<TypeId, usize>,
    /// `(a, b)`: `a` runs before `b`.
    edges: Vec<(usize, usize)>,
}

impl StageGraph {
    /// Registers a stage including its own constraints and returns its index.
    pub(crate) fn register(&mut self, id: StageId) -> usize {
        if let Some(&i) = self.index.get(&id.type_id) {
            return i;
        }
        let i = self.nodes.len();
        self.nodes.push(id);
        self.index.insert(id.type_id, i);
        (id.configure)(&mut StageOrder {
            graph: self,
            current: i,
        });
        i
    }

    pub(crate) fn order_of(&mut self, id: StageId) -> StageOrder<'_> {
        let current = self.register(id);
        StageOrder {
            graph: self,
            current,
        }
    }

    /// Topological sort. On ties the stage registered first wins, so the chain
    /// is deterministic.
    pub(crate) fn sort(&self) -> Result<Vec<usize>, StartError> {
        let mut next = vec![Vec::new(); self.nodes.len()];
        for &(a, b) in &self.edges {
            if a == b {
                return Err(StartError::StageCycle {
                    stages: vec![self.nodes[a].name],
                });
            }
            next[a].push(b);
        }
        topo_sort(&next).map_err(|stuck| StartError::StageCycle {
            stages: stuck.into_iter().map(|i| self.nodes[i].name).collect(),
        })
    }
}

/// Adds constraints for a stage.
///
/// ```ignore
/// ctx.stage(Physics).after(Update).before(Render);
/// ```
pub struct StageOrder<'a> {
    graph: &'a mut StageGraph,
    current: usize,
}

impl StageOrder<'_> {
    /// This stage runs after `other`.
    pub fn after<S: Stage>(&mut self, _other: S) -> &mut Self {
        let other = self.graph.register(StageId::of::<S>());
        self.graph.edges.push((other, self.current));
        self
    }

    /// This stage runs before `other`.
    pub fn before<S: Stage>(&mut self, _other: S) -> &mut Self {
        let other = self.graph.register(StageId::of::<S>());
        self.graph.edges.push((self.current, other));
        self
    }
}
