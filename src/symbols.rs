//! Conservative syntax-only symbol index. Ambiguous definitions are never selected.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Function {
    pub sig: syn::Signature,
    pub block: syn::Block,
    pub module: String,
    pub owner: Option<String>,
}
#[derive(Default)]
pub struct Index {
    pub functions: HashMap<String, Vec<Function>>,
    pub modules: HashMap<PathBuf, String>,
    aliases: HashMap<String, String>,
}
impl Index {
    pub fn build(files: &[(PathBuf, syn::File)]) -> Self {
        let mut index = Self::default();
        // Roots before descendants. Module declarations determine identities;
        // unrelated binaries and crates retain separate root namespaces.
        let mut pending: Vec<_> = files.iter().collect();
        pending.sort_by_key(|(p, _)| {
            (
                p.components().count(),
                !matches!(
                    p.file_name().and_then(|s| s.to_str()),
                    Some("lib.rs" | "main.rs")
                ),
            )
        });
        for (path, file) in pending {
            if !index.modules.contains_key(path) {
                let root = format!("root{}", index.modules.len());
                index.add_file(path, file, &root, files);
            }
        }
        index
    }
    fn add_file(
        &mut self,
        path: &Path,
        file: &syn::File,
        module: &str,
        files: &[(PathBuf, syn::File)],
    ) {
        if self.modules.contains_key(path) {
            return;
        }
        self.modules.insert(path.to_owned(), module.into());
        let dir = if matches!(
            path.file_name().and_then(|s| s.to_str()),
            Some("lib.rs" | "main.rs" | "mod.rs")
        ) {
            path.parent().unwrap().to_owned()
        } else {
            path.with_extension("")
        };
        self.items(&file.items, module, &dir, files);
    }
    pub fn single(file: &syn::File) -> Self {
        let mut index = Self::default();
        index.items(&file.items, "root", Path::new(""), &[]);
        index
    }
    fn items(
        &mut self,
        items: &[syn::Item],
        module: &str,
        dir: &Path,
        files: &[(PathBuf, syn::File)],
    ) {
        for item in items {
            match item {
                syn::Item::Fn(f) => self.insert(module, None, &f.sig, &f.block),
                syn::Item::Impl(i) if i.trait_.is_none() => {
                    if let syn::Type::Path(t) = &*i.self_ty {
                        let owner = self.qualify(module, &path_name(&t.path));
                        for item in &i.items {
                            if let syn::ImplItem::Fn(f) = item {
                                self.insert(module, Some(owner.clone()), &f.sig, &f.block);
                            }
                        }
                    }
                }
                syn::Item::Mod(m) => {
                    let child = format!("{module}::{}", m.ident);
                    if let Some((_, items)) = &m.content {
                        self.items(items, &child, &dir.join(m.ident.to_string()), files);
                    } else {
                        let a = dir.join(format!("{}.rs", m.ident));
                        let b = dir.join(m.ident.to_string()).join("mod.rs");
                        if let Some((p, f)) = files.iter().find(|(p, _)| p == &a || p == &b) {
                            self.add_file(p, f, &child, files);
                        }
                    }
                }
                syn::Item::Use(u) => self.use_tree(module, "", &u.tree),
                _ => {}
            }
        }
    }
    fn use_tree(&mut self, module: &str, prefix: &str, tree: &syn::UseTree) {
        match tree {
            syn::UseTree::Path(p) => {
                self.use_tree(module, &format!("{prefix}{}::", p.ident), &p.tree)
            }
            syn::UseTree::Name(n) => {
                self.aliases.insert(
                    format!("{module}::{}", n.ident),
                    format!("{prefix}{}", n.ident),
                );
            }
            syn::UseTree::Rename(n) => {
                self.aliases.insert(
                    format!("{module}::{}", n.rename),
                    format!("{prefix}{}", n.ident),
                );
            }
            syn::UseTree::Group(g) => {
                for t in &g.items {
                    self.use_tree(module, prefix, t);
                }
            }
            _ => {}
        }
    }
    fn insert(
        &mut self,
        module: &str,
        owner: Option<String>,
        sig: &syn::Signature,
        block: &syn::Block,
    ) {
        let key = format!("{}::{}", owner.as_deref().unwrap_or(module), sig.ident);
        self.functions.entry(key).or_default().push(Function {
            sig: sig.clone(),
            block: block.clone(),
            module: module.into(),
            owner,
        });
    }
    pub fn qualify(&self, module: &str, name: &str) -> String {
        let mut base: Vec<_> = module.split("::").collect();
        let mut parts = name.split("::").peekable();
        match parts.peek().copied() {
            Some("crate") => {
                base.truncate(1);
                parts.next();
            }
            Some("self") => {
                parts.next();
            }
            _ => {}
        }
        while parts.peek() == Some(&"super") {
            base.pop();
            parts.next();
        }
        base.extend(parts);
        base.join("::")
    }
    pub fn resolve(&self, module: &str, name: &str) -> Option<String> {
        let (head, tail) = name.split_once("::").unwrap_or((name, ""));
        let name = if let Some(alias) = self.aliases.get(&format!("{module}::{head}")) {
            format!(
                "{alias}{}",
                if tail.is_empty() {
                    String::new()
                } else {
                    format!("::{tail}")
                }
            )
        } else {
            name.into()
        };
        let key = self.qualify(module, &name);
        self.functions
            .get(&key)
            .filter(|f| f.len() == 1)
            .map(|_| key)
    }
}
pub fn path_name(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}
