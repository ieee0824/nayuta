use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
mod memory;
mod symbols;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Finding {
    line: usize,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenKind {
    Ident(String),
    OpenBrace,
    CloseBrace,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    kind: TokenKind,
    line: usize,
}

#[derive(Debug)]
struct Block {
    loop_depth: usize,
}

fn main() -> ExitCode {
    if env::args_os().nth(1).is_some_and(|arg| arg == "memory") {
        return memory::run(env::args_os().skip(2).collect());
    }
    let mut fail_on_findings = false;
    let mut paths = Vec::new();
    for arg in env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            "--fail" => fail_on_findings = true,
            _ if arg.starts_with('-') => {
                eprintln!("unknown option: {arg}\n");
                print_help();
                return ExitCode::from(2);
            }
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    if paths.is_empty() {
        paths.push(PathBuf::from("."));
    }

    let mut files = Vec::new();
    let mut had_error = false;
    for path in paths {
        if let Err(error) = collect_rust_files(&path, &mut files) {
            eprintln!("nayuta: {}: {error}", path.display());
            had_error = true;
        }
    }
    files.sort();
    files.dedup();

    let parsed_files: Vec<_> = files
        .iter()
        .filter_map(|path| {
            let source = fs::read_to_string(path).ok()?;
            Some((path.clone(), syn::parse_file(&source).ok()?))
        })
        .collect();
    let index = symbols::Index::build(&parsed_files);
    let mut count = 0;
    for file in &files {
        match fs::read_to_string(file) {
            Ok(source) => {
                let findings = match analyze_with_index(
                    &source,
                    Some((
                        &index,
                        index
                            .modules
                            .get(file)
                            .map(String::as_str)
                            .unwrap_or("root"),
                    )),
                ) {
                    Ok(findings) => findings,
                    Err(error) => {
                        eprintln!("nayuta: {}: parse error: {error}", file.display());
                        had_error = true;
                        continue;
                    }
                };
                for finding in findings {
                    println!(
                        "{}:{}: complexity: {} (syntactic heuristic; assumes independent N-sized iterations; amortization and deduplication are not analyzed)",
                        file.display(),
                        finding.line,
                        finding.message
                    );
                    count += 1;
                }
            }
            Err(error) => {
                eprintln!("nayuta: {}: {error}", file.display());
                had_error = true;
            }
        }
    }
    eprintln!(
        "nayuta: scanned {} Rust file(s), found {count} issue(s)",
        files.len()
    );
    if had_error || (fail_on_findings && count > 0) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn print_help() {
    println!(
        "nayuta - find likely high-complexity code in Rust\n\nUsage: nayuta [--fail] [PATH ...]\n       nayuta memory --help\n\nRecursively scans PATH (the current directory by default).\n--fail  exit with status 1 when a finding is reported"
    );
}

fn collect_rust_files(path: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Ok(());
    }
    if path.is_file() {
        if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(fs::canonicalize(path)?);
        }
        return Ok(());
    }
    let mut failure = None;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        if entry.file_type()?.is_symlink() {
            continue;
        }
        if child.is_dir() {
            let name = entry.file_name();
            if name != "target"
                && name != ".git"
                && let Err(error) = collect_rust_files(&child, files)
            {
                eprintln!("nayuta: {}: {error}", child.display());
                failure = Some(error);
            }
        } else if entry.file_type()?.is_file()
            && child.extension().is_some_and(|extension| extension == "rs")
        {
            files.push(fs::canonicalize(child)?);
        }
    }
    failure.map_or(Ok(()), Err)
}

#[cfg(test)]
fn analyze(source: &str) -> Vec<Finding> {
    try_analyze(source).expect("valid test source")
}

