// BE CAREFUL:
// Add more fields to this struct should result in adding new fields to options builder.
// `impl From<Experiments> for ExperimentsBuilder` should be updated.
#[derive(Debug)]
pub struct Experiments {
  pub css: bool,
  pub defer_import: bool,
  pub runtime_mode: RuntimeOutputMode,
  pub pure_functions: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RuntimeOutputMode {
  #[default]
  Webpack,
  Compatibility,
  CompatibilityWarning,
  Rspack,
}

impl RuntimeOutputMode {
  pub fn is_runtime_requirements_proxy_enabled(self) -> bool {
    !matches!(self, Self::Webpack)
  }

  pub fn as_str(self) -> &'static str {
    match self {
      Self::Webpack => "webpack",
      Self::Compatibility => "compatibility",
      Self::CompatibilityWarning => "compatibility-warning",
      Self::Rspack => "rspack",
    }
  }
}
