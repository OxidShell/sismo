//! Attribute macros for the `sismo` recorder.

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, LitStr, parse_macro_input};

/// Records every call of the annotated function as a span named
/// `module::function`. Async functions are recorded from first poll to
/// completion. Override the name with `#[sismo::function("name")]`.
#[proc_macro_attribute]
pub fn function(attr: TokenStream, item: TokenStream) -> TokenStream {
    let custom = if attr.is_empty() {
        None
    } else {
        Some(parse_macro_input!(attr as LitStr))
    };
    let ItemFn {
        attrs,
        vis,
        sig,
        block,
    } = parse_macro_input!(item as ItemFn);
    let ident = sig.ident.to_string();
    let name = if let Some(lit) = custom {
        quote!(#lit)
    } else {
        quote!(concat!(module_path!(), "::", #ident))
    };
    let callsite = quote! {
        static __SISMO_CALLSITE: ::sismo::Callsite = ::sismo::Callsite::new(
            ::sismo::__private::CallsiteKind::Span,
            #name,
            module_path!(),
            file!(),
            line!(),
        );
    };
    let body = if sig.asyncness.is_some() {
        quote! {
            #callsite
            ::sismo::Instrumented::new(&__SISMO_CALLSITE, async move #block).await
        }
    } else {
        quote! {
            #callsite
            let __sismo_guard = ::sismo::ScopeGuard::enter(&__SISMO_CALLSITE);
            #block
        }
    };
    quote! {
        #(#attrs)*
        #vis #sig {
            #body
        }
    }
    .into()
}
