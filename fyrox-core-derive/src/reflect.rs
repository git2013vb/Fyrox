//! Implements `Reflect` trait

pub mod args;
mod prop;
mod syntax;

use darling::ast;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use prop::Property;

pub fn impl_reflect(ty_args: &args::TypeArgs) -> TokenStream2 {
    if ty_args.hide_all {
        return self::gen_impl(
            ty_args,
            quote!(None),
            quote!(None),
            quote!(vec![]),
            quote!(vec![]),
            None,
        );
    }

    match &ty_args.data {
        ast::Data::Struct(ref field_args) => self::impl_reflect_struct(ty_args, field_args),
        ast::Data::Enum(ref variant_args) => self::impl_reflect_enum(ty_args, variant_args),
    }
}

pub fn impl_prop_constants(ty_args: &args::TypeArgs) -> TokenStream2 {
    let prop_keys = prop::props(ty_args).collect::<Vec<_>>();
    prop::impl_prop_constants(prop_keys.iter(), &ty_args.ident, &ty_args.generics)
}

fn impl_reflect_struct(ty_args: &args::TypeArgs, _field_args: &args::Fields) -> TokenStream2 {
    // Property keys for `Reflect::{field, field_mut, set_field}` impls:
    let props = prop::props(ty_args).collect::<Vec<_>>();
    let prop_values = props.iter().map(|p| &p.value).collect::<Vec<_>>();

    let (fields, field_muts): (Vec<_>, Vec<_>) = props
        .iter()
        .map(|p| {
            let quote = &p.field_quote;
            (quote!(&self.#quote), quote!(&mut self.#quote))
        })
        .unzip();
    let (fields, field_muts) = self::collect_field_refs(&props, &fields, &field_muts);
    let fields = fields.collect::<Vec<_>>();
    let field_muts = field_muts.collect::<Vec<_>>();

    let field_body = quote! {
        match name {
            #(
                #prop_values => Some(#fields),
            )*
            _ => None,
        }
    };

    let field_mut_body = quote! {
        match name {
            #(
                #prop_values => Some(#field_muts),
            )*
            _ => None,
        }
    };

    let fields_body = quote! {
        vec! [
            #(
                #fields,
            )*
        ]
    };

    let fields_mut_body = quote! {
        vec! [
            #(
                #field_muts,
            )*
        ]
    };

    let set_field_body = self::struct_set_field_body(ty_args);
    self::gen_impl(
        ty_args,
        field_body,
        field_mut_body,
        fields_body,
        fields_mut_body,
        set_field_body,
    )
}

fn struct_set_field_body(ty_args: &args::TypeArgs) -> Option<TokenStream2> {
    let props = prop::props(ty_args)
        .filter(|p| p.field.setter.is_some())
        .collect::<Vec<_>>();

    if props.is_empty() {
        return None;
    }

    let prop_values = props.iter().map(|p| &p.value);

    let set_fields = props.iter().map(|p| {
        let setter = p.field.setter.as_ref().unwrap();
        quote! {{
            match value.take() {
                Ok(value) => {
                    let prev = self.#setter(value);
                    Ok(Box::new(prev))
                }
                Err(current) => {
                    Err(current)
                }
            }
        }}
    });

    Some(quote! {
        match name {
            #(
                #prop_values => #set_fields,
            )*
            _ => {
                self.set(value)
            },
        }
    })
}

