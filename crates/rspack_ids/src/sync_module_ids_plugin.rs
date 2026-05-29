use std::{collections::BTreeMap, fs, io::ErrorKind, sync::Mutex};

use derive_more::Debug;
use rspack_core::{
  ChunkGraph, Compilation, CompilationBeforeModuleIds, CompilationModuleIds, LibIdentOptions,
  ModuleId, ModuleIdsArtifact, Plugin,
};
use rspack_error::{Diagnostic, Result, error};
use rspack_hook::{plugin, plugin_hook};

use crate::id_helpers::{ModuleFilterFn, get_used_module_ids_and_modules_with_async_filter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncModuleIdsPluginMode {
  Read,
  Create,
  Merge,
  Update,
}

impl Default for SyncModuleIdsPluginMode {
  fn default() -> Self {
    Self::Merge
  }
}

#[derive(Debug, Clone)]
pub struct SyncModuleIdsPluginOptions {
  pub path: String,
  pub context: Option<String>,
  #[debug(skip)]
  pub test: Option<ModuleFilterFn>,
  pub mode: SyncModuleIdsPluginMode,
}

#[derive(Debug, Default)]
struct SyncModuleIdsState {
  data: Option<BTreeMap<String, ModuleId>>,
  data_changed: bool,
}

#[plugin]
#[derive(Debug)]
pub struct SyncModuleIdsPlugin {
  path: String,
  context: Option<String>,
  #[debug(skip)]
  test: Option<ModuleFilterFn>,
  mode: SyncModuleIdsPluginMode,
  #[debug(skip)]
  state: Mutex<SyncModuleIdsState>,
}

impl SyncModuleIdsPlugin {
  pub fn new(options: SyncModuleIdsPluginOptions) -> Self {
    Self::new_inner(
      options.path,
      options.context,
      options.test,
      options.mode,
      Mutex::new(Default::default()),
    )
  }

  fn read_and_write(&self) -> bool {
    matches!(
      self.mode,
      SyncModuleIdsPluginMode::Merge | SyncModuleIdsPluginMode::Update
    )
  }

  fn need_read(&self) -> bool {
    self.read_and_write() || self.mode == SyncModuleIdsPluginMode::Read
  }

  fn need_write(&self) -> bool {
    self.read_and_write() || self.mode == SyncModuleIdsPluginMode::Create
  }

  fn need_prune(&self) -> bool {
    self.mode == SyncModuleIdsPluginMode::Update
  }

  fn read_data_from_disk(&self) -> Result<BTreeMap<String, ModuleId>> {
    let content = match fs::read_to_string(&self.path) {
      Ok(content) => content,
      Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Default::default()),
      Err(err) => {
        return Err(error!(
          "SyncModuleIdsPlugin: Failed to read '{}': {err}",
          self.path
        ));
      }
    };

    let raw: BTreeMap<String, serde_json::Value> =
      serde_json::from_str(&content).map_err(|err| {
        error!(
          "SyncModuleIdsPlugin: Failed to parse module ids from '{}': {err}",
          self.path
        )
      })?;

    Ok(
      raw
        .into_iter()
        .filter_map(|(name, value)| value_to_module_id(value).map(|id| (name, id)))
        .collect(),
    )
  }

  fn ensure_data_loaded(&self) -> Result<BTreeMap<String, ModuleId>> {
    let mut state = self.state.lock().expect("should lock SyncModuleIdsPlugin");
    if state.data.is_none() {
      state.data = Some(self.read_data_from_disk()?);
      state.data_changed = false;
    }
    Ok(state.data.clone().unwrap_or_default())
  }

  fn write_data_to_disk(&self) -> Result<()> {
    let state = self.state.lock().expect("should lock SyncModuleIdsPlugin");
    if !state.data_changed {
      return Ok(());
    }
    let Some(data) = &state.data else {
      return Ok(());
    };

    let json = data
      .iter()
      .map(|(name, id)| (name.clone(), module_id_to_value(id)))
      .collect::<BTreeMap<_, _>>();
    let content = serde_json::to_string(&json).map_err(|err| {
      error!(
        "SyncModuleIdsPlugin: Failed to serialize module ids for '{}': {err}",
        self.path
      )
    })?;
    fs::write(&self.path, content).map_err(|err| {
      error!(
        "SyncModuleIdsPlugin: Failed to write '{}': {err}",
        self.path
      )
    })?;
    Ok(())
  }
}

