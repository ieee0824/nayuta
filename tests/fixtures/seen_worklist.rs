use std::collections::HashSet;

fn walk(edges: &[Vec<usize>], start: usize) {
    let mut pending = vec![start];
    let mut seen = HashSet::new();
    while let Some(vertex) = pending.pop() {
        if !seen.insert(vertex) {
            continue;
        }
        for &next in &edges[vertex] {
            pending.push(next);
        }
    }
}

fn scan_groups(edges: &[Vec<usize>], groups: &[Vec<usize>]) {
    for group in groups {
        for &root in group {
            walk(edges, root);
        }
    }
}