#[cfg(test)]
fn try_analyze(source: &str) -> syn::Result<Vec<Finding>> {
    analyze_with_index(source, None)
}
fn analyze_with_index(
    source: &str,
    context: Option<(&symbols::Index, &str)>,
) -> syn::Result<Vec<Finding>> {
    use syn::visit::{self, Visit};
    struct Loops<'a> {
        functions: &'a symbols::Index,
        module: String,
        owner: Option<String>,
        types: std::collections::HashMap<String, String>,
        recursive: bool,
        active: Vec<String>,
        max_depth: usize,
        depth: usize,
        findings: Vec<Finding>,
    }
    impl Loops<'_> {
        fn bind_signature(&mut self, sig: &syn::Signature) {
            self.types.clear();
            for arg in &sig.inputs {
                if let syn::FnArg::Typed(arg) = arg
                    && let syn::Pat::Ident(pat) = &*arg.pat
                    && let Some(name) = type_name(&arg.ty)
                {
                    self.types.insert(
                        pat.ident.to_string(),
                        self.functions.qualify(&self.module, &name),
                    );
                }
            }
            if let Some(owner) = &self.owner {
                self.types.insert("self".into(), owner.clone());
            }
        }
        fn call(&mut self, key: &str, span: proc_macro2::Span) {
            let Some(defs) = self.functions.functions.get(key) else {
                return;
            };
            if defs.len() != 1 || defs[0].sig.asyncness.is_some() {
                return;
            }
            if self.active.iter().any(|x| x == key) {
                self.recursive = true;
                self.findings.push(Finding {
                    line: span.start().line,
                    message: format!(
                        "recursive call to {key}; recursion risk (see caller estimate)"
                    ),
                });
                return;
            }
            let function = &defs[0];
            let mut callee = Loops {
                functions: self.functions,
                module: function.module.clone(),
                owner: function.owner.clone(),
                types: Default::default(),
                recursive: false,
                active: self.active.clone(),
                max_depth: 0,
                depth: 0,
                findings: Vec::new(),
            };
            callee.active.push(key.into());
            callee.bind_signature(&function.sig);
            callee.visit_block(&function.block);
            if callee.recursive {
                self.recursive = true;
                let work_degree = self.depth + callee.max_depth;
                self.max_depth = self.max_depth.max(work_degree);
                self.findings.push(Finding {
                    line: span.start().line,
                    message: format!(
                        "call to {key} reaches recursion; potential {} or worse if recursion depth scales with N (heuristic, not a proven bound)",
                        order(work_degree + 1)
                    ),
                });
            } else if callee.max_depth > 0 {
                let degree = self.depth + callee.max_depth;
                self.max_depth = self.max_depth.max(degree);
                let name = key.rsplit("::").next().unwrap();
                self.findings.push(Finding {
                    line: span.start().line,
                    message: format!("call to {name} may grow as {}", order(degree)),
                });
            }
        }
        fn visit_callback(&mut self, expr: &syn::Expr) {
            if let syn::Expr::Closure(closure) = expr {
                self.visit_expr(&closure.body);
            } else {
                self.visit_expr(expr);
            }
        }
        fn visit_chain(&mut self, expr: &syn::Expr) {
            if let syn::Expr::Paren(p) = expr {
                self.visit_chain(&p.expr);
            } else if let syn::Expr::MethodCall(call) = expr {
                let expansion = usize::from(matches!(
                    call.method.to_string().as_str(),
                    "flat_map" | "flatten"
                ));
                self.depth -= expansion;
                self.visit_chain(&call.receiver);
                for arg in &call.args {
                    self.visit_callback(arg);
                }
                self.depth += expansion;
            } else {
                self.visit_expr(expr);
            }
        }
        fn enter(&mut self, span: proc_macro2::Span) {
            self.depth += 1;
            self.max_depth = self.max_depth.max(self.depth);
            self.findings.push(Finding {
                line: span.start().line,
                message: if self.depth == 1 {
                    "loop may grow as O(N)".into()
                } else {
                    format!(
                        "loop nesting depth is {}; work may grow as O(N^{})",
                        self.depth, self.depth
                    )
                },
            });
        }
    }
    impl<'ast> Visit<'ast> for Loops<'_> {
        fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
            if let syn::Expr::Path(path) = &*node.func
                && path.qself.is_none()
            {
                let mut name = symbols::path_name(&path.path);
                if name.starts_with("Self::")
                    && let Some(owner) = &self.owner
                {
                    name = format!("{}::{}", owner, &name[6..]);
                }
                let key = if self.functions.functions.contains_key(&name) {
                    Some(name.clone())
                } else {
                    self.functions.resolve(&self.module, &name)
                };
                if let Some(key) = key {
                    self.call(&key, node.paren_token.span.open());
                }
            }
            visit::visit_expr_call(self, node);
        }
        fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
            if matches!(
                node.method.to_string().as_str(),
                "collect"
                    | "sum"
                    | "product"
                    | "count"
                    | "for_each"
                    | "fold"
                    | "reduce"
                    | "any"
                    | "all"
                    | "find"
                    | "find_map"
                    | "position"
            ) && iterator_chain(&node.receiver)
            {
                let traversal = iterator_degree(&node.receiver);
                self.depth += traversal;
                self.max_depth = self.max_depth.max(self.depth);
                self.findings.push(Finding {
                    line: node.method.span().start().line,
                    message: format!("iterator consumption may grow as {}", order(self.depth)),
                });
                self.visit_chain(&node.receiver);
                for arg in &node.args {
                    self.visit_callback(arg);
                }
                self.depth -= traversal;
            } else {
                if let syn::Expr::Path(receiver) = &*node.receiver
                    && let Some(ident) = receiver.path.get_ident()
                    && let Some(owner) = self.types.get(&ident.to_string())
                {
                    let key = format!("{owner}::{}", node.method);
                    self.call(&key, node.method.span());
                }
                visit::visit_expr_method_call(self, node);
            }
        }

        fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
            let old = self.module.clone();
            self.module = format!("{}::{}", old, node.ident);
            visit::visit_item_mod(self, node);
            self.module = old;
        }
        fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
            let old = self.owner.clone();
            self.owner = type_name(&node.self_ty).map(|n| self.functions.qualify(&self.module, &n));
            visit::visit_item_impl(self, node);
            self.owner = old;
        }
        fn visit_block(&mut self, node: &'ast syn::Block) {
            let old = self.types.clone();
            visit::visit_block(self, node);
            self.types = old;
        }
        fn visit_local(&mut self, node: &'ast syn::Local) {
            visit::visit_local(self, node);
            match &node.pat {
                syn::Pat::Type(p) => {
                    if let syn::Pat::Ident(i) = &*p.pat {
                        self.types.remove(&i.ident.to_string());
                        if let Some(t) = type_name(&p.ty) {
                            self.types.insert(
                                i.ident.to_string(),
                                self.functions.qualify(&self.module, &t),
                            );
                        }
                    }
                }
                syn::Pat::Ident(i) => {
                    self.types.remove(&i.ident.to_string());
                }
                _ => {}
            }
        }
        // Deferred bodies are analyzed independently of their definition site.
        fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
            let outer = std::mem::replace(&mut self.depth, 0);
            let outer_max = self.max_depth;
            let outer_recursive = self.recursive;
            visit::visit_expr_closure(self, node);
            self.depth = outer;
            self.max_depth = outer_max;
            self.recursive = outer_recursive;
        }
        fn visit_expr_async(&mut self, node: &'ast syn::ExprAsync) {
            let outer = std::mem::replace(&mut self.depth, 0);
            let outer_max = self.max_depth;
            let outer_recursive = self.recursive;
            visit::visit_expr_async(self, node);
            self.depth = outer;
            self.max_depth = outer_max;
            self.recursive = outer_recursive;
        }
        fn visit_expr_repeat(&mut self, node: &'ast syn::ExprRepeat) {
            for attribute in &node.attrs {
                self.visit_attribute(attribute);
            }
            self.visit_expr(&node.expr);
            let outer = std::mem::replace(&mut self.depth, 0);
            let outer_max = self.max_depth;
            let outer_recursive = self.recursive;
            self.visit_expr(&node.len);
            self.depth = outer;
            self.max_depth = outer_max;
            self.recursive = outer_recursive;
        }
        fn visit_generic_argument(&mut self, node: &'ast syn::GenericArgument) {
            if let syn::GenericArgument::Const(expression) = node {
                let outer = std::mem::replace(&mut self.depth, 0);
                let outer_max = self.max_depth;
                let outer_recursive = self.recursive;
                self.visit_expr(expression);
                self.depth = outer;
                self.max_depth = outer_max;
                self.recursive = outer_recursive;
            } else {
                visit::visit_generic_argument(self, node);
            }
        }
        fn visit_type_array(&mut self, node: &'ast syn::TypeArray) {
            self.visit_type(&node.elem);
            let outer = std::mem::replace(&mut self.depth, 0);
            let outer_max = self.max_depth;
            let outer_recursive = self.recursive;
            self.visit_expr(&node.len);
            self.depth = outer;
            self.max_depth = outer_max;
            self.recursive = outer_recursive;
        }
        fn visit_item_const(&mut self, node: &'ast syn::ItemConst) {
            let outer = std::mem::replace(&mut self.depth, 0);
            let outer_max = self.max_depth;
            let outer_recursive = self.recursive;
            visit::visit_item_const(self, node);
            self.depth = outer;
            self.max_depth = outer_max;
            self.recursive = outer_recursive;
        }
        fn visit_item_static(&mut self, node: &'ast syn::ItemStatic) {
            let outer = std::mem::replace(&mut self.depth, 0);
            let outer_max = self.max_depth;
            let outer_recursive = self.recursive;
            visit::visit_item_static(self, node);
            self.depth = outer;
            self.max_depth = outer_max;
            self.recursive = outer_recursive;
        }
        fn visit_expr_const(&mut self, node: &'ast syn::ExprConst) {
            let outer = std::mem::replace(&mut self.depth, 0);
            let outer_max = self.max_depth;
            let outer_recursive = self.recursive;
            visit::visit_expr_const(self, node);
            self.depth = outer;
            self.max_depth = outer_max;
            self.recursive = outer_recursive;
        }
        fn visit_impl_item_const(&mut self, node: &'ast syn::ImplItemConst) {
            let outer = std::mem::replace(&mut self.depth, 0);
            let outer_max = self.max_depth;
            let outer_recursive = self.recursive;
            visit::visit_impl_item_const(self, node);
            self.depth = outer;
            self.max_depth = outer_max;
            self.recursive = outer_recursive;
        }
        fn visit_trait_item_const(&mut self, node: &'ast syn::TraitItemConst) {
            let outer = std::mem::replace(&mut self.depth, 0);
            let outer_max = self.max_depth;
            let outer_recursive = self.recursive;
            visit::visit_trait_item_const(self, node);
            self.depth = outer;
            self.max_depth = outer_max;
            self.recursive = outer_recursive;
        }
        fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
            let outer = std::mem::replace(&mut self.depth, 0);
            let outer_max = self.max_depth;
            let outer_recursive = self.recursive;
            let old_types = self.types.clone();
            self.bind_signature(&node.sig);
            visit::visit_impl_item_fn(self, node);
            self.types = old_types;
            self.depth = outer;
            self.max_depth = outer_max;
            self.recursive = outer_recursive;
        }
        fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
            let outer = std::mem::replace(&mut self.depth, 0);
            let outer_max = self.max_depth;
            let outer_recursive = self.recursive;
            visit::visit_trait_item_fn(self, node);
            self.depth = outer;
            self.max_depth = outer_max;
            self.recursive = outer_recursive;
        }
        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            let outer = std::mem::replace(&mut self.depth, 0);
            let outer_max = self.max_depth;
            let outer_recursive = self.recursive;
            let old_types = self.types.clone();
            self.bind_signature(&node.sig);
            visit::visit_item_fn(self, node);
            self.types = old_types;
            self.depth = outer;
            self.max_depth = outer_max;
            self.recursive = outer_recursive;
        }
        fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
            let extra = if iterator_chain(&node.expr) {
                iterator_degree(&node.expr) - 1
            } else {
                0
            };
            if iterator_chain(&node.expr) {
                self.depth += extra;
                self.enter(node.for_token.span);
                self.visit_chain(&node.expr);
            } else {
                self.visit_expr(&node.expr);
                self.enter(node.for_token.span);
            }
            self.visit_block(&node.body);
            self.depth -= 1 + extra;
        }
        fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
            self.enter(node.while_token.span);
            visit::visit_expr_while(self, node);
            self.depth -= 1;
        }
        fn visit_expr_loop(&mut self, node: &'ast syn::ExprLoop) {
            self.enter(node.loop_token.span);
            visit::visit_expr_loop(self, node);
            self.depth -= 1;
        }
    }
    let parsed = syn::parse_file(source);
    let local = parsed
        .as_ref()
        .map(symbols::Index::single)
        .unwrap_or_default();
    let (functions, module) = context.unwrap_or((&local, "root"));
    let mut visitor = Loops {
        functions,
        module: module.into(),
        owner: None,
        types: Default::default(),
        recursive: false,
        active: Vec::new(),
        max_depth: 0,
        depth: 0,
        findings: Vec::new(),
    };
    match &parsed {
        Ok(file) => visitor.visit_file(file),
        Err(error) => {
            let block = syn::parse_str::<syn::Block>(&format!("{{{source}}}"))
                .map_err(|_| error.clone())?;
            visitor.visit_block(&block);
        }
    }
    Ok(visitor.findings)
}

