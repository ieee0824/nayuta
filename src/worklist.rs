//! Conditional explanations for a deliberately closed grammar, not general AARA.
//! Every statement is accounted for before a whole-function bound is reported.
use std::collections::HashMap;
use syn::{Expr, Item, Pat, Stmt, Type, spanned::Spanned};

use crate::Finding;

const ASSUMPTIONS: &str = "assuming valid vertex IDs, a fixed graph, expected constant-time hashing and amortized vector growth; conditional pattern analysis, not a general proof";

// This resolver intentionally does not reuse the permissive call index: names
// that cannot be identified uniquely must not establish collection semantics.
#[derive(Default)]
struct Scope {
    names: HashMap<String, Vec<Option<String>>>,
    opaque: bool,
}
impl Scope {
    fn bind(&mut self, name: String, target: Option<String>) {
        self.names.entry(name).or_default().push(target);
    }
    fn imports(&mut self, prefix: &str, tree: &syn::UseTree) {
        match tree {
            syn::UseTree::Path(p) => self.imports(&format!("{prefix}{}::", p.ident), &p.tree),
            syn::UseTree::Name(n) => {
                self.bind(n.ident.to_string(), Some(format!("{prefix}{}", n.ident)))
            }
            syn::UseTree::Rename(n) => {
                self.bind(n.rename.to_string(), Some(format!("{prefix}{}", n.ident)))
            }
            syn::UseTree::Group(g) => {
                for t in &g.items {
                    self.imports(prefix, t);
                }
            }
            syn::UseTree::Glob(_) => self.opaque = true,
        }
    }
    fn new(file: &syn::File) -> Self {
        let mut scope = Self {
            opaque: !plain_attributes(file),
            ..Self::default()
        };
        for item in &file.items {
            let name = match item {
                Item::Use(u) => {
                    scope.imports("", &u.tree);
                    continue;
                }
                Item::Fn(x) => &x.sig.ident,
                Item::Struct(x) => &x.ident,
                Item::Enum(x) => &x.ident,
                Item::Union(x) => &x.ident,
                Item::Type(x) => &x.ident,
                Item::Const(x) => &x.ident,
                Item::Static(x) => &x.ident,
                Item::Mod(x) => &x.ident,
                Item::Trait(x) => &x.ident,
                Item::TraitAlias(x) => &x.ident,
                Item::ExternCrate(x) => x.rename.as_ref().map_or(&x.ident, |(_, id)| id),
                Item::Impl(_) => continue,
                // Macros can introduce bindings invisible to syn.
                _ => {
                    scope.opaque = true;
                    continue;
                }
            };
            scope.bind(name.to_string(), None);
        }
        scope
    }
    fn standard(&self, path: &syn::Path, target: &str, prelude: Option<&str>) -> bool {
        if self.opaque || self.names.contains_key("std") || !simple_path(path) {
            return false;
        }
        let name = crate::symbols::path_name(path);
        if name == target {
            return true;
        }
        if path.leading_colon.is_some() || path.segments.len() != 1 {
            return false;
        }
        match self.names.get(&name).map(Vec::as_slice) {
            Some([Some(import)]) => import == target,
            None => prelude == Some(name.as_str()),
            _ => false,
        }
    }
    fn primitive(&self, ty: &Type) -> bool {
        matches!(ty, Type::Path(p) if p.qself.is_none() && p.path.is_ident("usize") && !self.names.contains_key("usize"))
    }
    fn graph(&self, ty: &Type) -> bool {
        let Type::Reference(r) = ty else {
            return false;
        };
        let Type::Slice(slice) = &*r.elem else {
            return false;
        };
        let Type::Path(p) = &*slice.elem else {
            return false;
        };
        if r.mutability.is_some() || p.qself.is_some() {
            return false;
        }
        let mut path = p.path.clone();
        let Some(last) = path.segments.last_mut() else {
            return false;
        };
        let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
            return false;
        };
        if args.args.len() != 1
            || !matches!(&args.args[0], syn::GenericArgument::Type(t) if self.primitive(t))
        {
            return false;
        }
        last.arguments = syn::PathArguments::None;
        self.standard(&path, "std::vec::Vec", Some("Vec"))
    }
}

