use proc_macro::{Span, TokenStream};
use quote::quote;
use syn::{parse_macro_input, Error, ItemFn};

const NUM_WORKERS_ATTR: &str = "num_workers";
const DEFAULT_NUM_WORKERS: usize = 1;

#[proc_macro_attribute]
pub fn hooch_main(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(item as ItemFn);
    let is_async = input.sig.asyncness.is_some();

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
            let handle = RuntimeBuilder::default().build();

            handle.run_blocking(async {
                #main_hooch_fn().await
            });
        }

    };
    output.into()
}