fn type_name(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Reference(r) => type_name(&r.elem),
        syn::Type::Path(p) if p.qself.is_none() => Some(symbols::path_name(&p.path)),
        _ => None,
    }
}
fn iterator_degree(expr: &syn::Expr) -> usize {
    match expr {
        syn::Expr::MethodCall(c) if iterator_chain(&c.receiver) => {
            iterator_degree(&c.receiver)
                + usize::from(matches!(
                    c.method.to_string().as_str(),
                    "flat_map" | "flatten"
                ))
        }
        syn::Expr::Paren(p) => iterator_degree(&p.expr),
        _ => 1,
    }
}

fn order(degree: usize) -> String {
    if degree == 1 {
        "O(N)".into()
    } else {
        format!("O(N^{degree})")
    }
}

// Only recognize explicit iterator origins; arbitrary methods named `count`
// or `collect` are not sufficient evidence of iteration.
fn iterator_chain(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::Range(_) => true,
        syn::Expr::MethodCall(call) => match call.method.to_string().as_str() {
            "iter" | "iter_mut" | "into_iter" => true,
            "flat_map" | "flatten" | "map" | "filter" | "filter_map" | "inspect" | "enumerate"
            | "take" | "skip" | "take_while" | "skip_while" | "copied" | "cloned" | "rev" => {
                iterator_chain(&call.receiver)
            }
            _ => false,
        },
        syn::Expr::Paren(expr) => iterator_chain(&expr.expr),
        _ => false,
    }
}

