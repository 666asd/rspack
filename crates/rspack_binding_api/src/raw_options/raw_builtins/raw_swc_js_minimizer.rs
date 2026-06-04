use napi::Either;
use napi_derive::napi;
use rspack_error::{Result, ToStringResultToRspackResultExt};
use rspack_napi::JsonValue;
use rspack_plugin_swc_js_minimizer::{
  ExtractComments, MinimizerOptions, OptionWrapper, PluginOptions,
};
use serde::de::DeserializeOwned;
use swc_core::base::BoolOrDataConfig;

use crate::asset_condition::{RawAssetConditions, into_asset_conditions};

#[derive(Debug)]
#[napi(object)]
pub struct RawExtractComments {
  pub banner: Option<Either<String, bool>>,
  pub condition: Option<String>,
  pub condition_flags: Option<String>,
}

#[derive(Debug)]
#[napi(object, object_to_js = false)]
pub struct RawSwcJsMinimizerRspackPluginOptions {
  #[napi(ts_type = "string | RegExp | (string | RegExp)[]")]
  pub test: Option<RawAssetConditions>,
  #[napi(ts_type = "string | RegExp | (string | RegExp)[]")]
  pub include: Option<RawAssetConditions>,
  #[napi(ts_type = "string | RegExp | (string | RegExp)[]")]
  pub exclude: Option<RawAssetConditions>,
  pub extract_comments: Option<RawExtractComments>,
  pub minimizer_options: RawSwcJsMinimizerOptions,
}

#[derive(Debug)]
#[napi(object, object_to_js = false)]
pub struct RawSwcJsMinimizerOptions {
  #[napi(ts_type = "any")]
  pub ecma: JsonValue,
  #[napi(ts_type = "any")]
  pub compress: JsonValue,
  #[napi(ts_type = "any")]
  pub mangle: JsonValue,
  #[napi(ts_type = "any")]
  pub format: JsonValue,
  pub module: Option<bool>,
  pub minify: Option<bool>,
}

fn try_deserialize_into<T>(value: JsonValue) -> Result<T>
where
  T: DeserializeOwned,
{
  let value = value.into_inner();
  match simd_json::serde::from_owned_value(value.clone()) {
    Ok(value) => Ok(value),
    Err(error) => {
      if let simd_json::OwnedValue::String(raw) = value
        && let Ok(parsed) = simd_json::from_reader::<_, simd_json::OwnedValue>(raw.as_bytes())
      {
        return simd_json::serde::from_owned_value(parsed).to_rspack_result();
      }

      Err::<T, _>(error).to_rspack_result()
    }
  }
}

fn into_extract_comments(c: Option<RawExtractComments>) -> Option<ExtractComments> {
  let c = c?;
  let condition = c.condition?;
  let condition_flags = c.condition_flags.unwrap_or_default();
  let banner = match c.banner {
    Some(banner) => match banner {
      Either::A(s) => OptionWrapper::Custom(s),
      Either::B(b) => {
        if b {
          OptionWrapper::Default
        } else {
          OptionWrapper::Disabled
        }
      }
    },
    None => OptionWrapper::Default,
  };

  Some(ExtractComments {
    condition,
    condition_flags,
    banner,
  })
}

impl TryFrom<RawSwcJsMinimizerRspackPluginOptions> for PluginOptions {
  type Error = rspack_error::Error;

  fn try_from(value: RawSwcJsMinimizerRspackPluginOptions) -> Result<Self> {
    let compress = try_deserialize_into::<
      BoolOrDataConfig<rspack_plugin_swc_js_minimizer::TerserCompressorOptions>,
    >(value.minimizer_options.compress)?
    .or(|| BoolOrDataConfig::from_bool(true));
    let mangle = try_deserialize_into::<
      BoolOrDataConfig<rspack_plugin_swc_js_minimizer::MangleOptions>,
    >(value.minimizer_options.mangle)?
    .or(|| BoolOrDataConfig::from_bool(true));

    let ecma = try_deserialize_into::<rspack_plugin_swc_js_minimizer::TerserEcmaVersion>(
      value.minimizer_options.ecma,
    )?;

    Ok(Self {
      extract_comments: into_extract_comments(value.extract_comments),
      test: value.test.map(into_asset_conditions),
      include: value.include.map(into_asset_conditions),
      exclude: value.exclude.map(into_asset_conditions),
      minimizer_options: MinimizerOptions {
        compress,
        mangle,
        ecma,
        format: try_deserialize_into(value.minimizer_options.format)?,
        module: value.minimizer_options.module,
        minify: value.minimizer_options.minify,
        ..Default::default()
      },
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn should_deserialize_stringified_json_minimizer_options() {
    let raw = RawSwcJsMinimizerRspackPluginOptions {
      test: None,
      include: None,
      exclude: None,
      extract_comments: None,
      minimizer_options: RawSwcJsMinimizerOptions {
        ecma: JsonValue(simd_json::OwnedValue::String("5".to_string())),
        compress: JsonValue(simd_json::OwnedValue::String(r#"{"passes":2}"#.to_string())),
        mangle: JsonValue(simd_json::OwnedValue::String("true".to_string())),
        format: JsonValue(simd_json::OwnedValue::String(
          r#"{"comments":false}"#.to_string(),
        )),
        module: None,
        minify: None,
      },
    };

    let options = PluginOptions::try_from(raw);

    assert!(options.is_ok(), "{options:?}");
  }

  #[test]
  fn should_deserialize_default_minimizer_options() {
    let raw = RawSwcJsMinimizerRspackPluginOptions {
      test: None,
      include: None,
      exclude: None,
      extract_comments: None,
      minimizer_options: RawSwcJsMinimizerOptions {
        ecma: JsonValue(5.into()),
        compress: JsonValue(simd_json::json!({ "passes": 2 })),
        mangle: JsonValue(true.into()),
        format: JsonValue(simd_json::json!({ "comments": false })),
        module: None,
        minify: None,
      },
    };

    let options = PluginOptions::try_from(raw);

    assert!(options.is_ok(), "{options:?}");
  }
}
