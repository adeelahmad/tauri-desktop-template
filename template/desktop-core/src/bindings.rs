//! Typed IPC: TypeScript types for every Tauri command, generated from the Rust code.
//!
//! The UI calls the backend with `invoke('search', { query })`. This module writes the
//! types of those calls, so the editor knows the arguments and the result of each command:
//!
//! - `types.d.ts`: every struct and enum the commands take or return (the settings
//!   [`Config`](crate::Config), [`Hit`](crate::Hit), the app's feature flags, ...), from
//!   their `#[derive(TS)]`;
//! - `commands.d.ts`: `Commands` (each command's `args` and `result`), plus `Invoke` (a typed
//!   `invoke`) and `Api` (one typed function per command), read from the app's
//!   `tauri::generate_handler![...]` list and the commands' signatures.
//!
//! The app's test `export_bindings` (in `src-tauri/src/lib.rs`) runs this:
//! `cargo test -p <app> export_bindings` writes the files into `ui-react/src/bindings/`
//! (or `ui/bindings/` for the plain UI). With `CI` set it only compares, and fails when the
//! files differ from what the Rust code says (stale bindings). See `docs/GUIDE.md`,
//! "Typed IPC".
//!
//! It all runs in tests (feature `ts`, which the app enables for its tests only), so the
//! app itself does not contain it.
//!
//! # Why ts-rs
//!
//! [ts-rs](https://docs.rs/ts-rs) is a derive and nothing else: no runtime, no change to how
//! commands are written or registered, and it follows the serde attributes the types already
//! have (`rename_all`, `default`, `tag`, `flatten`). specta with tauri-specta would also type
//! the commands, but it replaces `generate_handler!` with its own builder and is still a
//! release candidate for Tauri 2. Here the command list comes from the
//! `generate_handler!` the app already has, parsed with `syn` (already in the build through
//! serde and Tauri), so `kitchen generate command` and hand-written commands need nothing
//! extra.
//!
//! # Example
//!
//! ```
//! use desktop_core::bindings::{Bindings, TS};
//!
//! #[derive(serde::Serialize, TS)]
//! #[serde(rename_all = "camelCase")]
//! struct Greeting {
//!     text: String,
//!     word_count: usize,
//! }
//!
//! let lib_rs = r#"
//!     /// Say hello.
//!     #[tauri::command]
//!     fn greet(name: String, loud: Option<bool>) -> Result<Greeting, String> { todo!() }
//!
//!     #[cfg(feature = "extra")]
//!     #[tauri::command]
//!     fn extra() {}
//!
//!     fn run() {
//!         builder.invoke_handler(tauri::generate_handler![greet, #[cfg(feature = "extra")] extra]);
//!     }
//! "#;
//! let mut b = Bindings::new();
//! b.add::<Greeting>("Greeting");
//! let commands = b.commands(lib_rs, &|_| None, &|key, value| Some(key == "feature" && value == Some("off-by-default")))?;
//! assert_eq!(commands.len(), 1, "`extra` is behind a feature that is off");
//! let files = b.render(&commands, "cargo test export_bindings");
//! let (_, types) = files.iter().find(|(f, _)| f == "types.d.ts").unwrap();
//! assert!(types.contains("export type Greeting = { text: string, wordCount: number, };"));
//! let (_, cmds) = files.iter().find(|(f, _)| f == "commands.d.ts").unwrap();
//! assert!(cmds.contains("greet: { args: { name: string; loud?: boolean | null }; result: Greeting };"));
//! # Ok::<(), String>(())
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub use ts_rs::{self, TS};

use syn::punctuated::Punctuated;
use syn::visit::Visit;

/// Parameters Tauri fills in itself: they are not part of what the UI sends.
const INJECTED: &[&str] = &["AppHandle", "State", "Window", "WebviewWindow", "Webview", "Request", "Invoke"];

/// The types the commands use, and how to write them in TypeScript.
pub struct Bindings {
    cfg: ts_rs::Config,
    /// TypeScript name -> its declaration, with its docs.
    types: BTreeMap<String, String>,
    /// Rust paths as the code writes them (`paths::Profile`, `desktop_core::Hit`) -> TypeScript name.
    paths: Vec<(String, String)>,
    /// Two Rust types with the same TypeScript name.
    clashes: Vec<String>,
}

