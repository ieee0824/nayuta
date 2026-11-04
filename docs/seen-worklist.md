# Conditional seen-guard worklist analysis

This recognizer adds explanations to the existing structural complexity candidates. It does not lower their degrees. It recognizes the entire function body before describing a per-call bound; recognizing a guard somewhere inside a larger function is insufficient.

## Supported grammar

The canonical example is [seen_worklist.rs](../tests/fixtures/seen_worklist.rs). Binding names can change, but they must remain distinct.

- A top-level synchronous free function in a scanned root file, with exactly two parameters in order: `edges: &[Vec<usize>]`, `start: usize`. No generics, explicit return type, receiver, const/unsafe/extern qualifiers, or additional statements.
- Exactly `let mut pending = vec![start];`, then `let mut seen = HashSet::new();`, then the traversal.
- Exactly `while let Some(vertex) = pending.pop()` with two body statements: `if !seen.insert(vertex) { continue; }`, then `for &next in &edges[vertex] { pending.push(next); }`.
- Standard `Vec` and `Some` prelude names, `vec!`, explicitly imported `std::collections::HashSet`, or their fully qualified standard paths. Direct imports and renamed imports of these standard items are supported. Implicit `HashSet` names are not recognized.
- Standard names must be unambiguous. Shadowing, conflicting definitions, a local `std` binding, glob imports, top-level macros, and attributes other than documentation cause conservative fallback. Macro expansion and cfg evaluation are not performed. A non-documentation attribute anywhere in the file currently disables these extra explanations.
- Inline/nested module functions, external module files when scanned as part of a parent module, methods, closures, function aliases, and cross-file propagation are outside the initial scope. Ordinary structural analysis continues to support its existing module and call features.

The supported caller is exactly `scan_groups` in the example: parameters `edges: &[Vec<usize>]` and `groups: &[Vec<usize>]`, a loop over groups, a loop over each group's roots, and one direct call to a uniquely recognized function in the same file, passing `edges` and that root in order. No extra statements or work may be present. This whole-body restriction is why its explanation can cover the caller function rather than just the call site.

These restrictions intentionally reject some equivalent valid programs, including reversed local declarations, typed locals, extra parentheses, trailing commas inside `vec!`, labeled loops, and explicit `-> ()`. Unsupported syntax produces the original structural candidates, not a claim of high actual complexity or a claim that no deduplication exists.

## Operation-count argument and cost assumptions

Let V be the number of vertices reachable from the valid start ID. Let E be the sum of the lengths of those vertices' adjacency lists. Duplicate entries, self-loops, and cycles are allowed; each adjacency entry counts separately.

The set starts empty and is only extended. Each vertex therefore passes the guard once. Each adjacency list is scanned once, so there are exactly E inner iterations and E pushes from edges. Including the initial root, there are exactly 1 + E successful pops and insertion attempts. Finiteness follows from the finite graph and these bounds on additions. This is an argument for the recognized template, not a general termination analysis of arbitrary Rust code.

These counts are deterministic. Expected O(V + E) time additionally assumes expected amortized constant cost for standard HashSet insertion, amortized constant cost for Vec push, and constant cost for usize hashing/equality, indexing, and other primitive operations. These assumptions are consistent with the [Rust collection cost model](https://doc.rust-lang.org/std/collections/index.html#performance). The analyzer does not prove an allocator cost model or a worst-case constant hash-table lookup bound.

For the caller, G is the number of groups (including empty groups), R is the sum of group lengths, and V_r/E_r describe the subgraph reachable on call r. The expected bound is O(G + sum over r of (V_r + E_r)). Each call has V_r >= 1, so the sum also accounts for per-root overhead. For maxima V and E across calls, a coarser bound is O(G + R(V + E)). For no roots, the group traversal remains O(G), plus fixed function overhead.

The graph is supplied by reference; construction of the input graph is outside the function's bound. Creating a new seen set per call precludes amortizing visits across distinct calls. A caller-supplied shared set is not recognized.

## Fallback and counterexamples

Every statement must match the grammar, so unrecognized expressions cannot hide arbitrary work or mutation. In particular, explanations are withheld for:

- Missing, reversed, or late guards; a different inserted key or adjacency index.
- Set recreation, assignment, clear/remove, aliases, or unknown calls.
- Extra pushes outside the guarded adjacency traversal, pushes of another vertex, or nested loops around pushes.
- Extra computation before the guard, after it, inside the adjacency loop, or elsewhere in the function/caller.
- Shadowed collections, constructors, or primitive types; ambiguous function targets.

A full scan of V vertices before each guard costs Theta(V(1 + E)) even though deduplication still works. Clearing seen or adding an item on every pop outside the guard can cause nontermination. These are reasons to validate the whole template, not just recognize the spelling `insert`.

Tests cover the accepted template, standard import aliases, qualified paths, coexistence with O(N^4) candidates, caller composition, the fallback cases above, and operation counts for sparse/dense graphs, cycles, duplicate edges, disconnected components, and empty groups. A CLI test verifies locations, assumptions, and `--fail` behavior. Operation-count tests do not measure wall-clock performance or establish a proof for arbitrary inputs.

## Research basis and limits

- [Tarjan, Amortized Computational Complexity (1985)](https://www.math.u-szeged.hu/~hajnal/courses/MSc_AlgoBony/2021/Tarjan.pdf): accounting for total work across operations.
- [Gulwani and Zuleger, The Reachability-Bound Problem (PLDI 2010)](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/12/pldi10_speed.pdf): distinguishing total visits to different program locations.
- [Sinn, Zuleger and Veith, Complexity and Resource Bound Analysis of Imperative Programs Using Difference Constraints (2017)](https://pmc.ncbi.nlm.nih.gov/articles/PMC6044401/): increment/reset accounting and amortized loop bounds. Heap/alias facts require additional analysis.
- [Carbonneaux, Hoffmann and Shao, Compositional Certified Resource Bounds (PLDI 2015)](https://www.cs.cmu.edu/~janh/assets/pdf/CarbonneauxHZ15.pdf): composing resource summaries; heap-dependent facts may require auxiliary variables and separately established assertions.
- [Lian and Wang, Automatic Linear Resource Bound Analysis for Rust via Prophecy Potentials (OOPSLA 2025)](https://arxiv.org/html/2502.19810v1): borrow-aware resource analysis, with substantial IR and type machinery. Its discussion of library APIs is not evidence of out-of-the-box support for this HashSet/Vec traversal.

This implementation is a conditional template recognizer informed by these ideas, not an implementation of their inference algorithms or soundness proofs. It does not claim general graph analysis, termination proving, trait/effect analysis, or shared-state amortization.

A [paper-style description in Japanese](seen-worklist-algorithm.md) presents the algorithm, operation-count argument, caller composition, analyzer complexity, and research context.