// Attributes (including cfg and proc macros) may change the grammar or bindings.
// Documentation attributes do not. Inspect the entire file conservatively.
fn plain_attributes(file: &syn::File) -> bool {
    struct Attributes(bool);
    impl<'ast> syn::visit::Visit<'ast> for Attributes {
        fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
            if !attr.path().is_ident("doc") {
                self.0 = false;
            }
        }
    }
    let mut check = Attributes(true);
    syn::visit::Visit::visit_file(&mut check, file);
    check.0
}
fn simple_path(path: &syn::Path) -> bool {
    path.segments
        .iter()
        .all(|p| matches!(p.arguments, syn::PathArguments::None))
}
fn ident(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Path(p) if p.qself.is_none() => p.path.get_ident().map(ToString::to_string),
        _ => None,
    }
}
fn binding(pat: &Pat, mutable: bool) -> Option<String> {
    match pat {
        Pat::Ident(p)
            if p.by_ref.is_none() && p.subpat.is_none() && p.mutability.is_some() == mutable =>
        {
            Some(p.ident.to_string())
        }
        _ => None,
    }
}
fn ref_binding(pat: &Pat) -> Option<String> {
    match pat {
        Pat::Reference(p) if p.mutability.is_none() => binding(&p.pat, false),
        _ => None,
    }
}
fn statement(stmt: &Stmt) -> Option<&Expr> {
    match stmt {
        Stmt::Expr(e, _) => Some(e),
        _ => None,
    }
}
fn local(stmt: &Stmt) -> Option<(String, &Expr)> {
    let Stmt::Local(l) = stmt else {
        return None;
    };
    let init = l.init.as_ref()?;
    if init.diverge.is_some() {
        return None;
    }
    Some((binding(&l.pat, true)?, &init.expr))
}
fn method<'a>(
    expr: &'a Expr,
    receiver: &str,
    name: &str,
    count: usize,
) -> Option<&'a syn::ExprMethodCall> {
    let Expr::MethodCall(m) = expr else {
        return None;
    };
    (ident(&m.receiver).as_deref() == Some(receiver)
        && m.method == name
        && m.turbofish.is_none()
        && m.args.len() == count)
        .then_some(m)
}
fn distinct(names: &[&str]) -> bool {
    names
        .iter()
        .enumerate()
        .all(|(i, n)| !names[..i].contains(n))
}
fn signature(f: &syn::ItemFn) -> Option<[(String, &Type); 2]> {
    let s = &f.sig;
    if s.asyncness.is_some()
        || s.unsafety.is_some()
        || s.constness.is_some()
        || s.abi.is_some()
        || s.variadic.is_some()
        || !s.generics.params.is_empty()
        || s.generics.where_clause.is_some()
        || !matches!(s.output, syn::ReturnType::Default)
        || s.inputs.len() != 2
    {
        return None;
    }
    let arg = |a: &syn::FnArg| match a {
        syn::FnArg::Typed(a) => binding(&a.pat, false),
        _ => None,
    };
    let syn::FnArg::Typed(a) = &s.inputs[0] else {
        return None;
    };
    let syn::FnArg::Typed(b) = &s.inputs[1] else {
        return None;
    };
    Some([(arg(&s.inputs[0])?, &a.ty), (arg(&s.inputs[1])?, &b.ty)])
}
fn worklist(f: &syn::ItemFn, scope: &Scope) -> Option<usize> {
    let [(edges, et), (start, st)] = signature(f)?;
    if !scope.graph(et) || !scope.primitive(st) {
        return None;
    }
    let [p, s, w] = f.block.stmts.as_slice() else {
        return None;
    };
    let (pending, init) = local(p)?;
    let Expr::Macro(mac) = init else {
        return None;
    };
    if !scope.standard(&mac.mac.path, "std::vec", Some("vec")) {
        return None;
    }
    let seed = syn::parse2::<Expr>(mac.mac.tokens.clone()).ok()?;
    if ident(&seed).as_deref() != Some(&start) {
        return None;
    }
    let (seen, init) = local(s)?;
    let Expr::Call(new) = init else {
        return None;
    };
    let Expr::Path(constructor) = &*new.func else {
        return None;
    };
    let mut path = constructor.path.clone();
    let last = path.segments.pop()?.into_value();
    path.segments.pop_punct();
    if constructor.qself.is_some()
        || !new.args.is_empty()
        || last.ident != "new"
        || !matches!(last.arguments, syn::PathArguments::None)
        || !scope.standard(&path, "std::collections::HashSet", None)
    {
        return None;
    }
    let Expr::While(w) = statement(w)? else {
        return None;
    };
    if w.label.is_some() {
        return None;
    }
    let Expr::Let(condition) = &*w.cond else {
        return None;
    };
    let Pat::TupleStruct(some) = &*condition.pat else {
        return None;
    };
    if some.qself.is_some()
        || some.elems.len() != 1
        || !scope.standard(&some.path, "std::option::Option::Some", Some("Some"))
    {
        return None;
    }
    let vertex = binding(&some.elems[0], false)?;
    method(&condition.expr, &pending, "pop", 0)?;
    let [guard, adjacency] = w.body.stmts.as_slice() else {
        return None;
    };
    let Expr::If(guard) = statement(guard)? else {
        return None;
    };
    if guard.else_branch.is_some() {
        return None;
    }
    let Expr::Unary(negation) = &*guard.cond else {
        return None;
    };
    if !matches!(negation.op, syn::UnOp::Not(_)) {
        return None;
    }
    let insert = method(&negation.expr, &seen, "insert", 1)?;
    if ident(&insert.args[0]).as_deref() != Some(&vertex) {
        return None;
    }
    let [skip] = guard.then_branch.stmts.as_slice() else {
        return None;
    };
    if !matches!(statement(skip)?, Expr::Continue(c) if c.label.is_none()) {
        return None;
    }
    let Expr::ForLoop(adjacency) = statement(adjacency)? else {
        return None;
    };
    if adjacency.label.is_some() {
        return None;
    }
    let next = ref_binding(&adjacency.pat)?;
    let Expr::Reference(reference) = &*adjacency.expr else {
        return None;
    };
    let Expr::Index(index) = &*reference.expr else {
        return None;
    };
    if reference.mutability.is_some()
        || ident(&index.expr).as_deref() != Some(&edges)
        || ident(&index.index).as_deref() != Some(&vertex)
    {
        return None;
    }
    let [push] = adjacency.body.stmts.as_slice() else {
        return None;
    };
    let push = method(statement(push)?, &pending, "push", 1)?;
    if ident(&push.args[0]).as_deref() != Some(&next)
        || !distinct(&[&edges, &start, &pending, &seen, &vertex, &next])
    {
        return None;
    }
    // Local value names can shadow the Option constructor in patterns.
    if [&edges, &start, &pending, &seen]
        .iter()
        .any(|name| name.as_str() == "Some")
    {
        return None;
    }
    Some(w.while_token.span.start().line)
}