impl Default for Bindings {
    fn default() -> Self {
        Self::new()
    }
}

/// One Tauri command, as the UI sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    /// The name `invoke` takes.
    pub name: String,
    /// The doc comment, one entry per line.
    pub docs: Vec<String>,
    /// What the UI passes (Tauri's own parameters such as `AppHandle` left out).
    pub args: Vec<Arg>,
    /// What the promise resolves to (an `Err` rejects it).
    pub result: String,
}

/// One argument of a command.
#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    /// The key in the `invoke` arguments (camelCase, as Tauri expects by default).
    pub name: String,
    /// Its TypeScript type.
    pub ts: String,
    /// An `Option`: the key may be left out.
    pub optional: bool,
}

impl Bindings {
    /// No types yet, except `JsonValue` (for `serde_json::Value`). Large integers (`u64`,
    /// `i64`) are `number`, as serde_json sends them.
    pub fn new() -> Self {
        let mut b = Self { cfg: ts_rs::Config::new().with_large_int("number"), types: BTreeMap::new(), paths: Vec::new(), clashes: Vec::new() };
        b.add::<serde_json::Value>("serde_json::Value");
        b.add::<serde_json::Value>("Value");
        b
    }

    /// Add a type the commands use, under the path the code writes it with (`Hit`,
    /// `desktop_core::Hit`, `tray::Status`), with every type it contains.
    pub fn add<T: TS + 'static + ?Sized>(&mut self, rust_path: &str) -> &mut Self {
        let name = T::ident(&self.cfg);
        self.paths.push((rust_path.to_string(), name));
        self.declare::<T>();
        self
    }

    fn declare<T: TS + 'static + ?Sized>(&mut self) {
        if T::output_path().is_none() {
            // A primitive or a wrapper (`Vec<T>`, `Option<T>`): only the types inside it.
            struct Inner<'a>(&'a mut Bindings);
            impl ts_rs::TypeVisitor for Inner<'_> {
                fn visit<U: TS + 'static + ?Sized>(&mut self) {
                    self.0.declare::<U>();
                }
            }
            T::visit_generics(&mut Inner(self));
            return;
        }
        let name = T::ident(&self.cfg);
        let mut decl = T::docs().unwrap_or_default();
        decl.push_str("export ");
        decl.push_str(&T::decl(&self.cfg));
        match self.types.get(&name) {
            // The same type (or a stand-in for it, like `serde_json::Value` for `JsonValue`).
            Some(old) if *old == decl => return,
            Some(_) => {
                let clash = format!("two Rust types are called `{name}` in TypeScript: give one #[ts(rename = \"...\")]");
                if !self.clashes.contains(&clash) {
                    self.clashes.push(clash);
                }
                return;
            }
            None => {}
        }
        self.types.insert(name, decl);
        struct Deps<'a>(&'a mut Bindings);
        impl ts_rs::TypeVisitor for Deps<'_> {
            fn visit<U: TS + 'static + ?Sized>(&mut self) {
                self.0.declare::<U>();
            }
        }
        T::visit_dependencies(&mut Deps(self));
        T::visit_generics(&mut Deps(self));
    }

    /// The commands in `lib_rs`'s `generate_handler![...]`, in that order, typed.
    ///
    /// `module` returns the source of another module (`"shell"` for `shell::open_settings`),
    /// and `cfg` says whether a `#[cfg(...)]` predicate holds: `("feature", Some("ai"))`,
    /// `("desktop", None)`; `None` for one it does not know, which is an error.
    pub fn commands(&self, lib_rs: &str, module: &dyn Fn(&str) -> Option<String>, cfg: &dyn Fn(&str, Option<&str>) -> Option<bool>) -> Result<Vec<Command>, String> {
        let lib = syn::parse_file(lib_rs).map_err(|e| format!("lib.rs: {e}"))?;
        let mut finder = HandlerList(None);
        finder.visit_file(&lib);
        let tokens = finder.0.ok_or("no tauri::generate_handler![...] in lib.rs")?;
        let entries = syn::parse::Parser::parse2(Punctuated::<Entry, syn::Token![,]>::parse_terminated, tokens).map_err(|e| format!("generate_handler!: {e}"))?;
        let mut files: BTreeMap<String, syn::File> = BTreeMap::new();
        let mut out = Vec::new();
        for entry in entries {
            if !all_hold(&entry.attrs, cfg)? {
                continue;
            }
            let segs: Vec<String> = entry.path.segments.iter().map(|s| s.ident.to_string()).collect();
            let (name, mods) = segs.split_last().ok_or("empty path in generate_handler!")?;
            let mods: Vec<&str> = mods.iter().map(String::as_str).filter(|m| *m != "crate").collect();
            let module_path = mods.join("::");
            if !files.contains_key(&module_path) {
                let src = if module_path.is_empty() { Some(lib_rs.to_string()) } else { module(&module_path) };
                let src = src.ok_or_else(|| format!("command {name}: cannot read module {module_path}"))?;
                files.insert(module_path.clone(), syn::parse_file(&src).map_err(|e| format!("{module_path}: {e}"))?);
            }
            let file = &files[&module_path];
            let mut found = None;
            for item in &file.items {
                if let syn::Item::Fn(f) = item {
                    if f.sig.ident == name && f.attrs.iter().any(is_command) && all_hold(&f.attrs, cfg)? {
                        found = Some(f);
                        break;
                    }
                }
            }
            let f = found.ok_or_else(|| format!("command {name}: no #[tauri::command] fn {name} in {}", if module_path.is_empty() { "lib.rs" } else { &module_path }))?;
            out.push(self.command(f, &module_path)?);
        }
        if let Some(c) = self.clashes.first() {
            return Err(c.clone());
        }
        Ok(out)
    }

    fn command(&self, f: &syn::ItemFn, module: &str) -> Result<Command, String> {
        let name = f.sig.ident.to_string();
        let snake = f.attrs.iter().filter(|a| is_command(a)).any(|a| a.meta.require_list().is_ok_and(|l| l.tokens.to_string().replace(' ', "").contains("rename_all=\"snake_case\"")));
        let mut args = Vec::new();
        for input in &f.sig.inputs {
            let syn::FnArg::Typed(arg) = input else { return Err(format!("command {name}: methods cannot be commands")) };
            if last_ident(&arg.ty).is_some_and(|t| INJECTED.contains(&t.as_str())) {
                continue;
            }
            let syn::Pat::Ident(ident) = &*arg.pat else { return Err(format!("command {name}: use a plain name for each argument")) };
            let rust = ident.ident.to_string();
            let key = if snake { rust } else { camel(rust.trim_start_matches('_')) };
            let (optional, ty) = match option_inner(&arg.ty) {
                Some(inner) => (true, inner),
                None => (false, &*arg.ty),
            };
            let mut ts = self.ts(ty, module).map_err(|e| format!("command {name}, argument {key}: {e}"))?;
            if optional {
                ts.push_str(" | null");
            }
            args.push(Arg { name: key, ts, optional });
        }
        let result = match &f.sig.output {
            syn::ReturnType::Default => "null".to_string(),
            syn::ReturnType::Type(_, ty) => self.ts(result_ok(ty), module).map_err(|e| format!("command {name}, result: {e}"))?,
        };
        Ok(Command { name, docs: docs(&f.attrs), args, result })
    }

    /// A Rust type as TypeScript.
    fn ts(&self, ty: &syn::Type, module: &str) -> Result<String, String> {
        match ty {
            syn::Type::Reference(r) => self.ts(&r.elem, module),
            syn::Type::Paren(p) => self.ts(&p.elem, module),
            syn::Type::Group(g) => self.ts(&g.elem, module),
            syn::Type::Tuple(t) if t.elems.is_empty() => Ok("null".into()),
            syn::Type::Tuple(t) => Ok(format!("[{}]", t.elems.iter().map(|e| self.ts(e, module)).collect::<Result<Vec<_>, _>>()?.join(", "))),
            syn::Type::Slice(s) => Ok(format!("Array<{}>", self.ts(&s.elem, module)?)),
            syn::Type::Array(a) => Ok(format!("Array<{}>", self.ts(&a.elem, module)?)),
            syn::Type::Path(p) => self.path(&p.path, module),
            other => Err(format!("cannot type `{}` (use a named type)", quote_ty(other))),
        }
    }

    fn path(&self, path: &syn::Path, module: &str) -> Result<String, String> {
        let last = path.segments.last().ok_or("empty type path")?;
        let generic = |i: usize| -> Result<String, String> {
            let syn::PathArguments::AngleBracketed(a) = &last.arguments else { return Err(format!("`{}` needs its type parameters", last.ident)) };
            let ty = a.args.iter().filter_map(|g| if let syn::GenericArgument::Type(t) = g { Some(t) } else { None }).nth(i).ok_or_else(|| format!("`{}` needs its type parameters", last.ident))?;
            self.ts(ty, module)
        };
        let ident = last.ident.to_string();
        Ok(match ident.as_str() {
            "String" | "str" | "PathBuf" | "Path" | "char" | "OsString" => "string".into(),
            "bool" => "boolean".into(),
            "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "f32" | "f64" => "number".into(),
            "Option" => format!("{} | null", generic(0)?),
            "Vec" | "VecDeque" | "HashSet" | "BTreeSet" => format!("Array<{}>", generic(0)?),
            "HashMap" | "BTreeMap" => format!("Partial<Record<{}, {}>>", generic(0)?, generic(1)?),
            "Box" | "Arc" | "Rc" | "Cow" => generic(0)?,
            "Result" => generic(0)?,
            _ => self.named(path, module).ok_or_else(|| {
                let p = path.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::");
                format!("`{p}` has no TypeScript type: derive ts_rs::TS on it (#[cfg_attr(test, derive(ts_rs::TS))]) and add it in export_bindings")
            })?,
        })
    }

    /// A registered type, looked up as `<module>::<path>`, then as written, then by its last
    /// segment when that is unique.
    fn named(&self, path: &syn::Path, module: &str) -> Option<String> {
        let segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        let written = segs.join("::");
        let bare = segs.iter().skip_while(|s| matches!(s.as_str(), "crate" | "super" | "self")).cloned().collect::<Vec<_>>().join("::");
        let mut tries = Vec::new();
        if !module.is_empty() {
            tries.push(format!("{module}::{bare}"));
        }
        tries.push(written);
        tries.push(bare);
        for t in &tries {
            if let Some((_, ts)) = self.paths.iter().find(|(p, _)| p == t) {
                return Some(ts.clone());
            }
        }
        let last = segs.last()?;
        let mut same = self.paths.iter().filter(|(p, _)| p.rsplit("::").next() == Some(last.as_str())).map(|(_, ts)| ts).collect::<BTreeSet<_>>();
        match (same.pop_first(), same.is_empty()) {
            (Some(ts), true) => Some(ts.clone()),
            _ => None,
        }
    }

    /// `types.d.ts` and `commands.d.ts` for these commands. `how` is the command that
    /// regenerates them, named in their header.
    pub fn render(&self, commands: &[Command], how: &str) -> Vec<(String, String)> {
        let header = format!("// Generated from the Rust code by `{how}`. Do not edit: change the Rust\n// code and run that again (docs/GUIDE.md, \"Typed IPC\").\n");
        let mut types = header.clone();
        for decl in self.types.values() {
            types.push('\n');
            types.push_str(decl);
            types.push('\n');
        }
        let used: BTreeSet<&str> = self
            .types
            .keys()
            .map(String::as_str)
            .filter(|t| commands.iter().any(|c| mentions(&c.result, t) || c.args.iter().any(|a| mentions(&a.ts, t))))
            .collect();
        let mut cmds = header;
        if !used.is_empty() {
            cmds.push_str(&format!("import type {} {} {} from './types';\n", '{', used.into_iter().collect::<Vec<_>>().join(", "), '}'));
        }
        cmds.push_str("\n/** Every Tauri command: what it takes (`args`) and what its promise resolves to (`result`; an error rejects it). */\nexport interface Commands {\n");
        for c in commands {
            if !c.docs.is_empty() {
                cmds.push_str("  /**\n");
                for line in &c.docs {
                    cmds.push_str(&format!("   *{}{}\n", if line.is_empty() { "" } else { " " }, line.replace("*/", "*\\/")));
                }
                cmds.push_str("   */\n");
            }
            let args = if c.args.is_empty() {
                "Record<string, never>".to_string()
            } else {
                let fields: Vec<String> = c.args.iter().map(|a| format!("{}{}: {}", a.name, if a.optional { "?" } else { "" }, a.ts)).collect();
                format!("{} {} {}", '{', fields.join("; "), '}')
            };
            cmds.push_str(&format!("  {}: {} args: {}; result: {} {};\n", c.name, '{', args, c.result, '}'));
        }
        cmds.push_str("}\n\n");
        cmds.push_str(
            "/** The arguments of command `K`, as a parameter list: optional when every key is. */\n\
             export type ArgsOf<K extends keyof Commands> = {} extends Commands[K]['args'] ? [args?: Commands[K]['args']] : [args: Commands[K]['args']];\n\n\
             /** `invoke` typed by command name: `invoke('search', { query })` resolves to `Hit[]`. */\n\
             export type Invoke = <K extends keyof Commands>(cmd: K, ...args: ArgsOf<K>) => Promise<Commands[K]['result']>;\n\n\
             /** One typed function per command: `api.search({ query })`. */\n\
             export type Api = { [K in keyof Commands]: (...args: ArgsOf<K>) => Promise<Commands[K]['result']> };\n",
        );
        vec![("types.d.ts".into(), types), ("commands.d.ts".into(), cmds)]
    }
}

