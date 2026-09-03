//! Rust AST symbol extraction for Phase II semantic clustering.
//!
//! Deliberately narrow: this locates top-level (and impl-method) symbols and
//! a superset of the identifiers each one's body mentions. It does not
//! resolve types, macros, or imports — that would require a real compiler
//! front-end. The identifier superset is a conservative, honest signal
//! ("this symbol's text mentions that name"), not a claim of verified call
//! graph or type relationships. Unsupported/unparseable input degrades to an
//! empty symbol list rather than a guess (see `ScrawnyError::ParseFailure`
//! being surfaced by the caller, never swallowed as "no symbols here").

use scrawny_core::{Range, ScrawnyError};
use serde::{Deserialize, Serialize};
use std::path::Path;
use syn::spanned::Spanned;
use syn::visit::Visit;

pub mod attribution;
pub use attribution::{tag_hunk_lines, TaggedLine};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Impl,
    Const,
    Static,
    TypeAlias,
    Module,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Impl => "impl",
            SymbolKind::Const => "const",
            SymbolKind::Static => "static",
            SymbolKind::TypeAlias => "type",
            SymbolKind::Module => "module",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub range: Range,
    /// Every identifier this symbol's definition mentions (its own name
    /// included) — a superset used to find candidate dependency edges, not
    /// a resolved reference list.
    pub references: Vec<String>,
}

