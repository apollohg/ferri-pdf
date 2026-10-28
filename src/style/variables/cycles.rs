use super::Values;
use std::collections::{HashMap, HashSet};

pub(super) fn members(values: &Values) -> HashSet<String> {
    let names: Vec<_> = values.keys().collect();
    let indices: HashMap<_, _> = names
        .iter()
        .enumerate()
        .map(|(i, name)| (name.as_str(), i))
        .collect();
    let mut edges = vec![Vec::new(); names.len()];
    let mut reversed = vec![Vec::new(); names.len()];
    for (index, name) in names.iter().enumerate() {
        let mut dependencies = HashSet::new();
        if let Some(value) = &values[*name] {
            value.dependencies(&mut dependencies);
        }
        for dependency in dependencies {
            if let Some(&target) = indices.get(dependency.as_str()) {
                edges[index].push(target);
                reversed[target].push(index);
            }
        }
    }
    // Finish-order and reverse traversals identify complete strongly connected components.
    let mut visited = vec![false; names.len()];
    let mut order = Vec::with_capacity(names.len());
    for start in 0..names.len() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![(start, 0)];
        while let Some((node, next)) = stack.last_mut() {
            if let Some(&target) = edges[*node].get(*next) {
                *next += 1;
                if !visited[target] {
                    visited[target] = true;
                    stack.push((target, 0));
                }
            } else {
                order.push(*node);
                stack.pop();
            }
        }
    }
    visited.fill(false);
    let mut cyclic = HashSet::new();
    for start in order.into_iter().rev() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![start];
        let mut component = Vec::new();
        while let Some(node) = stack.pop() {
            component.push(node);
            for &target in &reversed[node] {
                if !visited[target] {
                    visited[target] = true;
                    stack.push(target);
                }
            }
        }
        if component.len() > 1 || edges[start].contains(&start) {
            cyclic.extend(component.into_iter().map(|node| names[node].clone()));
        }
    }
    cyclic
}