/// Whether to only compare the bindings (and fail when they are stale) instead of writing
/// them: `KITCHEN_BINDINGS=check` or `=write` decides, else checking when `CI` is set.
pub fn check_mode() -> bool {
    match std::env::var("KITCHEN_BINDINGS").as_deref() {
        Ok("check") => true,
        Ok("write") => false,
        _ => std::env::var_os("CI").is_some_and(|v| !v.is_empty() && v != "false"),
    }
}

/// Write `files` into `dir`, or with `check`, compare them and fail if any differs from what
/// is there (a missing file is written either way, so a new app gets its first bindings).
/// `how` is the command that rewrites them, for the error. Returns the files that were written.
pub fn sync(dir: &Path, files: &[(String, String)], check: bool, how: &str) -> Result<Vec<PathBuf>, String> {
    let mut written = Vec::new();
    let mut stale = Vec::new();
    for (name, text) in files {
        let path = dir.join(name);
        match std::fs::read_to_string(&path) {
            Ok(old) if old.replace("\r\n", "\n") == *text => {}
            Ok(_) if check => stale.push(path.display().to_string()),
            _ => {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
                written.push(path);
            }
        }
    }
    if stale.is_empty() {
        Ok(written)
    } else {
        Err(format!("stale TypeScript bindings (the Rust commands or types changed): {}. Run `KITCHEN_BINDINGS=write {how}` and commit the result", stale.join(", ")))
    }
}