#[allow(dead_code)]
fn legacy_analyze(source: &str) -> Vec<Finding> {
    let tokens = tokenize(source);
    let mut findings = Vec::new();
    let mut blocks = Vec::<Block>::new();
    let mut pending_loop = false;
    for token in &tokens {
        if let TokenKind::Ident(word) = &token.kind
            && matches!(word.as_str(), "for" | "while" | "loop")
        {
            pending_loop = true;
        }
        match token.kind {
            TokenKind::OpenBrace => {
                let parent_depth = blocks.last().map_or(0, |block| block.loop_depth);
                let starts_loop = std::mem::take(&mut pending_loop);
                let loop_depth = parent_depth + usize::from(starts_loop);
                if starts_loop {
                    findings.push(Finding {
                        line: token.line,
                        message: if loop_depth == 1 {
                            "loop may grow as O(N)".to_owned()
                        } else {
                            format!(
                                "loop nesting depth is {loop_depth}; work may grow as O(N^{loop_depth})"
                            )
                        },
                    });
                }
                blocks.push(Block { loop_depth });
            }
            TokenKind::CloseBrace => {
                blocks.pop();
                pending_loop = false;
            }
            _ => {}
        }
    }
    findings
}

fn tokenize(source: &str) -> Vec<Token> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let (mut index, mut line) = (0, 1);
    while index < chars.len() {
        match chars[index] {
            '\n' => {
                line += 1;
                index += 1;
            }
            c if c.is_whitespace() => index += 1,
            '/' if chars.get(index + 1) == Some(&'/') => {
                index += 2;
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
            }
            '/' if chars.get(index + 1) == Some(&'*') => {
                index += 2;
                let mut depth = 1;
                while index < chars.len() && depth > 0 {
                    if chars[index] == '\n' {
                        line += 1;
                    }
                    if chars[index] == '/' && chars.get(index + 1) == Some(&'*') {
                        depth += 1;
                        index += 2;
                    } else if chars[index] == '*' && chars.get(index + 1) == Some(&'/') {
                        depth -= 1;
                        index += 2;
                    } else {
                        index += 1;
                    }
                }
            }
            'r' if raw_string_hashes(&chars, index).is_some() => {
                let hashes = raw_string_hashes(&chars, index).expect("checked above");
                skip_raw_string(&chars, &mut index, &mut line, hashes);
            }
            '"' => skip_quoted(&chars, &mut index, &mut line, '"'),
            '\'' if looks_like_char_literal(&chars, index) => {
                skip_quoted(&chars, &mut index, &mut line, '\'')
            }
            '{' => {
                tokens.push(Token {
                    kind: TokenKind::OpenBrace,
                    line,
                });
                index += 1;
            }
            '}' => {
                tokens.push(Token {
                    kind: TokenKind::CloseBrace,
                    line,
                });
                index += 1;
            }
            c if c == '_' || c.is_alphabetic() => {
                let start = index;
                index += 1;
                while index < chars.len() && (chars[index] == '_' || chars[index].is_alphanumeric())
                {
                    index += 1;
                }
                tokens.push(Token {
                    kind: TokenKind::Ident(chars[start..index].iter().collect()),
                    line,
                });
            }
            _ => {
                tokens.push(Token {
                    kind: TokenKind::Other,
                    line,
                });
                index += 1;
            }
        }
    }
    tokens
}

