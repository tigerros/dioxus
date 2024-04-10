use proc_macro2::TokenStream;
use quote::{quote, ToTokens, TokenStreamExt};
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::*;

pub struct ComponentBody {
    pub item_fn: ItemFn,
}

impl Parse for ComponentBody {
    fn parse(input: ParseStream) -> Result<Self> {
        let item_fn: ItemFn = input.parse()?;
        validate_component_fn_signature(&item_fn)?;
        Ok(Self { item_fn })
    }
}

impl ToTokens for ComponentBody {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        // https://github.com/DioxusLabs/dioxus/issues/1938
        // If there's only one input and the input is `props: Props`, we don't need to generate a props struct
        // Just attach the non_snake_case attribute to the function
        // eventually we'll dump this metadata into devtooling that lets us find all these components
        // 
        // Components can also use the struct pattern to "inline" their props.
        // Freya uses this a bunch (because it's clean),
        // e.g. `fn Navbar(NavbarProps { title }: NavbarProps)` was previously being incorrectly parsed
        if self.is_explicit_props_ident() || self.has_struct_parameter_pattern() {
            let comp_fn = &self.item_fn;
            tokens.append_all(prefer_camel_case_for_fn_ident(comp_fn).into_token_stream());
            return;
        }

        let comp_fn = self.comp_fn();

        // If there's no props declared, we simply omit the props argument
        // This is basically so you can annotate the App component with #[component] and still be compatible with the
        // launch signatures that take fn() -> Element
        let props_struct = match self.item_fn.sig.inputs.is_empty() {
            // No props declared, so we don't need to generate a props struct
            true => quote! {},

            // Props declared, so we generate a props struct and thatn also attach the doc attributes to it
            false => {
                let doc = format!("Properties for the [`{}`] component.", &comp_fn.sig.ident);
                let props_struct = self.props_struct();
                quote! {
                    #[doc = #doc]
                    #props_struct
                }
            }
        };

        tokens.append_all(quote! {
            #props_struct
            #comp_fn
        });
    }
}