/// `name` appears in `ts` as a whole word.
fn mentions(ts: &str, name: &str) -> bool {
    ts.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == name)
}

/// The entries of `generate_handler![...]`: `#[cfg(...)] module::command`.
struct Entry {
    attrs: Vec<syn::Attribute>,
    path: syn::Path,
}

impl syn::parse::Parse for Entry {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Entry { attrs: input.call(syn::Attribute::parse_outer)?, path: input.parse()? })
    }
}

struct HandlerList(Option<proc_macro2::TokenStream>);

impl<'ast> Visit<'ast> for HandlerList {
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        if m.path.segments.last().is_some_and(|s| s.ident == "generate_handler") {
            self.0 = Some(m.tokens.clone());
        }
    }
}

fn is_command(a: &syn::Attribute) -> bool {
    a.path().segments.last().is_some_and(|s| s.ident == "command")
}

/// Every `#[cfg(...)]` among `attrs` holds.
fn all_hold(attrs: &[syn::Attribute], cfg: &dyn Fn(&str, Option<&str>) -> Option<bool>) -> Result<bool, String> {
    for a in attrs.iter().filter(|a| a.path().is_ident("cfg")) {
        let pred: syn::Meta = a.parse_args().map_err(|e| format!("#[cfg]: {e}"))?;
        if !holds(&pred, cfg)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn holds(pred: &syn::Meta, cfg: &dyn Fn(&str, Option<&str>) -> Option<bool>) -> Result<bool, String> {
    let key = |p: &syn::Path| p.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::");
    match pred {
        syn::Meta::Path(p) => cfg(&key(p), None).ok_or_else(|| format!("unknown cfg `{}`", key(p))),
        syn::Meta::NameValue(nv) => {
            let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) = &nv.value else { return Err(format!("cfg `{}` needs a string", key(&nv.path))) };
            cfg(&key(&nv.path), Some(&s.value())).ok_or_else(|| format!("unknown cfg `{} = {:?}`: add it to the cfg list in export_bindings", key(&nv.path), s.value()))
        }
        syn::Meta::List(l) => {
            let inner = l.parse_args_with(Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated).map_err(|e| format!("#[cfg]: {e}"))?;
            let vals = inner.iter().map(|m| holds(m, cfg)).collect::<Result<Vec<_>, _>>()?;
            match key(&l.path).as_str() {
                "all" => Ok(vals.iter().all(|v| *v)),
                "any" => Ok(vals.iter().any(|v| *v)),
                "not" if vals.len() == 1 => Ok(!vals[0]),
                other => Err(format!("unknown cfg operator `{other}`")),
            }
        }
    }
}

fn last_ident(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        syn::Type::Reference(r) => last_ident(&r.elem),
        _ => None,
    }
}