fn groups(f: &syn::ItemFn, scope: &Scope, walks: &HashMap<String, usize>) -> Option<usize> {
    let [(edges, et), (groups, gt)] = signature(f)?;
    if !scope.graph(et) || !scope.graph(gt) {
        return None;
    }
    let [outer] = f.block.stmts.as_slice() else {
        return None;
    };
    let Expr::ForLoop(outer) = statement(outer)? else {
        return None;
    };
    if outer.label.is_some() || ident(&outer.expr).as_deref() != Some(&groups) {
        return None;
    }
    let group = binding(&outer.pat, false)?;
    let [inner] = outer.body.stmts.as_slice() else {
        return None;
    };
    let Expr::ForLoop(inner) = statement(inner)? else {
        return None;
    };
    if inner.label.is_some() || ident(&inner.expr).as_deref() != Some(&group) {
        return None;
    }
    let root = ref_binding(&inner.pat)?;
    let [call] = inner.body.stmts.as_slice() else {
        return None;
    };
    let Expr::Call(call) = statement(call)? else {
        return None;
    };
    let callee = ident(&call.func)?;
    if !walks.contains_key(&callee)
        || call.args.len() != 2
        || ident(&call.args[0]).as_deref() != Some(&edges)
        || ident(&call.args[1]).as_deref() != Some(&root)
        || !distinct(&[&edges, &groups, &group, &root, &callee])
    {
        return None;
    }
    Some(call.span().start().line)
}

