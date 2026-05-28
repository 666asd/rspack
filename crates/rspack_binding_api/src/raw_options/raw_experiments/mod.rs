mod raw_incremental;

use napi_derive::napi;
pub use raw_incremental::RawIncremental;
use rspack_core::{Experiments, RuntimeOutputMode};
use rspack_regex::RspackRegex;

use super::WithFalse;

#[derive(Debug)]
#[napi(object, object_to_js = false)]
pub struct RawExperiments {
  #[napi(ts_type = "false | Array<RegExp>")]
  pub use_input_file_system: Option<WithFalse<Vec<RspackRegex>>>,
  pub css: Option<bool>,
  pub defer_import: bool,
  #[napi(ts_type = "\"webpack\" | \"compatibility\" | \"compatibility-warning\" | \"rspack\"")]
  pub runtime_mode: String,
  pub pure_functions: bool,
}

impl From<RawExperiments> for Experiments {
  fn from(value: RawExperiments) -> Self {
    Self {
      css: value.css.unwrap_or(false),
      defer_import: value.defer_import,
      runtime_mode: match value.runtime_mode.as_str() {
        "webpack" => RuntimeOutputMode::Webpack,
        "compatibility" => RuntimeOutputMode::Compatibility,
        "compatibility-warning" => RuntimeOutputMode::CompatibilityWarning,
        "rspack" => RuntimeOutputMode::Rspack,
        _ => RuntimeOutputMode::Webpack,
      },
      pure_functions: value.pure_functions,
    }
  }
}