fn first_generic<'a>(ty: &'a syn::Type, outer: &str) -> Option<&'a syn::Type> {
    let syn::Type::Path(p) = ty else { return None };
    let last = p.path.segments.last()?;
    if last.ident != outer {
        return None;
    }
    let syn::PathArguments::AngleBracketed(a) = &last.arguments else { return None };
    a.args.iter().find_map(|g| if let syn::GenericArgument::Type(t) = g { Some(t) } else { None })
}

fn option_inner(ty: &syn::Type) -> Option<&syn::Type> {
    first_generic(ty, "Option")
}

fn result_ok(ty: &syn::Type) -> &syn::Type {
    first_generic(ty, "Result").unwrap_or(ty)
}

fn camel(s: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in s.chars() {
        if c == '_' {
            up = !out.is_empty();
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn docs(attrs: &[syn::Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter(|a| a.path().is_ident("doc"))
        .filter_map(|a| match &a.meta {
            syn::Meta::NameValue(syn::MetaNameValue { value: syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }), .. }) => Some(s.value().strip_prefix(' ').map(String::from).unwrap_or_else(|| s.value())),
            _ => None,
        })
        .collect()
}

fn quote_ty(ty: &syn::Type) -> String {
    use quote::ToTokens;
    ty.to_token_stream().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, TS)]
    #[serde(rename_all = "camelCase")]
    struct Inner {
        size_mb: u64,
    }

    /// An outer type.
    #[derive(serde::Serialize, TS)]
    struct Outer {
        inner: Vec<Inner>,
        extra: Option<serde_json::Value>,
    }

    const LIB: &str = r#"
        use thing::Outer;
        mod shell;
        /// Find things.
        ///
        /// Second paragraph.
        #[tauri::command(async)]
        fn find(app: tauri::AppHandle, state: tauri::State<AppState>, query_text: String, limit: Option<usize>) -> Result<Vec<Outer>, String> { todo!() }
        #[tauri::command(rename_all = "snake_case")]
        fn raw(some_key: bool) {}
        #[cfg(not(feature = "ai"))]
        #[tauri::command]
        fn ask() -> u8 { 0 }
        #[cfg(feature = "ai")]
        #[tauri::command]
        fn ask() -> String { String::new() }
        fn run() {
            b.invoke_handler(wrap(tauri::generate_handler![
                find,
                raw,
                #[cfg(any(feature = "ai", feature = "nothing"))]
                ask,
                #[cfg(all(desktop, feature = "nothing"))]
                gone,
                shell::open,
            ]));
        }
    "#;

    fn on(key: &str, value: Option<&str>) -> Option<bool> {
        match (key, value) {
            ("desktop", None) => Some(true),
            ("feature", Some(f)) => Some(f == "ai"),
            _ => None,
        }
    }

    fn module(m: &str) -> Option<String> {
        (m == "shell").then(|| "#[tauri::command] pub fn open(window: tauri::Window, map: std::collections::HashMap<String, u32>) -> Result<(), String> { Ok(()) }".to_string())
    }

    #[test]
    fn commands_follow_the_handler_list_and_its_cfgs() {
        let mut b = Bindings::new();
        b.add::<Outer>("thing::Outer");
        let c = b.commands(LIB, &module, &on).unwrap();
        let names: Vec<&str> = c.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["find", "raw", "ask", "open"]);
        assert_eq!(c[0].docs, ["Find things.", "", "Second paragraph."]);
        assert_eq!(c[0].args, [Arg { name: "queryText".into(), ts: "string".into(), optional: false }, Arg { name: "limit".into(), ts: "number | null".into(), optional: true }]);
        assert_eq!(c[0].result, "Array<Outer>");
        assert_eq!(c[1].args[0].name, "some_key", "rename_all = snake_case keeps the Rust name");
        assert_eq!(c[1].result, "null");
        assert_eq!(c[2].result, "string", "the `ask` whose cfg holds");
        assert_eq!(c[3].args[0].ts, "Partial<Record<string, number>>");
        assert_eq!(c[3].result, "null");
    }

    #[test]
    fn render_declares_every_type_once_and_imports_what_commands_use() {
        let mut b = Bindings::new();
        b.add::<Outer>("Outer");
        let c = b.commands(LIB, &module, &on).unwrap();
        let files = b.render(&c, "cargo test export_bindings");
        let types = &files[0].1;
        assert!(types.contains("export type Inner = { sizeMb: number, };"), "{types}");
        assert!(types.contains("/**\n * An outer type.\n */\nexport type Outer = { inner: Array<Inner>, extra: JsonValue | null, };"), "{types}");
        assert_eq!(types.matches("export type JsonValue").count(), 1);
        let cmds = &files[1].1;
        assert!(cmds.contains("import type { Outer } from './types';"), "{cmds}");
        assert!(cmds.contains("  /**\n   * Find things.\n   *\n   * Second paragraph.\n   */\n  find: { args: { queryText: string; limit?: number | null }; result: Array<Outer> };"), "{cmds}");
        assert!(cmds.contains("  raw: { args: { some_key: boolean }; result: null };"));
        let braces = concat!("{", "{");
        assert!(!types.contains(braces) && !cmds.contains(braces), "no template braces in generated files");
    }

    #[test]
    fn unknown_types_and_cfgs_are_errors_that_say_what_to_do() {
        let b = Bindings::new();
        let e = b.commands(LIB, &module, &on).unwrap_err();
        assert!(e.contains("`Outer` has no TypeScript type"), "{e}");
        let e = b.commands("fn run() { tauri::generate_handler![#[cfg(windows)] x] }", &module, &on).unwrap_err();
        assert!(e.contains("unknown cfg `windows`"), "{e}");
        let e = b.commands("fn run() {}", &module, &on).unwrap_err();
        assert!(e.contains("no tauri::generate_handler!"), "{e}");
    }

    #[test]
    fn same_name_different_types_clash() {
        mod a {
            #[allow(dead_code)]
            #[derive(ts_rs::TS)]
            pub struct Status(pub u8);
        }
        mod b {
            #[allow(dead_code)]
            #[derive(ts_rs::TS)]
            pub struct Status(pub String);
        }
        let mut bind = Bindings::new();
        bind.add::<a::Status>("a::Status").add::<b::Status>("b::Status");
        let e = bind.commands("fn run() { tauri::generate_handler![] }", &|_| None, &on).unwrap_err();
        assert!(e.contains("two Rust types are called `Status`"), "{e}");
    }

    #[test]
    fn sync_writes_or_reports_stale_files() {
        let dir = std::env::temp_dir().join(format!("bindings-sync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files = vec![("a.d.ts".to_string(), "one\n".to_string())];
        assert_eq!(sync(&dir, &files, true, "x").unwrap().len(), 1, "a missing file is written even when checking");
        assert!(sync(&dir, &files, true, "x").unwrap().is_empty(), "unchanged");
        let changed = vec![("a.d.ts".to_string(), "two\n".to_string())];
        assert!(sync(&dir, &changed, true, "x").unwrap_err().contains("stale"));
        assert_eq!(sync(&dir, &changed, false, "x").unwrap().len(), 1);
        assert_eq!(std::fs::read_to_string(dir.join("a.d.ts")).unwrap(), "two\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn camel_case_like_tauri() {
        assert_eq!(camel("delete_data"), "deleteData");
        assert_eq!(camel("k"), "k");
        assert_eq!(camel("a_b_c"), "aBC");
    }
}