fn raw_string_hashes(chars: &[char], index: usize) -> Option<usize> {
    let mut cursor = index + 1;
    while chars.get(cursor) == Some(&'#') {
        cursor += 1;
    }
    (chars.get(cursor) == Some(&'"')).then_some(cursor - index - 1)
}

fn skip_raw_string(chars: &[char], index: &mut usize, line: &mut usize, hashes: usize) {
    *index += hashes + 2;
    while *index < chars.len() {
        if chars[*index] == '\n' {
            *line += 1;
        }
        if chars[*index] == '"'
            && (0..hashes).all(|offset| chars.get(*index + 1 + offset) == Some(&'#'))
        {
            *index += hashes + 1;
            break;
        }
        *index += 1;
    }
}

fn skip_quoted(chars: &[char], index: &mut usize, line: &mut usize, quote: char) {
    *index += 1;
    while *index < chars.len() {
        if chars[*index] == '\n' {
            *line += 1;
        }
        if chars[*index] == '\\' {
            *index += 2;
            continue;
        }
        let current = chars[*index];
        *index += 1;
        if current == quote {
            break;
        }
    }
}

fn looks_like_char_literal(chars: &[char], index: usize) -> bool {
    chars.get(index + 2) == Some(&'\'')
        || (chars.get(index + 1) == Some(&'\\') && chars.get(index + 3) == Some(&'\''))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_modules_aliases_and_typed_methods() {
        let findings = analyze(
            "mod work { pub fn scan(xs: &[u8]) { for _ in xs {} } } use crate::work::scan as run; struct Worker; impl Worker { fn scan(&self, xs: &[u8]) { work::scan(xs); } } fn caller(w: &Worker, xs: &[u8]) { for _ in xs { run(xs); w.scan(xs); } }",
        );
        assert!(
            findings
                .iter()
                .filter(|f| f.message == "call to scan may grow as O(N^2)")
                .count()
                >= 2
        );
    }

    #[test]
    fn flat_map_and_flatten_expand_consumed_cardinality() {
        for chain in ["xs.iter().flat_map(|_| xs.iter())", "xs.iter().flatten()"] {
            let findings = analyze(&format!("fn f() {{ {chain}.count(); }}"));
            assert!(
                findings
                    .iter()
                    .any(|f| f.message == "iterator consumption may grow as O(N^2)")
            );
            assert!(analyze(&format!("fn f() {{ let unused = {chain}; }}")).is_empty());
        }
        let findings =
            analyze("fn f() { for x in xs.iter().flat_map(|_| ys.iter()) { for y in zs {} } }");
        assert!(findings.iter().any(|f| f.message.contains("O(N^3)")));
    }

    #[test]
    fn recursive_risk_and_candidate_propagate_to_callers() {
        let findings =
            analyze("fn a() { for _ in xs { b(); } } fn b() { a(); } fn caller() { a(); }");
        assert!(findings.iter().any(|f| {
            f.message
                .contains("potential O(N^2) or worse if recursion depth scales with N")
        }));
        assert!(
            !findings
                .iter()
                .any(|f| f.message.starts_with("call to a may grow"))
        );
    }

    #[test]
    fn recursion_candidates_explain_assumptions() {
        let linear =
            analyze("fn walk(n: usize) { if n > 0 { walk(n - 1); } } fn caller() { walk(10); }");
        assert!(
            linear
                .iter()
                .any(|f| f.message.contains("potential O(N) or worse"))
        );
        assert!(
            !linear
                .iter()
                .any(|f| f.message.contains("potential O(N^2)"))
        );
        let quadratic = analyze(
            "fn walk(n: usize) { for _ in 0..n {} if n > 0 { walk(n - 1); } } fn caller() { walk(10); }",
        );
        assert!(
            quadratic
                .iter()
                .any(|f| f.message.contains("potential O(N^2) or worse"))
        );
        let halving = analyze("fn walk(n: usize) { if n > 1 { walk(n / 2); } }");
        assert!(
            halving
                .iter()
                .filter(|f| f.message.contains("potential"))
                .all(|f| f.message.contains("if recursion depth scales with N"))
        );
    }

    #[test]
    fn same_names_in_different_modules_do_not_collide() {
        let findings = analyze(
            "mod a { fn scan() { for _ in xs {} } fn run() { scan(); } } mod b { fn scan() {} fn run() { scan(); } }",
        );
        assert_eq!(
            findings
                .iter()
                .filter(|f| f.message.starts_with("call to scan"))
                .count(),
            1
        );
    }

    #[test]
    fn propagates_transitive_calls_and_iterator_callbacks() {
        let findings = analyze(
            "fn leaf(xs: &[u8]) { for _ in xs {} } fn mid(xs: &[u8]) { leaf(xs); } fn outer(xs: &[u8]) { xs.iter().map(|x| { mid(xs); x }).count(); }",
        );
        assert!(
            findings
                .iter()
                .any(|f| f.message == "call to leaf may grow as O(N)")
        );
        assert!(
            findings
                .iter()
                .any(|f| f.message == "call to mid may grow as O(N^2)")
        );
    }

    #[test]
    fn find_map_consumes_one_traversal_even_with_adaptors() {
        for chain in ["xs.iter()", "xs.iter().map(|x| x).filter(|x| **x > 0)"] {
            let findings = analyze(&format!(
                "fn lookup(xs: &[u8]) -> Option<u8> {{ {chain}.find_map(|x| (*x == 42).then_some(*x)) }}"
            ));
            assert_eq!(findings.len(), 1, "{chain}");
            assert_eq!(findings[0].message, "iterator consumption may grow as O(N)");
        }
    }

    #[test]
    fn find_map_composes_outer_loops_and_callback_work() {
        let outer =
            analyze("fn lookup(xs: &[u8]) { for _ in xs { xs.iter().find_map(|x| Some(*x)); } }");
        assert_eq!(outer.len(), 2);
        assert_eq!(outer[1].message, "iterator consumption may grow as O(N^2)");

        let callback_loop = analyze(
            "fn lookup(xs: &[u8]) { xs.iter().find_map(|x| { for _ in xs {} Some(*x) }); }",
        );
        assert_eq!(callback_loop.len(), 2);
        assert_eq!(
            callback_loop[0].message,
            "iterator consumption may grow as O(N)"
        );
        assert!(callback_loop[1].message.contains("O(N^2)"));

        let callback_call = analyze(
            "fn scan(xs: &[u8]) { for _ in xs {} } fn lookup(xs: &[u8]) { xs.iter().find_map(|x| { scan(xs); Some(*x) }); }",
        );
        assert_eq!(callback_call.len(), 3);
        assert_eq!(callback_call[2].message, "call to scan may grow as O(N^2)");
    }

    #[test]
    fn find_map_requires_an_iterator_origin() {
        assert!(analyze(
            "struct Lookup; impl Lookup { fn find_map(&self) -> Option<u8> { None } } fn lookup(value: &Lookup) { value.find_map(); }"
        ).is_empty());
    }

    #[test]
    fn iterator_adaptors_are_lazy_and_fused() {
        assert!(
            analyze("fn f(xs: &[u8]) { let _ = xs.iter().map(|x| x).filter(|x| true); }")
                .is_empty()
        );
        let findings =
            analyze("fn f(xs: &[u8]) { xs.iter().map(|x| x).filter(|x| true).count(); }");
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.ends_with("O(N)"));
        let findings =
            analyze("fn f(xs: &[u8]) { xs.iter().for_each(|_| { xs.iter().count(); }); }");
        assert!(findings.iter().any(|f| f.message.ends_with("O(N^2)")));
    }

    #[test]
    fn recursive_calls_terminate_and_unused_closures_do_not_inflate_calls() {
        analyze("fn a() { b(); } fn b() { a(); }");
        let findings = analyze(
            "fn helper(xs: &[u8]) { let unused = || { for _ in xs {} }; } fn caller(xs: &[u8]) { helper(xs); }",
        );
        assert!(!findings.iter().any(|f| f.message.starts_with("call to")));
    }

    #[test]
    fn array_lengths_and_const_arguments_have_independent_depth() {
        let value = "{ let mut i = 0; while i < 4 { i += 1; } i }";
        for statement in [
            format!("let _ = [0u8; {value}];"),
            format!("let _ = Marker::<{value}>;"),
            format!("let _: [u8; {value}];"),
        ] {
            let source = format!(
                "struct Marker<const N: usize>; fn outer(xs: &[u8]) {{ for _ in xs {{ {statement} for _ in xs {{}} }} }}"
            );
            let findings = analyze(&source);
            assert_eq!(findings.len(), 3, "{statement}");
            assert_eq!(findings[1].message, "loop may grow as O(N)", "{statement}");
            assert!(findings[2].message.contains("O(N^2)"), "{statement}");
        }
    }
    #[test]
    fn repeat_element_preserves_runtime_depth() {
        let findings = analyze(
            "fn outer(xs: &[u8]) { for _ in xs { let _ = [{ for _ in xs {} 0u8 }; 4]; for _ in xs {} } }",
        );
        assert_eq!(findings.len(), 3);
        assert!(findings[1].message.contains("O(N^2)"));
        assert!(findings[2].message.contains("O(N^2)"));
    }
    #[test]
    fn constant_evaluation_has_independent_depth() {
        let value = "{ let mut i = 0; while i < 4 { i += 1; } i }";
        for declaration in [
            format!("const VALUE: usize = {value};"),
            format!("static VALUE: usize = {value};"),
            format!("let _ = const {value};"),
            format!("struct Local; impl Local {{ const VALUE: usize = {value}; }}"),
            format!("trait Local {{ const VALUE: usize = {value}; }}"),
        ] {
            let source = format!(
                "fn outer(xs: &[u8]) {{ for _ in xs {{ {declaration} for _ in xs {{}} }} }}"
            );
            let findings = analyze(&source);
            assert_eq!(findings.len(), 3, "{declaration}");
            assert_eq!(
                findings[1].message, "loop may grow as O(N)",
                "{declaration}"
            );
            assert!(findings[2].message.contains("O(N^2)"), "{declaration}");
        }
    }
    #[test]
    fn methods_have_independent_depth_and_restore_outer_depth() {
        for item in [
            "struct Local; impl Local { fn run(ys: &[u8]) { for _ in ys {} } }",
            "trait Local { fn run(ys: &[u8]) { for _ in ys {} } }",
        ] {
            let source =
                format!("fn outer(xs: &[u8]) {{ for _ in xs {{ {item} for _ in xs {{}} }} }}");
            let findings = analyze(&source);
            assert_eq!(findings.len(), 3);
            assert_eq!(findings[0].message, "loop may grow as O(N)");
            assert_eq!(findings[1].message, "loop may grow as O(N)");
            assert!(findings[2].message.contains("O(N^2)"));
        }
    }
    #[test]
    fn local_functions_have_independent_loop_depth() {
        let findings = analyze(
            "fn f(xs: &[u8]) { for _ in xs { fn helper(ys: &[u8]) { for _ in ys {} } for _ in xs {} } }",
        );
        assert_eq!(findings.len(), 3);
        assert_eq!(findings[1].message, "loop may grow as O(N)");
        assert!(findings[2].message.contains("O(N^2)"));
    }
    #[test]
    fn issue_regressions() {
        let results = analyze(
            "struct A; impl Clone for A { fn clone(&self) -> Self { for i in 0..1 {} A } }",
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].message, "loop may grow as O(N)");
        assert!(analyze("fn call<F>(f: F) where F: for<'a> Fn(&'a str) { f(\"x\"); }").is_empty());
        let results = analyze(
            "fn f(v: &[Vec<i32>]) { for row in v.iter().filter(|r| { !r.is_empty() }) { for x in row {} } }",
        );
        assert_eq!(results.len(), 2);
        assert!(results[1].message.contains("O(N^2)"));
        assert!(analyze("fn f(x: bool) { let r#loop = 1; if x { let _ = r#loop; } }").is_empty());
        let source = r####"fn f() { let a = br"C:\"; let b = cr"x\"; for i in 0..10 {} }"####;
        assert_eq!(analyze(source).len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlink_cycles() {
        let directory = std::env::temp_dir().join(format!("nayuta-test-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("sample.rs"), "fn f() {}").unwrap();
        std::os::unix::fs::symlink(&directory, directory.join("cycle")).unwrap();
        let mut files = Vec::new();
        collect_rust_files(&directory, &mut files).unwrap();
        assert_eq!(files.len(), 1);
        fs::remove_file(directory.join("cycle")).unwrap();
        fs::remove_file(directory.join("sample.rs")).unwrap();
        fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn reports_linear_and_nested_loops() {
        let source = r#"for user in users {
            client.query(user.id);
            for order in user.orders { println!("{}", order); }
        }"#;
        let findings = analyze(source);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].message, "loop may grow as O(N)");
        assert!(findings[1].message.contains("O(N^2)"));
    }
    #[test]
    fn ignores_comments_strings_and_calls_outside_loops() {
        let source = r#"client.query(id);
            // for x in xs { db.fetch(x); }
            let text = "while true { db.get(id); }";"#;
        assert!(analyze(source).is_empty());
    }
    #[test]
    fn understands_nested_block_comments() {
        assert!(analyze("/* for x in xs { /* nested */ db.load(x); } */").is_empty());
    }

    #[test]
    fn ignores_raw_strings() {
        let source = r####"let sample = r###"for x in xs { db.fetch(x); }"###;"####;
        assert!(analyze(source).is_empty());
    }
}
