use quote::quote;
use syn::{
  Expr, ImplItem, ItemImpl, Result, Token,
  parse::{Parse, ParseStream},
  parse_macro_input, parse_quote,
};

enum SharedPlugin {
  Stateless,
  Expr(Expr),
}

#[derive(Default)]
struct MacroArgs {
  shared_plugin: Option<SharedPlugin>,
}

impl Parse for MacroArgs {
  fn parse(input: ParseStream) -> Result<Self> {
    if input.is_empty() {
      return Ok(Self::default());
    }

    let ident: syn::Ident = input.parse()?;
    match ident.to_string().as_str() {
      "stateless" => {
        if !input.is_empty() {
          return Err(input.error("unexpected tokens after `stateless`"));
        }
        Ok(Self {
          shared_plugin: Some(SharedPlugin::Stateless),
        })
      }
      "shared" => {
        input.parse::<Token![=]>()?;
        let expr = input.parse::<Expr>()?;
        if !input.is_empty() {
          return Err(input.error("unexpected tokens after shared plugin expression"));
        }
        Ok(Self {
          shared_plugin: Some(SharedPlugin::Expr(expr)),
        })
      }
      _ => Err(syn::Error::new_spanned(
        ident,
        "expected `stateless` or `shared = <expr>`",
      )),
    }
  }
}

pub fn expand(
  args: proc_macro::TokenStream,
  tokens: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
  let args = parse_macro_input!(args as MacroArgs);

  let mut input = parse_macro_input!(tokens as ItemImpl);
  match expand_impl(&mut input, args) {
    Ok(extra) => quote!(#input #extra).into(),
    Err(err) => err.to_compile_error().into(),
  }
}

fn expand_impl(input: &mut ItemImpl, args: MacroArgs) -> Result<proc_macro2::TokenStream> {
  let Some((_, trait_path, _)) = &input.trait_ else {
    return Err(syn::Error::new_spanned(
      &input.self_ty,
      "expected a trait impl for JavascriptParserPlugin",
    ));
  };

  if trait_path
    .segments
    .last()
    .is_none_or(|segment| segment.ident != "JavascriptParserPlugin")
  {
    return Err(syn::Error::new_spanned(
      trait_path,
      "attribute only supports impl JavascriptParserPlugin for ...",
    ));
  }

  let mut hook_variants = Vec::new();
  for item in &input.items {
    let ImplItem::Fn(func) = item else {
      continue;
    };

    let method_name = func.sig.ident.to_string();
    let normalized_name = method_name.strip_prefix("r#").unwrap_or(&method_name);
    if normalized_name == "implemented_hooks" || normalized_name == "hooks" {
      return Err(syn::Error::new_spanned(
        &func.sig.ident,
        "remove manual hook metadata; this attribute generates it automatically",
      ));
    }

    hook_variants.push(hook_variant_ident(&func.sig.ident)?);
  }

  let body = if let Some(first) = hook_variants.first() {
    let rest = &hook_variants[1..];
    quote! {
      ::rspack_plugin_javascript::JavascriptParserPluginHooks::empty()
        .with(::rspack_plugin_javascript::JavascriptParserPluginHook::#first)
        #(.with(::rspack_plugin_javascript::JavascriptParserPluginHook::#rest))*
    }
  } else {
    quote! {
      ::rspack_plugin_javascript::JavascriptParserPluginHooks::empty()
    }
  };

  input.items.insert(
    0,
    parse_quote! {
      fn implemented_hooks(&self) -> ::rspack_plugin_javascript::JavascriptParserPluginHooks {
        #body
      }
    },
  );

  let shared = args.shared_plugin.map(|shared_plugin| {
    let self_ty = &input.self_ty;
    let init = match shared_plugin {
      SharedPlugin::Stateless => quote! { #self_ty },
      SharedPlugin::Expr(expr) => quote! { #expr },
    };

    quote! {
      impl #self_ty {
        pub fn shared() -> ::rspack_plugin_javascript::BoxJavascriptParserPlugin {
          static PLUGIN: ::std::sync::LazyLock<::rspack_plugin_javascript::BoxJavascriptParserPlugin> =
            ::std::sync::LazyLock::new(|| ::std::sync::Arc::new(#init));
          ::std::clone::Clone::clone(&*PLUGIN)
        }
      }
    }
  });

  Ok(shared.unwrap_or_default())
}

fn hook_variant_ident(method_ident: &syn::Ident) -> Result<syn::Ident> {
  let method_name = method_ident.to_string();
  let normalized_name = method_name.strip_prefix("r#").unwrap_or(&method_name);

  let mut variant_name = String::with_capacity(normalized_name.len());
  let mut uppercase_next = true;

  for ch in normalized_name.chars() {
    if ch == '_' {
      uppercase_next = true;
      continue;
    }

    if uppercase_next {
      variant_name.extend(ch.to_uppercase());
      uppercase_next = false;
    } else {
      variant_name.push(ch);
    }
  }

  if variant_name.is_empty() {
    return Err(syn::Error::new_spanned(
      method_ident,
      "failed to derive parser hook variant from method name",
    ));
  }

  Ok(syn::Ident::new(&variant_name, method_ident.span()))
}