/// Parse a Rust source file's top-level items (and impl-block methods) into
/// `Symbol`s. Returns `ScrawnyError::ParseFailure` on invalid syntax — the
/// caller decides how to degrade (see scrawny-analysis's clustering, which
/// falls back to file-level grouping for any file it can't get symbols for).
pub fn parse_rust_symbols(path: &Path, source: &str) -> Result<Vec<Symbol>, ScrawnyError> {
    let file = syn::parse_file(source).map_err(|e| ScrawnyError::ParseFailure {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;

    let mut symbols = Vec::new();
    for item in &file.items {
        collect_item(item, None, &mut symbols);
    }
    Ok(symbols)
}

fn collect_item(item: &syn::Item, scope: Option<&str>, out: &mut Vec<Symbol>) {
    match item {
        syn::Item::Fn(f) => out.push(symbol_from(
            qualify(scope, &f.sig.ident.to_string()),
            SymbolKind::Function,
            item,
            f,
        )),
        syn::Item::Struct(s) => out.push(symbol_from(
            qualify(scope, &s.ident.to_string()),
            SymbolKind::Struct,
            item,
            s,
        )),
        syn::Item::Enum(e) => out.push(symbol_from(
            qualify(scope, &e.ident.to_string()),
            SymbolKind::Enum,
            item,
            e,
        )),
        syn::Item::Trait(t) => out.push(symbol_from(
            qualify(scope, &t.ident.to_string()),
            SymbolKind::Trait,
            item,
            t,
        )),
        syn::Item::Const(c) => out.push(symbol_from(
            qualify(scope, &c.ident.to_string()),
            SymbolKind::Const,
            item,
            c,
        )),
        syn::Item::Static(s) => out.push(symbol_from(
            qualify(scope, &s.ident.to_string()),
            SymbolKind::Static,
            item,
            s,
        )),
        syn::Item::Type(t) => out.push(symbol_from(
            qualify(scope, &t.ident.to_string()),
            SymbolKind::TypeAlias,
            item,
            t,
        )),
        syn::Item::Mod(m) => {
            let name = qualify(scope, &m.ident.to_string());
            out.push(symbol_from(name.clone(), SymbolKind::Module, item, m));
            if let Some((_, items)) = &m.content {
                for inner in items {
                    collect_item(inner, Some(&name), out);
                }
            }
        }
        syn::Item::Impl(imp) => {
            let type_name = type_name_of(&imp.self_ty);
            out.push(symbol_from(
                qualify(scope, &type_name),
                SymbolKind::Impl,
                item,
                imp,
            ));
            for impl_item in &imp.items {
                if let syn::ImplItem::Fn(method) = impl_item {
                    out.push(symbol_from(
                        qualify(scope, &format!("{type_name}::{}", method.sig.ident)),
                        SymbolKind::Method,
                        impl_item,
                        method,
                    ));
                }
            }
        }
        _ => {}
    }
}

fn qualify(scope: Option<&str>, name: &str) -> String {
    match scope {
        Some(s) => format!("{s}::{name}"),
        None => name.to_string(),
    }
}

fn type_name_of(ty: &syn::Type) -> String {
    if let syn::Type::Path(p) = ty {
        if let Some(segment) = p.path.segments.last() {
            return segment.ident.to_string();
        }
    }
    quote_like(ty)
}

/// Fallback stringification for a type we don't have a specific case for —
/// used only as a display label, never parsed back.
fn quote_like(ty: &syn::Type) -> String {
    format!("{:?}", ty).chars().take(40).collect()
}

fn symbol_from<S: Spanned, N: ToIdents>(name: String, kind: SymbolKind, spanned: &S, node: &N) -> Symbol {
    let span = spanned.span();
    let start = span.start().line.max(1);
    let end = span.end().line.max(start);
    Symbol {
        name,
        kind,
        range: Range::new(start, end),
        references: collect_idents(node),
    }
}

/// Every identifier mentioned anywhere inside a syntax node — a superset
/// used to derive candidate dependency edges between symbols.
trait ToIdents {
    fn accept(&self, visitor: &mut IdentCollector);
}

macro_rules! impl_to_idents {
    ($ty:ty, $visit_fn:ident) => {
        impl ToIdents for $ty {
            fn accept(&self, visitor: &mut IdentCollector) {
                visitor.$visit_fn(self);
            }
        }
    };
}

impl_to_idents!(syn::ItemFn, visit_item_fn);
impl_to_idents!(syn::ItemStruct, visit_item_struct);
impl_to_idents!(syn::ItemEnum, visit_item_enum);
impl_to_idents!(syn::ItemTrait, visit_item_trait);
impl_to_idents!(syn::ItemConst, visit_item_const);
impl_to_idents!(syn::ItemStatic, visit_item_static);
impl_to_idents!(syn::ItemType, visit_item_type);
impl_to_idents!(syn::ItemMod, visit_item_mod);
impl_to_idents!(syn::ItemImpl, visit_item_impl);
impl_to_idents!(syn::ImplItemFn, visit_impl_item_fn);

fn collect_idents<N: ToIdents>(node: &N) -> Vec<String> {
    let mut visitor = IdentCollector::default();
    node.accept(&mut visitor);
    visitor.idents.sort();
    visitor.idents.dedup();
    visitor.idents
}

#[derive(Default)]
struct IdentCollector {
    idents: Vec<String>,
}

impl<'ast> Visit<'ast> for IdentCollector {
    fn visit_ident(&mut self, ident: &'ast proc_macro2::Ident) {
        self.idents.push(ident.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn path() -> PathBuf {
        PathBuf::from("test.rs")
    }

    #[test]
    fn extracts_top_level_function_with_line_range() {
        let source = "fn main() {\n    println!(\"hi\");\n}\n";
        let symbols = parse_rust_symbols(&path(), source).unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "main");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[0].range.start, 1);
        assert_eq!(symbols[0].range.end, 3);
    }

    #[test]
    fn extracts_struct_enum_and_trait() {
        let source = r#"
struct Point { x: i32, y: i32 }
enum Shape { Circle, Square }
trait Draw { fn draw(&self); }
"#;
        let symbols = parse_rust_symbols(&path(), source).unwrap();
        let names: Vec<_> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Point"));
        assert!(names.contains(&"Shape"));
        assert!(names.contains(&"Draw"));
    }

    #[test]
    fn extracts_impl_methods_with_qualified_names() {
        let source = r#"
struct Widget;
impl Widget {
    fn new() -> Self { Widget }
    fn render(&self) {}
}
"#;
        let symbols = parse_rust_symbols(&path(), source).unwrap();
        let names: Vec<_> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Widget::new"));
        assert!(names.contains(&"Widget::render"));
        assert!(names.contains(&"Widget")); // the impl container itself
    }

    #[test]
    fn function_body_references_include_called_symbol_names() {
        let source = "fn helper() {}\nfn main() { helper(); }\n";
        let symbols = parse_rust_symbols(&path(), source).unwrap();
        let main_symbol = symbols.iter().find(|s| s.name == "main").unwrap();
        assert!(main_symbol.references.contains(&"helper".to_string()));
    }

    #[test]
    fn invalid_syntax_is_a_parse_failure_not_a_silent_empty_result() {
        let source = "fn main( {{{ this is not rust";
        let result = parse_rust_symbols(&path(), source);
        assert!(result.is_err());
    }

    #[test]
    fn nested_module_items_get_dotted_names() {
        let source = "mod inner {\n    fn helper() {}\n}\n";
        let symbols = parse_rust_symbols(&path(), source).unwrap();
        let names: Vec<_> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"inner"));
        assert!(names.contains(&"inner::helper"));
    }
}
