mod raw_incremental;

use napi_derive::napi;
pub use raw_incremental::RawIncremental;
use rspack_core::{Experiments, runtime_mode::RuntimeMode};
use rspack_error::Result;
use rspack_regex::RspackRegex;

use super::WithFalse;

#[derive(Debug)]
#[napi(object, object_to_js = false)]
pub struct RawExperiments {
  #[napi(ts_type = "false | Array<RegExp>")]
  pub use_input_file_system: Option<WithFalse<Vec<RspackRegex>>>,
  pub css: Option<bool>,
  pub defer_import: bool,
  pub pure_functions: bool,
  #[napi(ts_type = "\"webpack\" | \"rspack\"")]
  pub runtime_mode: Option<String>,
}

impl TryFrom<RawExperiments> for Experiments {
  type Error = rspack_error::Error;

  fn try_from(value: RawExperiments) -> Result<Self> {
    let runtime_mode = if value.runtime_mode.as_deref() == Some("rspack") {
      RuntimeMode::Rspack
    } else {
      RuntimeMode::Webpack
    };

    Ok(Self {
      css: value.css.unwrap_or(false),
      defer_import: value.defer_import,
      pure_functions: value.pure_functions,
      runtime_mode,
    })
  }
}