pub(crate) fn analyze(file: &syn::File) -> Vec<Finding> {
    let scope = Scope::new(file);
    if scope.opaque {
        return Vec::new();
    }
    let functions: Vec<_> = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(f)
                if scope
                    .names
                    .get(&f.sig.ident.to_string())
                    .is_some_and(|defs| defs.len() == 1) =>
            {
                Some(f)
            }
            _ => None,
        })
        .collect();
    let walks: HashMap<_, _> = functions
        .iter()
        .filter_map(|f| worklist(f, &scope).map(|line| (f.sig.ident.to_string(), line)))
        .collect();
    let mut findings = Vec::new();
    for f in functions {
        if let Some(line) = walks.get(&f.sig.ident.to_string()) {
            findings.push(Finding {
                line: *line,
                conditional: true,
                message: format!("deduplicated worklist candidate: expected O(V + E) per call; V = reachable vertices, E = their adjacency entries; fresh seen set per call; {ASSUMPTIONS}"),
            });
        } else if let Some(line) = groups(f, &scope, &walks) {
            findings.push(Finding {
                line,
                conditional: true,
                message: format!("deduplicated worklist calls: expected O(G + sum_r(V_r + E_r)) for this function; G = number of groups, R = total roots/calls, r ranges over R calls, V_r/E_r = reachable vertices/adjacency entries of call r; fresh seen set for each call, no sharing across roots; {ASSUMPTIONS}"),
            });
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    const EXAMPLE: &str = include_str!("../tests/fixtures/seen_worklist.rs");
    fn notes(source: &str) -> Vec<Finding> {
        analyze(&syn::parse_file(source).unwrap())
    }
    #[test]
    fn explains_reachable_edges_and_fresh_sets_per_call() {
        let found = notes(EXAMPLE);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].line, 6);
        assert!(found[0].message.contains("expected O(V + E) per call"));
        assert!(found[0].message.contains("E = their adjacency entries"));
        assert_eq!(found[1].line, 19);
        assert!(found[1].message.contains("O(G + sum_r(V_r + E_r))"));
        assert!(found[1].message.contains("R = total roots/calls"));
        assert!(found[1].message.contains("no sharing across roots"));
        // The structural candidates still coexist with the new explanations.
        let all = crate::analyze(EXAMPLE);
        assert!(
            all.iter()
                .any(|f| !f.conditional && f.message.contains("O(N^4)"))
        );
        assert_eq!(all.iter().filter(|f| f.conditional).count(), 2);
    }
    #[test]
    fn accepts_resolved_standard_names_and_renamed_bindings() {
        for source in [
            EXAMPLE
                .replace(
                    "use std::collections::HashSet;",
                    "use std::collections::HashSet as Visited;",
                )
                .replace("HashSet::new", "Visited::new"),
            EXAMPLE
                .replace("use std::collections::HashSet;", "")
                .replace("HashSet::new", "::std::collections::HashSet::new")
                .replace("Vec<", "::std::vec::Vec<")
                .replace("vec![", "::std::vec![")
                .replace("Some(vertex)", "::std::option::Option::Some(vertex)"),
            EXAMPLE
                .replace("vertex", "node")
                .replace("pending", "stack")
                .replace("seen", "visited")
                .replace("next", "neighbor"),
            format!("struct Unrelated;\n{EXAMPLE}"),
        ] {
            assert_eq!(notes(&source).len(), 2, "{source}");
        }
    }
    #[test]
    fn rejects_unproven_traversals() {
        let cases = [
            ("if !seen.insert(vertex)", "if false"),
            ("if !seen.insert(vertex)", "if seen.insert(vertex)"),
            ("seen.insert(vertex)", "seen.insert(start)"),
            ("&edges[vertex]", "&edges[start]"),
            ("pending.push(next);", "pending.push(vertex);"),
            ("continue;", "break;"),
            ("continue;", "continue 'outer;"),
            ("continue;", "unknown(); continue;"),
            (
                "if !seen.insert(vertex)",
                "seen.clear(); if !seen.insert(vertex)",
            ),
            (
                "if !seen.insert(vertex)",
                "seen.remove(&vertex); if !seen.insert(vertex)",
            ),
            (
                "if !seen.insert(vertex)",
                "seen = HashSet::new(); if !seen.insert(vertex)",
            ),
            (
                "if !seen.insert(vertex)",
                "let mut seen = HashSet::new(); if !seen.insert(vertex)",
            ),
            (
                "if !seen.insert(vertex)",
                "let alias = &mut seen; unknown(alias); if !seen.insert(vertex)",
            ),
            (
                "if !seen.insert(vertex)",
                "unknown(&mut seen); if !seen.insert(vertex)",
            ),
            (
                "if !seen.insert(vertex)",
                "pending.push(vertex); if !seen.insert(vertex)",
            ),
            (
                "if !seen.insert(vertex)",
                "for _ in edges {} if !seen.insert(vertex)",
            ),
            ("for &next", "for _ in edges {} for &next"),
            ("pending.push(next);", "unknown(); pending.push(next);"),
            (
                "pending.push(next);",
                "for _ in edges { pending.push(next); }",
            ),
            ("pending.push(next);", "pending.push(next); seen.clear();"),
            (
                "for &next in &edges[vertex]",
                "for &vertex in &edges[vertex]",
            ),
            ("let mut seen = HashSet::new();", "let mut seen = other();"),
            ("vec![start]", "vec![start; edges.len()]"),
            ("start: usize", "start: Custom"),
            ("Vec<usize>", "Custom<usize>"),
            ("fn walk(", "async fn walk("),
            ("fn walk(", "fn walk<T>("),
        ];
        for (from, to) in cases {
            assert!(EXAMPLE.contains(from));
            let source = EXAMPLE.replacen(from, to, 1);
            assert!(
                notes(&source).is_empty(),
                "incorrect explanation for {from} -> {to}"
            );
            assert!(crate::analyze(&source).iter().any(|f| !f.conditional));
        }
        // Guard after adjacency scanning cannot protect its cost.
        let late = EXAMPLE.replace(
            "        if !seen.insert(vertex) {\n            continue;\n        }\n", ""
        ).replace("            pending.push(next);", "            pending.push(next);\n            if !seen.insert(vertex) { continue; }");
        assert!(notes(&late).is_empty());
    }
    #[test]
    fn rejects_shadowing_ambiguous_names_and_macros() {
        for prefix in [
            "struct HashSet;",
            "struct Vec;",
            "type usize = u64;",
            "mod std {}",
            "fn Some() {}",
            "use other::*;",
            "use other::HashSet;",
            "macro_rules! vec { ($x:expr) => { unknown() } }",
            "make_types!();",
            "#![no_implicit_prelude]",
            "#[cfg(feature = \"x\")] struct Whatever;",
        ] {
            assert!(
                notes(&format!("{prefix}\n{EXAMPLE}")).is_empty(),
                "{prefix}"
            );
        }
        assert!(notes(&EXAMPLE.replace("std::collections::HashSet", "other::HashSet")).is_empty());
        assert!(notes(&EXAMPLE.replace("pending", "Some")).is_empty());
        assert!(notes(&format!("{EXAMPLE}\nfn walk() {{}} ")).is_empty());
    }
    #[test]
    fn caller_explanations_require_complete_matching_bodies() {
        for (from, to) in [
            ("walk(edges, root);", "walk(edges, root); unknown();"),
            ("walk(edges, root);", "walk(other, root);"),
            ("walk(edges, root);", "alias(edges, root);"),
            ("for group in groups", "for group in other"),
            ("for &root in group", "for &root in groups[0]"),
            ("fn scan_groups(edges:", "fn scan_groups(walk:"),
        ] {
            let found = notes(&EXAMPLE.replacen(from, to, 1));
            assert_eq!(found.len(), 1, "{from} -> {to}");
            assert!(found[0].message.contains("per call"));
        }
        // Shared state is outside the recognized signature and fresh-set body.
        let shared = EXAMPLE
            .replace("start: usize)", "start: usize, seen: &mut HashSet<usize>)")
            .replace("    let mut seen = HashSet::new();\n", "");
        assert!(notes(&shared).is_empty());
    }
    #[test]
    fn respects_module_boundaries_and_custom_collection_types() {
        let custom = EXAMPLE.replace(
            "use std::collections::HashSet;",
            "struct HashSet; impl HashSet { fn new() -> Self { Self } fn insert(&mut self, _: usize) -> bool { true } }",
        );
        assert!(notes(&custom).is_empty());
        assert!(notes(&format!("mod nested {{ {EXAMPLE} }}")).is_empty());

        let root = syn::parse_file("mod child;").unwrap();
        let child = syn::parse_file(EXAMPLE).unwrap();
        let index = crate::symbols::Index::build(&[
            ("src/main.rs".into(), root),
            ("src/child.rs".into(), child),
        ]);
        let module = &index.modules[std::path::Path::new("src/child.rs")];
        let findings = crate::analyze_with_index(EXAMPLE, Some((&index, module))).unwrap();
        assert!(findings.iter().all(|f| !f.conditional));
        assert!(findings.iter().any(|f| f.message.contains("O(N^4)")));
    }
    #[test]
    fn counts_edges_in_sparse_dense_and_duplicate_graphs() {
        fn count(edges: &[Vec<usize>], start: usize) -> (usize, usize, usize) {
            let mut pending = vec![start];
            let mut seen = std::collections::HashSet::new();
            let (mut pops, mut scans) = (0, 0);
            while let Some(vertex) = pending.pop() {
                pops += 1;
                if !seen.insert(vertex) {
                    continue;
                }
                for &next in &edges[vertex] {
                    scans += 1;
                    pending.push(next);
                }
            }
            let e: usize = seen.iter().map(|&v| edges[v].len()).sum();
            assert_eq!(scans, e);
            assert_eq!(pops, 1 + e);
            (seen.len(), scans, pops)
        }
        assert_eq!(count(&[vec![1], vec![2], vec![], vec![3]], 0), (3, 2, 3));
        assert_eq!(count(&vec![(0..8).collect(); 8], 0), (8, 64, 65));
        assert_eq!(count(&[vec![0, 1, 1], vec![0, 1]], 0), (2, 5, 6));
        // Empty groups still incur outer-loop work; roots may reach disjoint components.
        let edges = [vec![1], vec![], vec![2]];
        let groups = [vec![], vec![0, 2], vec![]];
        let costs: Vec<_> = groups.iter().flatten().map(|&r| count(&edges, r)).collect();
        assert_eq!(groups.len(), 3);
        assert_eq!(costs, [(2, 1, 2), (1, 1, 2)]);
    }
}
