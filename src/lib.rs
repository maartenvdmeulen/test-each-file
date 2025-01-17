#![doc = include_str!("../README.md")]
use pathdiff::diff_paths;
use proc_macro2::{Ident, TokenStream};
use proc_macro_error::{abort, abort_call_site, proc_macro_error};
use std::collections::{HashMap, HashSet};
use std::fs::canonicalize;
use std::path::{Path, PathBuf};

use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{bracketed, parse_macro_input, Expr, LitStr, Token};

struct ForEachFile {
    path: LitStr,
    module: Option<Ident>,
    function: Expr,
    extensions: Vec<String>,
}

impl Parse for ForEachFile {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // Parse extensions if the keyword `for` is used. Aborts if no extensions are given.
        let extensions = input
            .parse::<Token![for]>()
            .and_then(|_| {
                let content;
                bracketed!(content in input);

                match Punctuated::<LitStr, Token![,]>::parse_separated_nonempty(&content) {
                    Ok(extensions) => Ok(extensions
                        .into_iter()
                        .map(|extension| extension.value())
                        .collect()),
                    Err(e) => abort!(e.span(), "Expected at least one extension to be given."),
                }
            })
            .unwrap_or_default();

        // Parse the path to the tests.
        if let Err(e) = input.parse::<Token![in]>() {
            abort!(e.span(), "Expected the keyword `in` before the path.");
        };

        let path = match input.parse::<LitStr>() {
            Ok(path) => path,
            Err(e) => abort!(e.span(), "Expected a path after the keyword 'in'."),
        };

        let module = input
            .parse::<Token![as]>()
            .and_then(|_| match input.parse::<Ident>() {
                Ok(module) => Ok(module),
                Err(e) => abort!(e.span(), "Expected a module to be given."),
            })
            .ok();

        // Parse function to call.
        if let Err(e) = input.parse::<Token![=>]>() {
            abort!(e.span(), "Expected `=>` before the function to call.");
        };

        let function = match input.parse::<Expr>() {
            Ok(function) => function,
            Err(e) => abort!(e.span(), "Expected a function to call after `=>`."),
        };

        Ok(Self {
            path,
            module,
            function,
            extensions,
        })
    }
}

#[derive(Default)]
struct Tree {
    children: HashMap<PathBuf, Tree>,
    here: HashSet<PathBuf>,
}

impl Tree {
    fn new(base: &Path, ignore_extensions: bool) -> Self {
        let mut tree = Self::default();
        for entry in base.read_dir().unwrap() {
            let mut entry = entry.unwrap().path();
            if entry.is_file() {
                if ignore_extensions {
                    entry.set_extension("");
                }
                tree.here.insert(entry);
            } else if entry.is_dir() {
                tree.children.insert(
                    entry.as_path().to_path_buf(),
                    Self::new(entry.as_path(), ignore_extensions),
                );
            } else {
                abort_call_site!(format!("Unsupported path: {:#?}.", entry))
            }
        }
        tree
    }
}

fn generate_from_tree(tree: &Tree, parsed: &ForEachFile, stream: &mut TokenStream) {
    for file in &tree.here {
        let mut diff = diff_paths(file, parsed.path.value()).unwrap();
        diff.set_extension("");
        let file_name = diff.file_name().unwrap().to_str().unwrap();

        let file_name = format_ident!("{file_name}");

        let function = &parsed.function;

        let content = if parsed.extensions.is_empty() {
            let file = canonicalize(file).unwrap();
            let file = file.to_str().unwrap();
            quote!(include_str!(#file))
        } else {
            let mut content = TokenStream::new();

            for ext in &parsed.extensions {
                let mut file = file.clone();
                file.set_extension(ext);
                let file = canonicalize(file).unwrap();
                let file = file.to_str().unwrap();

                content.extend(quote!(include_str!(#file),));
            }

            quote!([#content])
        };

        stream.extend(quote! {
            #[test]
            fn #file_name() {
                (#function)(#content)
            }
        });
    }

    for (name, directory) in &tree.children {
        let mut sub_stream = TokenStream::new();
        generate_from_tree(directory, parsed, &mut sub_stream);
        let name = format_ident!("{}", name.file_name().unwrap().to_str().unwrap());
        stream.extend(quote! {
            mod #name {
                use super::*;
                #sub_stream
            }
        });
    }
}

/// Easily generate tests for files in a specified directory for comprehensive testing.
///
/// See crate level documentation for details.
#[proc_macro]
#[proc_macro_error]
pub fn test_each_file(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let parsed = parse_macro_input!(input as ForEachFile);

    if !Path::new(&parsed.path.value()).is_dir() {
        abort!(parsed.path.span(), "Given directory does not exist");
    }

    let mut tokens = TokenStream::new();
    let files = Tree::new(parsed.path.value().as_ref(), !parsed.extensions.is_empty());
    generate_from_tree(&files, &parsed, &mut tokens);

    if let Some(module) = parsed.module {
        tokens = quote! {
            #[cfg(test)]
            mod #module {
                use super::*;
                #tokens
            }
        }
    }

    proc_macro::TokenStream::from(tokens)
}
