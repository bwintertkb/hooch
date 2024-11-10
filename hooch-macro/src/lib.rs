use proc_macro::{Span, TokenStream};
use quote::quote;
use syn::{parse::Parse, parse_macro_input, Error, Ident, ItemFn, LitInt, Token};

const DEFAULT_NUM_WORKERS: usize = 1;

struct WorkersAttr {
    workers: usize,
}

impl Parse for WorkersAttr {
    // ParseStream acts as an internal cursor that keeps track of the current position in the
    // token stream. Each `input.parse::<Type>()?` call parses a part of the token stream and
    // advances the cursor
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        let value: LitInt = input.parse()?;

        if name == "workers" {
            let workers = value.base10_parse()?;
            Ok(Self { workers })
        } else {
            Err(input.error("Expected `workers` argument"))
        }
    }
}

#[proc_macro_attribute]
pub fn hooch_main(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(item as ItemFn);
    let is_async = input.sig.asyncness.is_some();

    if input.sig.ident != "main" {
        let error = Error::new_spanned(input, "hooch_main can only be used on 'main'");
        return error.to_compile_error().into();
    }

    let workers: usize = if attr.is_empty() {
        DEFAULT_NUM_WORKERS
    } else {
        let WorkersAttr { workers } = parse_macro_input!(attr as WorkersAttr);
        workers
    };

    input.sig.ident = syn::Ident::new("main_hooch", input.sig.ident.span());
    let main_hooch_fn = syn::Ident::new("main_hooch", Span::call_site().into());

    if !is_async {
        let error = Error::new_spanned(input.sig.fn_token, "main must be async");
        return error.to_compile_error().into();
    }

    let output = quote! {
        use hooch::runtime::RuntimeBuilder;
        #input
        fn main() {
            let handle = RuntimeBuilder::new().num_workers(#workers).build();

            handle.run_blocking(async {
                #main_hooch_fn().await
            });
        }

    };
    output.into()
}
