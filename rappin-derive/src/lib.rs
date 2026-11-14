//! Generate Rappin argument parsing, help and read-only validation metadata.
use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::quote;
use syn::{parse_macro_input, spanned::Spanned, DeriveInput, Expr, Fields, Lit, Meta, Type};

/// Derive `rappin::FromArgs` for a struct with named fields.
///
/// Field documentation supplies help. `Option<T>` is optional. `#[arg(default =
/// 6, min = 2, max = 100)]` supplies a literal default and inclusive bounds.
/// String bounds count Unicode characters. `#[arg(rest)]` consumes the final
/// string argument. Custom field types use their `FromStr` implementation and
/// report argument validation as deferred during checked-invocation preparation.
/// Use the struct attribute `#[arg(crate = "sdk_alias")]` to select an explicit
/// SDK import when dependency-name discovery is ambiguous.
///
/// Enable the SDK's `derive` feature. A complete compiling example is provided
/// by `rappin::FromArgs` and the SDK tutorial.
///
/// ```text
/// #[derive(rappin::FromArgs)]
/// struct Roll {
///     /// Number of sides.
///     #[arg(default = 6, min = 2, max = 100)]
///     sides: u32,
/// }
/// ```
#[proc_macro_derive(FromArgs, attributes(arg))]
pub fn from_args(input: TokenStream) -> TokenStream {
    expand(parse_macro_input!(input as DeriveInput))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[derive(Default)]
struct Options {
    default: Option<Expr>,
    min: Option<Expr>,
    max: Option<Expr>,
    rest: bool,
}

fn literal(expression: &Expr) -> bool {
    match expression {
        Expr::Lit(_) => true,
        Expr::Unary(value) if matches!(value.op, syn::UnOp::Neg(_)) => {
            matches!(&*value.expr, Expr::Lit(value) if matches!(value.lit, Lit::Int(_) | Lit::Float(_)))
        }
        _ => false,
    }
}

fn options(field: &syn::Field) -> syn::Result<Options> {
    let mut options = Options::default();
    for attribute in field.attrs.iter().filter(|a| a.path().is_ident("arg")) {
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("rest") {
                if options.rest {
                    return Err(meta.error("duplicate rest attribute"));
                }
                options.rest = true;
                return Ok(());
            }
            let target = if meta.path.is_ident("default") {
                &mut options.default
            } else if meta.path.is_ident("min") {
                &mut options.min
            } else if meta.path.is_ident("max") {
                &mut options.max
            } else {
                return Err(meta.error("expected default, min, max or rest"));
            };
            if target.is_some() {
                return Err(meta.error("duplicate argument attribute"));
            }
            let value: Expr = meta.value()?.parse()?;
            if !literal(&value) {
                return Err(syn::Error::new(
                    value.span(),
                    "argument attributes require literal values",
                ));
            }
            *target = Some(value);
            Ok(())
        })?;
    }
    Ok(options)
}

fn type_name(ty: &Type) -> String {
    if let Type::Path(path) = ty {
        return path
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
    }
    String::new()
}

fn optional_type(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else { return None };
    let segment = path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    match arguments.args.first()? {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    }
}

