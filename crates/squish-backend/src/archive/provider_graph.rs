//! Content-derived provider identities over the SCC condensation graph.
use super::{ArchiveError, fail, hash_field};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(super) struct ProviderEdge {
    pub label: Vec<u8>,
    pub target: usize,
}
pub(super) struct ProviderNode {
    pub seed: String,
    pub edges: Vec<ProviderEdge>,
}
/// Computes graph identities without depending on input indices or checkout order.
pub(super) fn fingerprints(nodes: &[ProviderNode]) -> Result<Vec<String>, ArchiveError> {
    if nodes.iter().any(|node| {
        node.edges
            .windows(2)
            .any(|edges| edges[0].label >= edges[1].label)
    }) {
        return Err(fail(
            "SOPack provider edge labels must be unique and sorted",
        ));
    }
    let components = components(nodes);
    let mut owner = vec![0; nodes.len()];
    for (id, members) in components.iter().enumerate() {
        for &member in members {
            owner[member] = id;
        }
    }
    let mut pending = vec![0usize; components.len()];
    let mut parents = vec![Vec::new(); components.len()];
    for (id, members) in components.iter().enumerate() {
        let targets: BTreeSet<_> = members
            .iter()
            .flat_map(|&member| nodes[member].edges.iter().map(|edge| owner[edge.target]))
            .filter(|&target| target != id)
            .collect();
        pending[id] = targets.len();
        for target in targets {
            parents[target].push(id);
        }
    }
    let mut ready: Vec<_> = pending
        .iter()
        .enumerate()
        .filter_map(|(id, &count)| (count == 0).then_some(id))
        .collect();
    let mut result = vec![String::new(); nodes.len()];
    let mut visit_marks = vec![usize::MAX; nodes.len()];
    while let Some(id) = ready.pop() {
        fingerprint_component(
            nodes,
            &components[id],
            &owner,
            id,
            &mut result,
            &mut visit_marks,
        )?;
        for &parent in &parents[id] {
            pending[parent] -= 1;
            if pending[parent] == 0 {
                ready.push(parent);
            }
        }
    }
    Ok(result)
}
/// Iterative Kosaraju traversal prevents stack growth on long dependency chains.
fn components(nodes: &[ProviderNode]) -> Vec<Vec<usize>> {
    let mut visited = vec![false; nodes.len()];
    let mut finish = Vec::with_capacity(nodes.len());
    for start in 0..nodes.len() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![(start, 0usize)];
        while let Some((node, index)) = stack.last_mut() {
            if *index == nodes[*node].edges.len() {
                finish.push(*node);
                stack.pop();
                continue;
            }
            let target = nodes[*node].edges[*index].target;
            *index += 1;
            if !visited[target] {
                visited[target] = true;
                stack.push((target, 0));
            }
        }
    }
    let mut reverse = vec![Vec::new(); nodes.len()];
    for (id, node) in nodes.iter().enumerate() {
        for edge in &node.edges {
            reverse[edge.target].push(id);
        }
    }
    visited.fill(false);
    let mut groups = Vec::new();
    for start in finish.into_iter().rev() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![start];
        let mut members = Vec::new();
        while let Some(node) = stack.pop() {
            members.push(node);
            for &parent in &reverse[node] {
                if !visited[parent] {
                    visited[parent] = true;
                    stack.push(parent);
                }
            }
        }
        groups.push(members);
    }
    groups
}
fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
fn fingerprint_component(
    nodes: &[ProviderNode],
    members: &[usize],
    owner: &[usize],
    component: usize,
    result: &mut [String],
    visit_marks: &mut [usize],
) -> Result<(), ArchiveError> {
    let mut labels: BTreeMap<usize, String> = members
        .iter()
        .map(|&node| (node, nodes[node].seed.clone()))
        .collect();
    let tied = members.len() > 1 && labels.values().collect::<BTreeSet<_>>().len() != members.len();
    let ordered = if tied {
        if let Some(order) = anchored_order(nodes, members, owner, component, visit_marks) {
            order
        } else {
            labels = refine(nodes, members, owner, component, result)?;
            let mut order = members.to_vec();
            order.sort_by(|a, b| labels[a].cmp(&labels[b]));
            order
        }
    } else {
        let mut order = members.to_vec();
        order.sort_by(|a, b| labels[a].cmp(&labels[b]));
        order
    };
    let ordinals: BTreeMap<_, _> = ordered
        .iter()
        .enumerate()
        .map(|(ordinal, &node)| (node, ordinal as u64))
        .collect();
    let mut hash = blake3::Hasher::new();
    hash_field(&mut hash, b"sopack-provider-scc-v3");
    for &node in &ordered {
        hash_field(&mut hash, nodes[node].seed.as_bytes());
        for edge in &nodes[node].edges {
            hash_field(&mut hash, &edge.label);
            if owner[edge.target] == component {
                hash_field(&mut hash, b"internal");
                hash_field(&mut hash, &ordinals[&edge.target].to_le_bytes());
            } else {
                hash_field(&mut hash, b"external");
                hash_field(&mut hash, result[edge.target].as_bytes());
            }
        }
        hash_field(&mut hash, b"end-node");
    }
    let component_digest = hash.finalize();
    for &node in &ordered {
        let mut bytes = Vec::with_capacity(40);
        bytes.extend_from_slice(component_digest.as_bytes());
        bytes.extend_from_slice(&ordinals[&node].to_le_bytes());
        result[node] = digest(&bytes);
    }
    Ok(())
}
/// A unique content seed canonically anchors an SCC; sorted labelled BFS visits it once.
/// Strong connectivity guarantees every member is reached, without index-based tie breaks.
fn anchored_order(
    nodes: &[ProviderNode],
    members: &[usize],
    owner: &[usize],
    component: usize,
    visit_marks: &mut [usize],
) -> Option<Vec<usize>> {
    let mut seeds: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for &member in members {
        let entry = seeds.entry(&nodes[member].seed).or_insert((member, 0));
        entry.1 += 1;
    }
    let anchor = seeds
        .values()
        .find_map(|&(member, count)| (count == 1).then_some(member))?;
    visit_marks[anchor] = component;
    let mut queue = VecDeque::from([anchor]);
    let mut order = Vec::with_capacity(members.len());
    while let Some(node) = queue.pop_front() {
        order.push(node);
        for edge in &nodes[node].edges {
            if owner[edge.target] == component && visit_marks[edge.target] != component {
                visit_marks[edge.target] = component;
                queue.push_back(edge.target);
            }
        }
    }
    debug_assert_eq!(order.len(), members.len());
    Some(order)
}
/// Refinement only visits tied cyclic components, stopping as soon as their partition stabilizes.
fn refine(
    nodes: &[ProviderNode],
    members: &[usize],
    owner: &[usize],
    component: usize,
    result: &[String],
) -> Result<BTreeMap<usize, String>, ArchiveError> {
    let mut colors: BTreeMap<usize, usize> = members.iter().map(|&node| (node, 0)).collect();
    let mut previous_classes = 0;
    for _ in 0..=members.len() {
        let signatures: BTreeMap<_, _> = members
            .iter()
            .map(|&node| {
                let mut hash = blake3::Hasher::new();
                hash_field(&mut hash, nodes[node].seed.as_bytes());
                hash_field(&mut hash, &(colors[&node] as u64).to_le_bytes());
                for edge in &nodes[node].edges {
                    hash_field(&mut hash, &edge.label);
                    if owner[edge.target] == component {
                        hash_field(&mut hash, &(colors[&edge.target] as u64).to_le_bytes());
                    } else {
                        hash_field(&mut hash, result[edge.target].as_bytes());
                    }
                }
                (node, hash.finalize().to_hex().to_string())
            })
            .collect();
        let distinct: BTreeSet<_> = signatures.values().cloned().collect();
        if distinct.len() == members.len() {
            return Ok(signatures);
        }
        if distinct.len() == previous_classes {
            return Err(fail("ambiguous content-equivalent cyclic SOPack providers"));
        }
        previous_classes = distinct.len();
        let ranks: BTreeMap<_, _> = distinct
            .into_iter()
            .enumerate()
            .map(|(rank, value)| (value, rank))
            .collect();
        colors = signatures
            .into_iter()
            .map(|(node, value)| (node, ranks[&value]))
            .collect();
    }
    Err(fail("SOPack provider refinement exceeded component bound"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn graph(size: usize, cycle: bool) -> Vec<ProviderNode> {
        (0..size)
            .map(|i| ProviderNode {
                seed: format!("seed-{i:04}"),
                edges: if cycle {
                    vec![ProviderEdge {
                        label: b"import-0".to_vec(),
                        target: (i + 1) % size,
                    }]
                } else {
                    vec![]
                },
            })
            .collect()
    }
    #[test]
    fn isolated_and_unique_cycle_scale_without_refinement() {
        for size in [1, 64, 512] {
            for cycle in [false, true] {
                let values = fingerprints(&graph(size, cycle)).unwrap();
                assert_eq!(values.len(), size);
                assert_eq!(values.iter().collect::<BTreeSet<_>>().len(), size);
            }
        }
    }
    #[test]
    fn provider_indices_do_not_affect_identity() {
        let nodes = graph(8, true);
        let first = fingerprints(&nodes).unwrap();
        let reordered: Vec<_> = (0..8)
            .rev()
            .map(|i| ProviderNode {
                seed: nodes[i].seed.clone(),
                edges: vec![ProviderEdge {
                    label: b"import-0".to_vec(),
                    target: 7 - nodes[i].edges[0].target,
                }],
            })
            .collect();
        let second = fingerprints(&reordered).unwrap();
        assert_eq!(first, second.into_iter().rev().collect::<Vec<_>>());
    }
    #[test]
    fn topology_and_cycle_length_are_encoded() {
        let a = fingerprints(&graph(5, true)).unwrap();
        let b = fingerprints(&graph(6, true)).unwrap();
        assert_ne!(a[0], b[0]);
        let isolated = fingerprints(&graph(5, false)).unwrap();
        assert_ne!(a, isolated);
    }
    #[test]
    fn uniform_seed_root_anchored_512_cycle_uses_one_canonical_walk() {
        let mut nodes = graph(512, true);
        for node in &mut nodes {
            node.seed = "same-provider-content".into();
        }
        nodes[0].seed = "distinguished-root".into();
        let members: Vec<_> = (0..nodes.len()).collect();
        let owners = vec![0; nodes.len()];
        let walk = anchored_order(
            &nodes,
            &members,
            &owners,
            0,
            &mut vec![usize::MAX; nodes.len()],
        )
        .unwrap();
        assert_eq!(walk, members);
        let first = fingerprints(&nodes).unwrap();
        let reversed: Vec<_> = (0..512)
            .rev()
            .map(|i| ProviderNode {
                seed: nodes[i].seed.clone(),
                edges: vec![ProviderEdge {
                    label: b"import-0".to_vec(),
                    target: 511 - nodes[i].edges[0].target,
                }],
            })
            .collect();
        assert_eq!(
            first,
            fingerprints(&reversed)
                .unwrap()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
        );
        let mut shorter = graph(511, true);
        for node in &mut shorter {
            node.seed = "same-provider-content".into();
        }
        shorter[0].seed = "distinguished-root".into();
        assert_ne!(first[0], fingerprints(&shorter).unwrap()[0]);
    }
    #[test]
    fn ambiguous_symmetric_providers_are_rejected() {
        let mut nodes = graph(3, true);
        for node in &mut nodes {
            node.seed = "same".into();
        }
        assert!(fingerprints(&nodes).is_err());
    }
    #[test]
    fn seed_ties_can_be_disambiguated_by_external_content() {
        let mut nodes = graph(3, true);
        nodes[0].seed = "same".into();
        nodes[1].seed = "same".into();
        nodes[2].seed = "different".into();
        assert!(fingerprints(&nodes).is_ok());
    }
}
