use std::sync::Arc;

use rspack_core::{
  BoxLoader, Context, ModuleRuleUseLoader, NormalModuleFactoryResolveLoader, Plugin, Resolver,
};
use rspack_error::{Error, Label, Result, SerdeResultToRspackResultExt};
use rspack_hook::{plugin, plugin_hook};
use rspack_util::json_stringify_str;
use simd_json::{OwnedValue, StaticNode};

use crate::{LIGHTNINGCSS_LOADER_IDENTIFIER, config::Config};

fn parse_config(options: &str) -> Result<crate::config::RawConfig> {
  let mut options_bytes = options.as_bytes().to_vec();
  let value: OwnedValue = simd_json::from_slice(&mut options_bytes).to_rspack_result_with_detail(
    options,
    "Could not parse builtin:lightningcss-loader options",
  )?;
  validate_object_options(&value, options)?;
  simd_json::serde::from_owned_value(value).to_rspack_result_with_detail(
    options,
    "Could not parse builtin:lightningcss-loader options",
  )
}

fn validate_object_options(value: &OwnedValue, options: &str) -> Result<()> {
  let OwnedValue::Object(object) = value else {
    return Ok(());
  };

  for (field, expected) in [
    ("drafts", "Draft"),
    ("nonStandard", "NonStandard"),
    ("pseudoClasses", "PseudoClasses"),
  ] {
    let Some(value) = object.get(field) else {
      continue;
    };
    if matches!(
      value,
      OwnedValue::Object(_) | OwnedValue::Static(StaticNode::Null)
    ) {
      continue;
    }

    let offset = field_value_error_offset(options, field).unwrap_or(0);
    let (line, column) = line_column(options, offset);
    let mut error = Error::error("Could not parse builtin:lightningcss-loader options".into());
    error.labels = Some(vec![Label {
      name: Some(format!(
        "invalid type: {}, expected struct {expected} at line {line} column {column}",
        invalid_type(value)
      )),
      offset,
      len: 0,
    }]);
    error.src = Some(options.to_string());
    return Err(error);
  }

  Ok(())
}

fn invalid_type(value: &OwnedValue) -> String {
  match value {
    OwnedValue::String(value) => format!("string {}", json_stringify_str(value)),
    OwnedValue::Static(StaticNode::Bool(value)) => format!("boolean `{value}`"),
    OwnedValue::Static(value) => value.to_string(),
    OwnedValue::Array(_) => "sequence".to_string(),
    OwnedValue::Object(_) => "map".to_string(),
  }
}

fn field_value_error_offset(options: &str, field: &str) -> Option<usize> {
  let key = format!("\"{field}\"");
  let key_start = options.find(&key)?;
  let after_key = key_start + key.len();
  let colon = options[after_key..].find(':')? + after_key;
  let value_start = options[colon + 1..]
    .char_indices()
    .find_map(|(index, c)| (!c.is_whitespace()).then_some(colon + 1 + index))?;
  Some(json_value_error_offset(options, value_start))
}

fn json_value_error_offset(options: &str, value_start: usize) -> usize {
  let bytes = options.as_bytes();
  if bytes.get(value_start) == Some(&b'"') {
    let mut escaped = false;
    for (index, byte) in bytes.iter().enumerate().skip(value_start + 1) {
      if escaped {
        escaped = false;
      } else if *byte == b'\\' {
        escaped = true;
      } else if *byte == b'"' {
        return index;
      }
    }
  }

  options[value_start..]
    .char_indices()
    .find_map(|(index, c)| {
      matches!(c, ',' | '}' | ']' | '\n' | '\r').then_some(value_start + index.saturating_sub(1))
    })
    .unwrap_or_else(|| options.len().saturating_sub(1))
}

fn line_column(content: &str, offset: usize) -> (usize, usize) {
  let offset = offset.min(content.len());
  let prefix = &content[..offset];
  let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
  let column = prefix
    .rsplit('\n')
    .next()
    .map_or(0, |line| line.chars().count())
    + 1;
  (line, column)
}

#[plugin]
#[derive(Debug)]
pub struct LightningcssLoaderPlugin;

impl LightningcssLoaderPlugin {
  pub fn new() -> Self {
    Self::new_inner()
  }
}

impl Default for LightningcssLoaderPlugin {
  fn default() -> Self {
    Self::new()
  }
}

impl Plugin for LightningcssLoaderPlugin {
  fn name(&self) -> &'static str {
    "LightningcssLoaderPlugin"
  }

  fn apply(&self, ctx: &mut rspack_core::ApplyContext<'_>) -> Result<()> {
    ctx
      .normal_module_factory_hooks
      .resolve_loader
      .tap(resolve_loader::new(self));
    Ok(())
  }
}

#[plugin_hook(NormalModuleFactoryResolveLoader for LightningcssLoaderPlugin)]
pub(crate) async fn resolve_loader(
  &self,
  _context: &Context,
  _resolver: &Resolver,
  l: &ModuleRuleUseLoader,
) -> Result<Option<BoxLoader>> {
  let loader_request = &l.loader;
  let options = l.options.as_deref().unwrap_or("{}");

  if loader_request.starts_with(LIGHTNINGCSS_LOADER_IDENTIFIER) {
    let config = parse_config(options)?;
    // TODO: builtin-loader supports function
    return Ok(Some(Arc::new(crate::LightningCssLoader::new(
      None,
      Config::try_from(config)?,
      loader_request,
    ))));
  }

  Ok(None)
}