impl ComponentBody {
    // build a new item fn, transforming the original item fn
    fn comp_fn(&self) -> ItemFn {
        let ComponentBody { item_fn, .. } = self;
        let ItemFn {
            attrs,
            vis,
            sig,
            block,
        } = item_fn;
        let Signature {
            inputs,
            ident: fn_ident,
            generics,
            output: fn_output,
            ..
        } = sig;

        let Generics { where_clause, .. } = generics;
        let (_, ty_generics, _) = generics.split_for_impl();

        // We generate a struct with the same name as the component but called `Props`
        let struct_ident = Ident::new(&format!("{fn_ident}Props"), fn_ident.span());

        // We pull in the field names from the original function signature, but need to strip off the mutability
        let struct_field_names = inputs.iter().filter_map(rebind_mutability);

        // Don't generate the props argument if there are no inputs
        // This means we need to skip adding the argument to the function signature, and also skip the expanded struct
        let props_ident = match inputs.is_empty() {
            true => quote! {},
            false => quote! { mut __props: #struct_ident #ty_generics },
        };
        let expanded_struct = match inputs.is_empty() {
            true => quote! {},
            false => quote! { let #struct_ident { #(#struct_field_names),* } = __props; },
        };

        // The extra nest is for the snake case warning to kick back in
        parse_quote! {
            #(#attrs)*
            #[allow(non_snake_case)]
            #vis fn #fn_ident #generics (#props_ident) #fn_output #where_clause {
                {
                    { struct #fn_ident {} }
                    #expanded_struct
                    #block
                }
            }
        }
    }

    /// Build an associated struct for the props of the component
    ///
    /// This will expand to the typed-builder implementation that we have vendored in this crate.
    /// TODO: don't vendor typed-builder and instead transform the tokens we give it before expansion.
    /// TODO: cache these tokens since this codegen is rather expensive (lots of tokens)
    ///
    /// We try our best to transfer over any declared doc attributes from the original function signature onto the
    /// props struct fields.
    fn props_struct(&self) -> ItemStruct {
        let ItemFn { vis, sig, .. } = &self.item_fn;
        let Signature {
            inputs,
            ident,
            generics,
            ..
        } = sig;

        let struct_fields = inputs.iter().map(move |f| make_prop_struct_field(f, vis));
        let struct_ident = Ident::new(&format!("{ident}Props"), ident.span());

        parse_quote! {
            #[derive(Props, Clone, PartialEq)]
            #[allow(non_camel_case_types)]
            #vis struct #struct_ident #generics {
                #(#struct_fields),*
            }
        }
    }

    fn is_explicit_props_ident(&self) -> bool {
        if let Some(first_input) = self.item_fn.sig.inputs.first() {
            if let FnArg::Typed(PatType { pat, .. }) = &first_input {
                if let Pat::Ident(ident) = pat.as_ref() {
                    return ident.ident == "props";
                }
            }
        }

        false
    }

    fn has_struct_parameter_pattern(&self) -> bool {
        if let Some(first_input) = self.item_fn.sig.inputs.first() {
            if let FnArg::Typed(PatType { pat, .. }) = &first_input {
                if matches!(pat.as_ref(), Pat::Struct(_)) {
                    return true;
                }
            }
        }

        false
    }
}

fn validate_component_fn_signature(item_fn: &ItemFn) -> Result<()> {
    // Do some validation....
    // 1. Ensure the component returns *something*
    if item_fn.sig.output == ReturnType::Default {
        return Err(Error::new(
            item_fn.sig.output.span(),
            "Must return a <dioxus_core::Element>".to_string(),
        ));
    }

    // 2. make sure there's no lifetimes on the component - we don't know how to handle those
    if item_fn.sig.generics.lifetimes().count() > 0 {
        return Err(Error::new(
            item_fn.sig.generics.span(),
            "Lifetimes are not supported in components".to_string(),
        ));
    }

    // 3. we can't handle async components
    if item_fn.sig.asyncness.is_some() {
        return Err(Error::new(
            item_fn.sig.asyncness.span(),
            "Async components are not supported".to_string(),
        ));
    }

    // 4. we can't handle const components
    if item_fn.sig.constness.is_some() {
        return Err(Error::new(
            item_fn.sig.constness.span(),
            "Const components are not supported".to_string(),
        ));
    }

    // 5. no receiver parameters
    if item_fn
        .sig
        .inputs
        .iter()
        .any(|f| matches!(f, FnArg::Receiver(_)))
    {
        return Err(Error::new(
            item_fn.sig.inputs.span(),
            "Receiver parameters are not supported".to_string(),
        ));
    }

    Ok(())
}

/// Convert a function arg with a given visibility (provided by the function) and then generate a field for the
/// associated props struct.
fn make_prop_struct_field(f: &FnArg, vis: &Visibility) -> TokenStream {
    // There's no receivers (&self) allowed in the component body
    let FnArg::Typed(pt) = f else { unreachable!() };

    let arg_pat = match pt.pat.as_ref() {
        // rip off mutability
        // todo: we actually don't want any of the extra bits of the field pattern
        Pat::Ident(f) => {
            let mut f = f.clone();
            f.mutability = None;
            quote! { #f }
        }
        a => quote! { #a },
    };

    let PatType {
        attrs,
        ty,
        colon_token,
        ..
    } = pt;

    quote! {
        #(#attrs)*
        #vis #arg_pat #colon_token #ty
    }
}

fn rebind_mutability(f: &FnArg) -> Option<TokenStream> {
    // There's no receivers (&self) allowed in the component body
    let FnArg::Typed(pt) = f else { unreachable!() };

    let pat = &pt.pat;

    let mut pat = pat.clone();

    // rip off mutability, but still write it out eventually
    if let Pat::Ident(ref mut pat_ident) = pat.as_mut() {
        pat_ident.mutability = None;
    }

    Some(quote!(mut  #pat))
}

/// Takes a function and returns a clone of it where an `UpperCamelCase` identifier is preferred by the compiler.
fn prefer_camel_case_for_fn_ident(item_fn: &ItemFn) -> ItemFn {
    let mut clone = item_fn.clone();
    let ident = &item_fn.sig.ident;
    let block = &item_fn.block;

    clone.attrs.push(parse_quote! { #[allow(non_snake_case)] });
    
    clone.block = parse_quote! {
        {
            { struct #ident {} }
            #block
        }
    };

    clone
}