fn impl_reflect_enum(ty_args: &args::TypeArgs, variant_args: &[args::VariantArgs]) -> TokenStream2 {
    let mut fields_list = Vec::new();
    let mut fields_list_mut = Vec::new();
    let (fields, field_muts): (Vec<_>, Vec<_>) = variant_args
        .iter()
        .map(|v| {
            let fields = v
                .fields
                .iter()
                .enumerate()
                .filter(|(_, f)| !f.hidden)
                .collect::<Vec<_>>();

            let props = fields
                .iter()
                .map(|(i, f)| prop::enum_prop(v, *i, f))
                .collect::<Vec<_>>();

            let prop_values = props.iter().map(|p| &p.value).collect::<Vec<_>>();

            let syntax = syntax::VariantSyntax::new(ty_args.ident.clone(), v);
            let matcher = syntax.matcher();

            let (fields, field_muts): (Vec<_>, Vec<_>) = fields
                .iter()
                .map(|(i, f)| {
                    let field_quote = syntax.field_match_ident(*i, f);
                    (quote!(#field_quote), quote!(#field_quote))
                })
                .unzip();
            let (fields, field_muts) = self::collect_field_refs(&props, &fields, &field_muts);
            let fields = fields.collect::<Vec<_>>();
            let field_muts = field_muts.collect::<Vec<_>>();

            let fields_list_raw = quote! {
                #(
                    #fields,
                )*
            };

            let fields_mut_list_raw = quote! {
                #(
                    #field_muts,
                )*
            };

            let fields = quote! {
                #(
                    #prop_values => match self {
                        #matcher => #fields,
                        _ => return None,
                    },
                )*
            };

            let field_muts = quote! {
                #(
                    #prop_values => match self {
                        #matcher => #field_muts,
                        _ => return None,
                    },
                )*
            };

            fields_list.push(quote! {
                match self {
                    #matcher => return vec![ #fields_list_raw ],
                    _ => (),
                }
            });

            fields_list_mut.push(quote! {
                match self {
                    #matcher => return vec![ #fields_mut_list_raw ],
                    _ => (),
                }
            });

            (fields, field_muts)
        })
        .unzip();

    if fields.is_empty() {
        self::gen_impl(
            ty_args,
            quote!(None),
            quote!(None),
            quote!(vec![]),
            quote!(vec![]),
            None,
        )
    } else {
        let field_body = quote! {
            Some(match name {
                #(
                    #fields
                )*
                _ => return None,
            })
        };

        let field_mut_body = quote! {
            Some(match name {
                #(
                    #field_muts
                )*
                _ => return None,
            })
        };

        let fields_body = quote! {
            #(
                #fields_list
            )*

            vec![]
        };

        let fields_mut_body = quote! {
            #(
                #fields_list_mut
            )*

            vec![]
        };

        self::gen_impl(
            ty_args,
            field_body,
            field_mut_body,
            fields_body,
            fields_mut_body,
            None,
        )
    }
}

fn gen_impl(
    ty_args: &args::TypeArgs,
    field: TokenStream2,
    field_mut: TokenStream2,
    fields: TokenStream2,
    fields_mut: TokenStream2,
    set_field: Option<TokenStream2>,
) -> TokenStream2 {
    let ty_ident = &ty_args.ident;
    let generics = ty_args.impl_generics();
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let as_list_impl = ty_args.as_list_impl();
    let as_array_impl = ty_args.as_array_impl();

    let set_field = set_field.map(|set_field| {
        quote! {
            fn set_field(&mut self, name: &str, value: Box<dyn Reflect>,) -> Result<Box<dyn Reflect>, Box<dyn Reflect>> {
                #set_field
            }
        }
    });

    quote! {
        #[allow(warnings)]
        impl #impl_generics Reflect for #ty_ident #ty_generics #where_clause {
            fn into_any(self: Box<Self>) -> Box<dyn ::core::any::Any> {
                self
            }

            fn set(&mut self, value: Box<dyn Reflect>) -> Result<Box<dyn Reflect>, Box<dyn Reflect>> {
                let value = match value.take() {
                    Ok(x) => x,
                    Err(err) => return Err(err),
                };
                let this = std::mem::replace(self, value);
                Ok(Box::new(this))
            }

            #set_field

            fn as_any(&self) -> &dyn ::core::any::Any {
                self
            }

            fn as_any_mut(&mut self) -> &mut dyn ::core::any::Any {
                self
            }

            fn as_reflect(&self) -> &dyn Reflect {
                self
            }

            fn as_reflect_mut(&mut self) -> &mut dyn Reflect {
                self
            }

            fn fields(&self) -> Vec<&dyn Reflect> {
                #fields
            }

            fn fields_mut(&mut self) -> Vec<&mut dyn Reflect> {
                #fields_mut
            }

            fn field(&self, name: &str) -> Option<&dyn Reflect> {
                #field
            }

            fn field_mut(&mut self, name: &str) -> Option<&mut dyn Reflect> {
                #field_mut
            }

            #as_array_impl

            #as_list_impl
        }
    }
}

/// Collects field references for match RHS, excluding `#[reflect(setter = ..)]` fields
fn collect_field_refs<'a, 'b: 'a>(
    props: &'b [Property<'a>],
    fields: &'b [TokenStream2],
    field_muts: &'b [TokenStream2],
) -> (
    impl Iterator<Item = TokenStream2> + 'b,
    impl Iterator<Item = TokenStream2> + 'b,
) {
    assert_eq!(props.len(), fields.len());

    // Perform field access override
    let fields = props.iter().zip(fields.iter()).map(|(p, f)| {
        if let Some(field_get) = &p.field.field {
            let ident = &p.field_quote;
            quote!(self.#ident.#field_get)
        } else {
            quote!(#f)
        }
    });

    let field_muts = props.iter().zip(field_muts.iter()).map(|(p, f)| {
        if let Some(field_get_mut) = &p.field.field_mut {
            let ident = &p.field_quote;
            quote!(self.#ident.#field_get_mut)
        } else {
            quote!(#f)
        }
    });

    (fields, field_muts)
}
