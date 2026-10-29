use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
mod memory;

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

    let mut count = 0;
    for file in &files {
        match fs::read_to_string(file) {
            Ok(source) => {
                let findings = match try_analyze(&source) {
                    Ok(findings) => findings,
                    Err(error) => {
                        eprintln!("nayuta: {}: parse error: {error}", file.display());
                        had_error = true;
                        continue;
                    }
                };
                for finding in findings {
                    println!(
                        "{}:{}: complexity: {}",
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

fn try_analyze(source: &str) -> syn::Result<Vec<Finding>> {
    use syn::visit::{self, Visit};
    struct Loops {
        depth: usize,
        findings: Vec<Finding>,
    }
    impl Loops {
        fn enter(&mut self, span: proc_macro2::Span) {
            self.depth += 1;
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
    impl<'ast> Visit<'ast> for Loops {
        fn visit_item_const(&mut self, node: &'ast syn::ItemConst) {
            let outer = std::mem::replace(&mut self.depth, 0);
            visit::visit_item_const(self, node);
            self.depth = outer;
        }
        fn visit_item_static(&mut self, node: &'ast syn::ItemStatic) {
            let outer = std::mem::replace(&mut self.depth, 0);
            visit::visit_item_static(self, node);
            self.depth = outer;
        }
        fn visit_expr_const(&mut self, node: &'ast syn::ExprConst) {
            let outer = std::mem::replace(&mut self.depth, 0);
            visit::visit_expr_const(self, node);
            self.depth = outer;
        }
        fn visit_impl_item_const(&mut self, node: &'ast syn::ImplItemConst) {
            let outer = std::mem::replace(&mut self.depth, 0);
            visit::visit_impl_item_const(self, node);
            self.depth = outer;
        }
        fn visit_trait_item_const(&mut self, node: &'ast syn::TraitItemConst) {
            let outer = std::mem::replace(&mut self.depth, 0);
            visit::visit_trait_item_const(self, node);
            self.depth = outer;
        }
        fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
            let outer = std::mem::replace(&mut self.depth, 0);
            visit::visit_impl_item_fn(self, node);
            self.depth = outer;
        }
        fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
            let outer = std::mem::replace(&mut self.depth, 0);
            visit::visit_trait_item_fn(self, node);
            self.depth = outer;
        }
        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            let outer = std::mem::replace(&mut self.depth, 0);
            visit::visit_item_fn(self, node);
            self.depth = outer;
        }
        fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
            self.visit_expr(&node.expr);
            self.enter(node.for_token.span);
            self.visit_block(&node.body);
            self.depth -= 1;
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
    let mut visitor = Loops {
        depth: 0,
        findings: Vec::new(),
    };
    match syn::parse_file(source) {
        Ok(file) => visitor.visit_file(&file),
        Err(error) => {
            let block =
                syn::parse_str::<syn::Block>(&format!("{{{source}}}")).map_err(|_| error)?;
            visitor.visit_block(&block);
        }
    }
    Ok(visitor.findings)
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