fn documentation(field: &syn::Field) -> String {
    field
        .attrs
        .iter()
        .filter_map(|attribute| {
            if !attribute.path().is_ident("doc") {
                return None;
            }
            let Meta::NameValue(value) = &attribute.meta else {
                return None;
            };
            let Expr::Lit(value) = &value.value else {
                return None;
            };
            let Lit::Str(value) = &value.lit else {
                return None;
            };
            Some(value.value().trim().to_owned())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn bound(value: &Option<Expr>) -> Tokens {
    match value {
        Some(value) => quote!(Some(#value)),
        None => quote!(None),
    }
}

fn number(value: &Expr) -> Option<f64> {
    match value {
        Expr::Lit(value) => match &value.lit {
            Lit::Int(value) => value.base10_parse().ok(),
            Lit::Float(value) => value.base10_parse().ok(),
            Lit::Str(value) => Some(value.value().chars().count() as f64),
            _ => None,
        },
        Expr::Unary(value) if matches!(value.op, syn::UnOp::Neg(_)) => {
            number(&value.expr).map(|n| -n)
        }
        _ => None,
    }
}

fn integer(value: &Expr) -> Option<i128> {
    match value {
        Expr::Lit(value) => match &value.lit {
            Lit::Int(value) => value.base10_parse().ok(),
            Lit::Str(value) => Some(value.value().chars().count() as i128),
            _ => None,
        },
        Expr::Unary(value) if matches!(value.op, syn::UnOp::Neg(_)) => {
            integer(&value.expr)?.checked_neg()
        }
        _ => None,
    }
}

fn exceeds(left: &Expr, right: &Expr) -> bool {
    if let (Some(left), Some(right)) = (integer(left), integer(right)) {
        return left > right;
    }
    matches!((number(left), number(right)), (Some(left), Some(right)) if left > right)
}

fn validate_bounds(options: &Options, span: proc_macro2::Span) -> syn::Result<()> {
    if matches!((&options.min, &options.max), (Some(min), Some(max)) if exceeds(min, max)) {
        return Err(syn::Error::new(span, "argument minimum exceeds maximum"));
    }
    if let Some(default) = &options.default {
        if options
            .min
            .as_ref()
            .is_some_and(|min| exceeds(min, default))
            || options
                .max
                .as_ref()
                .is_some_and(|max| exceeds(default, max))
        {
            return Err(syn::Error::new(
                span,
                "argument default is outside its bounds",
            ));
        }
    }
    Ok(())
}

fn schema_kind(name: &str, opts: &Options, sdk: &Tokens) -> Option<Tokens> {
    let min = bound(&opts.min);
    let max = bound(&opts.max);
    match name {
        "String" => {
            let minimum = opts.min.as_ref().map(|x| quote!(#x)).unwrap_or(quote!(0));
            Some(quote!(#sdk::ArgumentType::String { min_length: #minimum, max_length: #max }))
        }
        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" => {
            // Type bounds belong in preparation too, including the implicit unsigned minimum.
            let ty: Type =
                syn::parse_str(&format!("::std::primitive::{name}")).expect("primitive type");
            let minimum = opts
                .min
                .as_ref()
                .map(|x| quote!(#x))
                .unwrap_or(quote!(<#ty>::MIN));
            let maximum = opts
                .max
                .as_ref()
                .map(|x| quote!(#x))
                .unwrap_or(quote!(<#ty>::MAX));
            Some(quote!(#sdk::ArgumentType::Integer {
                min: Some({ let value: #ty = #minimum; i64::from(value) }),
                max: Some({ let value: #ty = #maximum; i64::from(value) }),
            }))
        }
        // f32 rounding cannot be represented exactly by the f64 runtime schema.
        // Execution parses and checks it, while preparation reports it deferred.
        "f32" => None,
        "f64" => Some(quote!(#sdk::ArgumentType::Number { min: #min, max: #max })),
        "bool" => Some(quote!(#sdk::ArgumentType::Boolean)),
        "UserId" => Some(quote!(#sdk::ArgumentType::UserId)),
        "ChannelId" => Some(quote!(#sdk::ArgumentType::ChannelId)),
        "HumanDuration" => {
            Some(quote!(#sdk::ArgumentType::Duration { min_seconds: None, max_seconds: None }))
        }
        _ => None,
    }
}

fn expand(input: DeriveInput) -> syn::Result<Tokens> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(input.generics.span(), "derive FromArgs on a concrete argument struct; implement FromArgs manually for generic parsers"));
    }
    let syn::Data::Struct(data) = input.data else {
        return Err(syn::Error::new(
            input.ident.span(),
            "FromArgs requires a struct with named fields",
        ));
    };
    let Fields::Named(fields) = data.fields else {
        return Err(syn::Error::new(
            input.ident.span(),
            "FromArgs requires named fields",
        ));
    };
    let mut sdk_path: Option<syn::Path> = None;
    for attribute in input
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("arg"))
    {
        attribute.parse_nested_meta(|meta| {
            if !meta.path.is_ident("crate") || sdk_path.is_some() {
                return Err(
                    meta.error("use one crate = \"sdk_alias\" attribute on the argument struct")
                );
            }
            let path: syn::LitStr = meta.value()?.parse()?;
            sdk_path = Some(path.parse()?);
            Ok(())
        })?;
    }
    let sdk = if let Some(path) = sdk_path {
        quote!(#path)
    } else {
        let crate_name = proc_macro_crate::crate_name("rappin-rs")
            .map_err(|e| syn::Error::new(input.ident.span(), e.to_string()))?;
        match crate_name {
            proc_macro_crate::FoundCrate::Itself => quote!(::rappin),
            proc_macro_crate::FoundCrate::Name(name) if name == "rappin_rs" => quote!(::rappin),
            proc_macro_crate::FoundCrate::Name(name) => {
                let name = syn::Ident::new(&name.replace('-', "_"), input.ident.span());
                quote!(::#name)
            }
        }
    };
    let mut reads = Vec::new();
    let mut names = Vec::new();
    let mut help = Vec::new();
    let mut schemas = Vec::new();
    let mut schema_types = Vec::new();
    let mut complete_schema = true;
    let mut optional_seen = false;
    let cursor = syn::Ident::new("__rappin_arguments", proc_macro2::Span::mixed_site());
    for (index, field) in fields.named.iter().enumerate() {
        let opts = options(field)?;
        validate_bounds(&opts, field.span())?;
        let ident = field.ident.as_ref().expect("named field");
        let name = ident.to_string().trim_start_matches("r#").to_owned();
        let optional = optional_type(&field.ty);
        let ty = optional.unwrap_or(&field.ty);
        let kind_name = type_name(ty);
        let required = optional.is_none() && opts.default.is_none();
        if optional_seen && required {
            return Err(syn::Error::new(
                field.span(),
                "required arguments must precede optional/defaulted arguments",
            ));
        }
        optional_seen |= !required;
        if optional.is_some() && opts.default.is_some() {
            return Err(syn::Error::new(
                field.span(),
                "use Option<T> or a defaulted T for an argument",
            ));
        }
        if opts.rest && (kind_name != "String" || index + 1 != fields.named.len()) {
            return Err(syn::Error::new(
                field.span(),
                "rest requires the final String or Option<String> field",
            ));
        }
        if (opts.min.is_some() || opts.max.is_some())
            && !matches!(
                kind_name.as_str(),
                "String"
                    | "i8"
                    | "i16"
                    | "i32"
                    | "i64"
                    | "u8"
                    | "u16"
                    | "u32"
                    | "u64"
                    | "usize"
                    | "isize"
                    | "f32"
                    | "f64"
            )
        {
            return Err(syn::Error::new(
                field.span(),
                "min/max require a numeric or String field",
            ));
        }
        let read = if opts.rest {
            quote!(if #cursor.remaining() == 0 {
                None
            } else {
                Some(#cursor.rest())
            })
        } else {
            quote!(#cursor.optional::<#ty>(#name)?)
        };
        let mut description = documentation(field);
        if description.is_empty() {
            description = name.replace('_', " ");
        }
        let read = if let Some(default) = &opts.default {
            description.push_str(&format!(" (default {})", quote!(#default)));
            let default = if kind_name == "String" {
                quote!(#default.to_owned())
            } else {
                quote!(#default)
            };
            quote!(#read.unwrap_or_else(|| #default))
        } else if optional.is_some() {
            read
        } else {
            quote!(#read.ok_or_else(|| #sdk::CommandError::Missing(#name.into()))?)
        };
        let value = if optional.is_some() {
            quote!(if let Some(value) = &#ident)
        } else {
            quote!(let value = &#ident;)
        };
        let measured = if kind_name == "String" {
            quote!(value.chars().count())
        } else {
            quote!(*value)
        };
        let bound_ty = if kind_name == "String" {
            quote!(usize)
        } else {
            quote!(#ty)
        };
        let mut checks = Vec::new();
        if let Some(min) = &opts.min {
            description.push_str(&format!(" (minimum {})", quote!(#min)));
            checks.push(quote!({ let bound: #bound_ty = #min; if #measured < bound { return Err(#sdk::CommandError::Invalid { name: #name.into(), reason: format!("minimum is {}", bound) }); } }));
        }
        if let Some(max) = &opts.max {
            description.push_str(&format!(" (maximum {})", quote!(#max)));
            checks.push(quote!({ let bound: #bound_ty = #max; if #measured > bound { return Err(#sdk::CommandError::Invalid { name: #name.into(), reason: format!("maximum is {}", bound) }); } }));
        }
        if matches!(kind_name.as_str(), "f32" | "f64") {
            checks.push(quote!(if !value.is_finite() { return Err(#sdk::CommandError::Invalid { name: #name.into(), reason: "use a finite number".into() }); }));
        }
        let checks = if checks.is_empty() {
            quote!()
        } else if optional.is_some() {
            quote!(#value { #(#checks)* })
        } else {
            quote!({ #value #(#checks)* })
        };
        let field_ty = &field.ty;
        reads.push(quote!(let #ident: #field_ty = #read; #checks));
        names.push(ident);
        help.push(quote!(#sdk::ArgumentInfo { name: #name.into(), description: #description.into(), required: #required }));
        if let Some(kind) = schema_kind(&kind_name, &opts, &sdk) {
            let canonical = match kind_name.as_str() {
                "String" => quote!(::std::string::String),
                "UserId" => quote!(#sdk::UserId),
                "ChannelId" => quote!(#sdk::ChannelId),
                "HumanDuration" => quote!(#sdk::HumanDuration),
                _ => {
                    let primitive = syn::Ident::new(&kind_name, ty.span());
                    quote!(::std::primitive::#primitive)
                }
            };
            schema_types.push(quote!(
                if ::std::any::TypeId::of::<#ty>() != ::std::any::TypeId::of::<#canonical>() {
                    return None;
                }
            ));
            let rest = opts.rest;
            schemas.push(quote!(#sdk::ArgumentSchema { name: #name.into(), description: #description.into(), kind: #kind, required: #required, rest: #rest }));
        } else {
            complete_schema = false;
        }
    }
    let schema = if complete_schema {
        quote!({ #(#schema_types)* Some(vec![#(#schemas),*]) })
    } else {
        quote!(None)
    };
    let name = input.ident;
    Ok(quote! {
        impl #sdk::FromArgs for #name {
            fn parse(#cursor: &mut #sdk::Arguments) -> ::std::result::Result<Self, #sdk::CommandError> {
                #(#reads)*
                Ok(Self { #(#names),* })
            }
            fn describe() -> Vec<#sdk::ArgumentInfo> { vec![#(#help),*] }
            fn schema() -> Option<Vec<#sdk::ArgumentSchema>> { #schema }
        }
    })
}