fn value_to_module_id(value: serde_json::Value) -> Option<ModuleId> {
  match value {
    serde_json::Value::String(value) => Some(value.into()),
    serde_json::Value::Number(value) => value
      .as_u64()
      .and_then(|value| u32::try_from(value).ok().map(ModuleId::from)),
    _ => None,
  }
}

fn module_id_to_value(id: &ModuleId) -> serde_json::Value {
  if let Some(id) = id.as_number() {
    serde_json::Value::Number(id.into())
  } else {
    serde_json::Value::String(id.to_string())
  }
}

#[plugin_hook(CompilationBeforeModuleIds for SyncModuleIdsPlugin)]
async fn before_module_ids(
  &self,
  compilation: &Compilation,
  _modules: &rspack_collections::IdentifierSet,
  module_ids: &mut ModuleIdsArtifact,
) -> Result<()> {
  if !self.need_read() {
    return Ok(());
  }

  let data = self.ensure_data_loaded()?;
  if data.is_empty() {
    return Ok(());
  }

  let context = self
    .context
    .as_deref()
    .unwrap_or(compilation.options.context.as_ref());
  let (mut used_ids, modules) =
    get_used_module_ids_and_modules_with_async_filter(compilation, module_ids, self.test.as_ref())
      .await?;
  let module_graph = compilation.get_module_graph();

  for module_identifier in modules {
    let Some(module) = module_graph.module_by_identifier(&module_identifier) else {
      continue;
    };
    let Some(name) = module.lib_ident(LibIdentOptions { context }) else {
      continue;
    };
    let Some(id) = data.get(name.as_ref()).cloned() else {
      continue;
    };
    let id_as_string = id.to_string();
    if used_ids.contains(&id_as_string) {
      return Err(error!(
        "SyncModuleIdsPlugin: Unable to restore id '{}' from '{}' as it's already used.",
        id, self.path
      ));
    }
    ChunkGraph::set_module_id(module_ids, module_identifier, id);
    used_ids.insert(id_as_string);
  }

  Ok(())
}

#[plugin_hook(CompilationModuleIds for SyncModuleIdsPlugin)]
async fn module_ids(
  &self,
  compilation: &Compilation,
  module_ids: &mut ModuleIdsArtifact,
  _diagnostics: &mut Vec<Diagnostic>,
) -> Result<()> {
  if !self.need_write() {
    return Ok(());
  }

  let context = self
    .context
    .as_deref()
    .unwrap_or(compilation.options.context.as_ref());
  let mut entries = Vec::new();
  for (_, module) in compilation.get_module_graph().modules() {
    if let Some(test) = &self.test
      && !test(compilation.compiler_id(), module.as_ref()).await?
    {
      continue;
    }
    let Some(name) = module.lib_ident(LibIdentOptions { context }) else {
      continue;
    };
    let Some(id) = ChunkGraph::get_module_id(module_ids, module.identifier()).cloned() else {
      continue;
    };
    entries.push((name.into_owned(), id));
  }

  let old_data = if self.mode == SyncModuleIdsPluginMode::Create {
    BTreeMap::default()
  } else {
    self.ensure_data_loaded()?
  };
  let mut data = if self.need_prune() || self.mode == SyncModuleIdsPluginMode::Create {
    BTreeMap::default()
  } else {
    old_data.clone()
  };

  let mut data_changed = data.len() != old_data.len();
  for (name, id) in entries {
    if old_data.get(&name) != Some(&id) {
      data_changed = true;
    }
    data.insert(name, id);
  }
  if data.len() != old_data.len() {
    data_changed = true;
  }

  {
    let mut state = self.state.lock().expect("should lock SyncModuleIdsPlugin");
    state.data = Some(data);
    state.data_changed |= data_changed;
  }
  self.write_data_to_disk()?;

  Ok(())
}

impl Plugin for SyncModuleIdsPlugin {
  fn name(&self) -> &'static str {
    "SyncModuleIdsPlugin"
  }

  fn apply(&self, ctx: &mut rspack_core::ApplyContext<'_>) -> Result<()> {
    ctx
      .compilation_hooks
      .before_module_ids
      .tap(before_module_ids::new(self));
    ctx.compilation_hooks.module_ids.tap(module_ids::new(self));
    Ok(())
  }
}
