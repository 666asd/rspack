// BE CAREFUL:
// Add more fields to this struct should result in adding new fields to options builder.
// `impl From<Experiments> for ExperimentsBuilder` should be updated.
pub mod runtime_mode {
  #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
  pub enum RuntimeMode {
    #[default]
    Webpack,
    Rspack,
  }
}

use runtime_mode::RuntimeMode;

#[derive(Debug)]
pub struct Experiments {
  pub css: bool,
  pub defer_import: bool,
  pub pure_functions: bool,
  pub runtime_mode: RuntimeMode,
}
