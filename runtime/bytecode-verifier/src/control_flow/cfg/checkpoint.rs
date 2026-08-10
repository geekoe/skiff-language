use std::collections::VecDeque;

use super::ControlFlowEdge;

pub(super) fn first_cycle_without_checkpoint(
    successors: &[Box<[ControlFlowEdge]>],
    checkpoints: &[bool],
) -> Result<Option<usize>, &'static str> {
    if successors.len() != checkpoints.len() {
        return Err("checkpoint classification is not dense over the ordinary CFG");
    }

    let mut indegree = vec![0_u64; successors.len()];
    let mut retained = 0_usize;
    for (source, edges) in successors.iter().enumerate() {
        if checkpoints[source] {
            continue;
        }
        retained = retained
            .checked_add(1)
            .ok_or("retained CFG node count overflowed usize")?;
        for edge in edges {
            let target = edge.target.get() as usize;
            let Some(target_checkpoint) = checkpoints.get(target) else {
                return Err("ordinary CFG edge target is outside the dense function slice");
            };
            if *target_checkpoint {
                continue;
            }
            indegree[target] = indegree[target]
                .checked_add(1)
                .ok_or("ordinary CFG indegree overflowed u64")?;
        }
    }

    let mut ready = checkpoints
        .iter()
        .enumerate()
        .filter_map(|(node, checkpoint)| (!checkpoint && indegree[node] == 0).then_some(node))
        .collect::<VecDeque<_>>();
    let mut removed = 0_usize;
    while let Some(source) = ready.pop_front() {
        removed = removed
            .checked_add(1)
            .ok_or("removed CFG node count overflowed usize")?;
        for edge in &successors[source] {
            let target = edge.target.get() as usize;
            if checkpoints[target] {
                continue;
            }
            indegree[target] = indegree[target]
                .checked_sub(1)
                .ok_or("ordinary CFG indegree underflowed during Kahn traversal")?;
            if indegree[target] == 0 {
                ready.push_back(target);
            }
        }
    }

    if removed == retained {
        Ok(None)
    } else {
        Ok(indegree.iter().position(|degree| *degree != 0))
    }
}
