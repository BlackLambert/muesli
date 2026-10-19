//! Ordering helpers shared by stages and modules.

use std::collections::BTreeSet;

/// Topological sort of the nodes `0..next.len()`, where `next[a]` lists the
/// nodes that must come after `a`.
///
/// On ties the node with the smallest index wins, so the order is
/// deterministic. On a cycle, returns the nodes that could not be placed.
pub(crate) fn topo_sort(next: &[Vec<usize>]) -> Result<Vec<usize>, Vec<usize>> {
    let n = next.len();
    let mut indegree = vec![0usize; n];
    for &j in next.iter().flatten() {
        indegree[j] += 1;
    }
    let mut ready: BTreeSet<usize> = (0..n).filter(|&i| indegree[i] == 0).collect();
    let mut order = Vec::with_capacity(n);
    while let Some(i) = ready.pop_first() {
        order.push(i);
        for &j in &next[i] {
            indegree[j] -= 1;
            if indegree[j] == 0 {
                ready.insert(j);
            }
        }
    }
    if order.len() < n {
        return Err((0..n).filter(|&i| indegree[i] > 0).collect());
    }
    Ok(order)
}
