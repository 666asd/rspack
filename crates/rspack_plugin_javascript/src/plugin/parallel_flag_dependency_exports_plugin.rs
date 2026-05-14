use std::collections::VecDeque;

use rayon::prelude::*;
use rspack_collections::{IdentifierMap, IdentifierSet};
use rspack_core::{
  AsyncModulesArtifact, BuildMetaExportsType, Compilation, CompilationFinishModules, DependencyId,
  EvaluatedInlinableValue, ExportInfoData, ExportNameOrSpec, ExportProvided, ExportSpec,
  ExportsInfo, ExportsInfoArtifact, ExportsInfoData, ExportsOfExportsSpec, ExportsSpec,
  ExportsSpecReexportInfo, GetTargetResult, Logger, ModuleGraph, ModuleGraphCacheArtifact,
  ModuleGraphConnection, ModuleIdentifier, Nullable, Plugin, SideEffectsStateArtifact, get_target,
  incremental::{self, IncrementalPasses},
};
use rspack_error::Result;
use rspack_hook::{plugin, plugin_hook};
use rustc_hash::FxHashMap;
use swc_core::ecma::atoms::Atom;

use super::flag_dependency_exports_plugin::FLAG_DEPENDENCY_EXPORTS_STAGE;

// This plugin computes provided exports in two phases:
//
// 1. Collect every dependency's `ExportsSpec` in parallel. While collecting, build a
//    static graph for internal reexports: `module -> modules it reexports from`.
// 2. Apply exports that do not need the graph result immediately. Each module is
//    computed against a cloned root `ExportsInfoData`; nested exports create
//    extra `ExportsInfoData` records in the module-local patch and are committed
//    back to the artifact after the parallel work finishes.
// 3. Walk the reexport graph from leaves to roots. Each wave contains modules
//    whose downstream exports are already final, so the wave can refresh and
//    apply its own exports in parallel.
// 4. Cycles cannot produce leaves. They fall back to fixed-point iteration, which
//    matches the old plugin's "repeat while changed" behavior for cyclic reexports.
//
// The important performance property is that non-nested internal reexport specs
// can opt out of phase 2 and only run during the graph walk. Other specs from
// the same module, such as local exports, are still applied immediately.
type InitialModuleExportsSpecs = Vec<CollectedExportsSpec>;
type RefreshedModuleExportsSpecs = Vec<CollectedExportsSpec>;

struct CollectedModuleExports {
  specs: CollectedModuleExportsSpecs,
  needs_backtracking: bool,
  dependencies: Option<Vec<ModuleIdentifier>>,
}

enum CollectedModuleExportsSpecs {
  Initial(InitialModuleExportsSpecs),
  Reexport,
}

impl CollectedModuleExportsSpecs {
  fn initial(&self) -> Option<&InitialModuleExportsSpecs> {
    match self {
      Self::Initial(specs) => Some(specs),
      Self::Reexport => None,
    }
  }
}

struct CollectedExportsSpec {
  dep_id: DependencyId,
  exports_spec: ExportsSpec,
}

enum ModuleExportsSpecsBatch<'a> {
  Initial(&'a [(ModuleIdentifier, CollectedModuleExports)]),
  Refreshed(Vec<(ModuleIdentifier, RefreshedModuleExportsSpecs)>),
}

struct ParallelFlagDependencyExportsState<'a> {
  mg: &'a ModuleGraph,
  mg_cache: &'a ModuleGraphCacheArtifact,
  exports_info_artifact: &'a mut ExportsInfoArtifact,
}

impl<'a> ParallelFlagDependencyExportsState<'a> {
  pub fn new(
    mg: &'a ModuleGraph,
    mg_cache: &'a ModuleGraphCacheArtifact,
    exports_info_artifact: &'a mut ExportsInfoArtifact,
  ) -> Self {
    Self {
      mg,
      mg_cache,
      exports_info_artifact,
    }
  }

  pub fn apply(&mut self, modules: IdentifierSet) {
    self.initialize_exports_info(&modules);

    let module_exports_specs = self.collect_exports_specs(&modules);
    let mut dependency_graph = build_reexport_dependency_graph(&module_exports_specs);
    self
      .process_module_exports_specs_batch(ModuleExportsSpecsBatch::Initial(&module_exports_specs));
    if dependency_graph.is_empty() {
      return;
    }
    let module_exports_specs = module_exports_specs
      .into_iter()
      .filter(|(module_id, _)| dependency_graph.contains_module(module_id))
      .collect::<IdentifierMap<_>>();

    while let Some(batch) = dependency_graph.take_leaf_modules() {
      let refreshed_exports_specs =
        self.collect_refreshed_modules_exports_specs(&batch, &module_exports_specs);
      self.process_module_exports_specs_batch(ModuleExportsSpecsBatch::Refreshed(
        refreshed_exports_specs,
      ));

      dependency_graph.finish_modules(&batch);
    }

    let cyclic_modules = dependency_graph.cyclic_modules();
    if !cyclic_modules.is_empty() {
      let mut changed = true;
      while changed {
        changed = false;
        for module_id in &cyclic_modules {
          changed |= self.process_refreshed_module_exports_specs(module_id, &module_exports_specs);
        }
      }
    }
  }

  fn process_refreshed_module_exports_specs(
    &mut self,
    module_id: &ModuleIdentifier,
    module_exports_specs: &IdentifierMap<CollectedModuleExports>,
  ) -> bool {
    if !module_exports_specs.contains_key(module_id) {
      return false;
    }
    let refreshed_exports_specs = refresh_module_exports_specs(
      self.mg,
      self.mg_cache,
      self.exports_info_artifact,
      module_id,
    );
    self.process_refreshed_exports_specs(module_id, &refreshed_exports_specs)
  }

  fn collect_refreshed_modules_exports_specs(
    &self,
    module_ids: &[ModuleIdentifier],
    module_exports_specs: &IdentifierMap<CollectedModuleExports>,
  ) -> Vec<(ModuleIdentifier, RefreshedModuleExportsSpecs)> {
    module_ids
      .par_iter()
      .filter_map(|module_id| {
        module_exports_specs.get(module_id)?;
        let refreshed_exports_specs = refresh_module_exports_specs(
          self.mg,
          self.mg_cache,
          self.exports_info_artifact,
          module_id,
        );
        Some((*module_id, refreshed_exports_specs))
      })
      .collect()
  }

  fn process_module_exports_specs_batch(&mut self, module_exports_specs: ModuleExportsSpecsBatch) {
    match module_exports_specs {
      ModuleExportsSpecsBatch::Initial(module_exports_specs) => {
        let mut patches = Vec::new();
        module_exports_specs
          .par_iter()
          .map(|(module_id, collected_exports)| {
            collected_exports
              .specs
              .initial()
              .map(|exports_specs| self.compute_exports_info_patch(*module_id, exports_specs))
          })
          .collect_into_vec(&mut patches);

        for patch in patches.into_iter().flatten() {
          commit_detached_exports_info_patch(self.exports_info_artifact, patch);
        }
      }
      ModuleExportsSpecsBatch::Refreshed(module_exports_specs) => {
        let mut patches = Vec::new();
        module_exports_specs
          .par_iter()
          .map(|(module_id, exports_specs)| {
            self.compute_exports_info_patch(*module_id, exports_specs)
          })
          .collect_into_vec(&mut patches);

        for patch in patches {
          commit_detached_exports_info_patch(self.exports_info_artifact, patch);
        }
      }
    }
  }

  fn compute_exports_info_patch(
    &self,
    module_id: ModuleIdentifier,
    exports_specs: &[CollectedExportsSpec],
  ) -> DetachedExportsInfoPatch {
    let has_nested_exports = exports_specs
      .iter()
      .any(|spec| spec.exports_spec.has_nested_exports());
    let mut root = self
      .exports_info_artifact
      .get_exports_info_data(&module_id)
      .clone();
    if !has_nested_exports {
      for exports_spec in exports_specs {
        process_exports_spec_root(
          self.mg,
          self.exports_info_artifact,
          &module_id,
          exports_spec.dep_id,
          &exports_spec.exports_spec,
          &mut root,
        );
      }
      return DetachedExportsInfoPatch::Root(root);
    }

    let mut patch = DetachedExportsInfoPatch::Nested {
      root,
      nested: Default::default(),
    };
    for exports_spec in exports_specs {
      process_exports_spec_detached(
        self.mg,
        self.exports_info_artifact,
        &module_id,
        exports_spec.dep_id,
        &exports_spec.exports_spec,
        &mut patch,
      );
    }
    patch
  }

  fn process_refreshed_exports_specs(
    &mut self,
    module_id: &ModuleIdentifier,
    exports_specs: &[CollectedExportsSpec],
  ) -> bool {
    let has_nested_exports = exports_specs
      .iter()
      .any(|spec| spec.exports_spec.has_nested_exports());
    let mut root = self
      .exports_info_artifact
      .get_exports_info_data(module_id)
      .clone();
    let mut changed = false;
    if !has_nested_exports {
      for exports_spec in exports_specs {
        changed |= process_exports_spec_root(
          self.mg,
          self.exports_info_artifact,
          module_id,
          exports_spec.dep_id,
          &exports_spec.exports_spec,
          &mut root,
        );
      }
      self
        .exports_info_artifact
        .set_exports_info_by_id(root.id(), root);
      return changed;
    }

    let mut patch = DetachedExportsInfoPatch::Nested {
      root,
      nested: Default::default(),
    };
    for exports_spec in exports_specs {
      changed |= process_exports_spec_detached(
        self.mg,
        self.exports_info_artifact,
        module_id,
        exports_spec.dep_id,
        &exports_spec.exports_spec,
        &mut patch,
      );
    }
    commit_detached_exports_info_patch(self.exports_info_artifact, patch);
    changed
  }

  fn initialize_exports_info(&mut self, modules: &IdentifierSet) {
    for module_id in modules {
      let exports_type_unset = self
        .mg
        .module_by_identifier(module_id)
        .expect("should have module")
        .build_meta()
        .exports_type
        == BuildMetaExportsType::Unset;
      let exports_info = self
        .exports_info_artifact
        .get_exports_info_data_mut(module_id);

      exports_info.reset_provide_info();
      if exports_type_unset
        && !matches!(
          exports_info.other_exports_info().provided(),
          Some(ExportProvided::Unknown)
        )
      {
        exports_info.set_has_provide_info();
        exports_info.set_unknown_exports_provided(false, None, None, None, None);
        continue;
      }

      exports_info.set_has_provide_info();
    }
  }

  fn collect_exports_specs(
    &self,
    modules: &IdentifierSet,
  ) -> Vec<(ModuleIdentifier, CollectedModuleExports)> {
    modules
      .par_iter()
      .filter_map(|module_id| {
        let collected_exports = collect_module_exports(
          module_id,
          self.mg,
          self.mg_cache,
          self.exports_info_artifact,
        )?;
        Some((*module_id, collected_exports))
      })
      .collect()
  }
}

struct ReexportDependencyGraph {
  reverse_dependencies: IdentifierMap<IdentifierSet>,
  dependency_count: IdentifierMap<usize>,
  leaf_modules: VecDeque<ModuleIdentifier>,
}

impl ReexportDependencyGraph {
  fn is_empty(&self) -> bool {
    self.dependency_count.is_empty()
  }

  fn contains_module(&self, module_id: &ModuleIdentifier) -> bool {
    self.dependency_count.contains_key(module_id)
  }

  fn take_leaf_modules(&mut self) -> Option<Vec<ModuleIdentifier>> {
    if self.leaf_modules.is_empty() {
      return None;
    }
    Some(self.leaf_modules.drain(..).collect())
  }

  fn finish_modules(&mut self, module_ids: &[ModuleIdentifier]) {
    for module_id in module_ids {
      if let Some(dependents) = self.reverse_dependencies.get(module_id) {
        for dependent in dependents {
          let Some(count) = self.dependency_count.get_mut(dependent) else {
            continue;
          };
          *count -= 1;
          if *count == 0 {
            self.leaf_modules.push_back(*dependent);
          }
        }
      }
    }
  }

  fn cyclic_modules(&self) -> Vec<ModuleIdentifier> {
    self
      .dependency_count
      .iter()
      .filter_map(|(module_id, count)| (*count > 0).then_some(*module_id))
      .collect()
  }
}

fn build_reexport_dependency_graph(
  module_exports_specs: &[(ModuleIdentifier, CollectedModuleExports)],
) -> ReexportDependencyGraph {
  let backtracking_modules = module_exports_specs
    .iter()
    .filter_map(|(module_id, collected_exports)| {
      collected_exports.needs_backtracking.then_some(*module_id)
    })
    .collect::<IdentifierSet>();
  let mut reverse_dependencies = IdentifierMap::<IdentifierSet>::default();
  let mut dependency_count = IdentifierMap::<usize>::default();
  for (module_id, collected_exports) in module_exports_specs {
    if !backtracking_modules.contains(module_id) {
      continue;
    }
    let mut count = 0;
    if let Some(dependencies) = &collected_exports.dependencies {
      for dependency in dependencies.iter().copied() {
        if dependency != *module_id && backtracking_modules.contains(&dependency) {
          reverse_dependencies
            .entry(dependency)
            .or_default()
            .insert(*module_id);
          count += 1;
        }
      }
    }
    dependency_count.insert(*module_id, count);
  }

  let leaf_modules = dependency_count
    .iter()
    .filter_map(|(module_id, count)| (*count == 0).then_some(*module_id))
    .collect::<VecDeque<_>>();

  ReexportDependencyGraph {
    reverse_dependencies,
    dependency_count,
    leaf_modules,
  }
}

fn refresh_module_exports_specs(
  mg: &ModuleGraph,
  mg_cache: &ModuleGraphCacheArtifact,
  exports_info_artifact: &ExportsInfoArtifact,
  module_id: &ModuleIdentifier,
) -> RefreshedModuleExportsSpecs {
  let Some(mgm) = mg.module_graph_module_by_identifier(module_id) else {
    return Vec::new();
  };
  let all_dependencies = mgm.all_dependencies();
  let mut refreshed_exports_specs = Vec::with_capacity(all_dependencies.len());
  for dep_id in all_dependencies {
    let Some(exports_spec) =
      mg.dependency_by_id(dep_id)
        .get_exports(mg, mg_cache, exports_info_artifact)
    else {
      continue;
    };
    push_refreshed_exports_spec(
      mg,
      exports_info_artifact,
      &mut refreshed_exports_specs,
      *dep_id,
      exports_spec,
    );
  }
  refreshed_exports_specs
}

fn push_refreshed_exports_spec(
  mg: &ModuleGraph,
  exports_info_artifact: &ExportsInfoArtifact,
  refreshed_exports_specs: &mut RefreshedModuleExportsSpecs,
  dep_id: DependencyId,
  exports_spec: ExportsSpec,
) {
  let exports_spec_ref = &exports_spec;
  let explicit_exports =
    known_exports_for_internal_unknown_reexport(mg, exports_info_artifact, &exports_spec);
  let Some((from, from_module, explicit_exports)) = explicit_exports else {
    refreshed_exports_specs.push(CollectedExportsSpec {
      dep_id,
      exports_spec,
    });
    return;
  };

  refreshed_exports_specs.push(CollectedExportsSpec {
    dep_id,
    exports_spec: unknown_exports_spec_with_extra_excludes(exports_spec_ref, &explicit_exports),
  });
  refreshed_exports_specs.push(CollectedExportsSpec {
    dep_id,
    exports_spec: known_exports_spec_for_internal_unknown_reexport(
      from,
      from_module,
      exports_spec_ref.priority,
      explicit_exports,
    ),
  });
}

fn known_exports_for_internal_unknown_reexport<'a>(
  mg: &ModuleGraph,
  exports_info_artifact: &ExportsInfoArtifact,
  exports_spec: &'a ExportsSpec,
) -> Option<(&'a ModuleGraphConnection, &'a ModuleIdentifier, Vec<Atom>)> {
  if !matches!(exports_spec.exports, ExportsOfExportsSpec::UnknownExports) {
    return None;
  }
  let from = exports_spec.from.as_ref()?;
  let from_module = from.module_identifier();
  if is_external_module(mg, from_module) {
    return None;
  }

  let explicit_exports = local_known_exports_from_module(exports_info_artifact, from_module);
  if explicit_exports.is_empty() {
    return None;
  }

  Some((from, from_module, explicit_exports))
}

fn known_exports_spec_for_internal_unknown_reexport(
  from: &ModuleGraphConnection,
  from_module: &ModuleIdentifier,
  priority: Option<u8>,
  explicit_exports: Vec<Atom>,
) -> ExportsSpec {
  ExportsSpec {
    exports: ExportsOfExportsSpec::Names(
      explicit_exports
        .into_iter()
        .map(|name| {
          ExportNameOrSpec::ExportSpec(ExportSpec {
            name: name.clone(),
            from: Some(from.to_owned()),
            export: Some(Nullable::Value(vec![name])),
            ..Default::default()
          })
        })
        .collect(),
    ),
    dependencies: Some(vec![*from_module]),
    priority,
    ..Default::default()
  }
}

fn unknown_exports_spec_with_extra_excludes(
  exports_spec: &ExportsSpec,
  extra_excludes: &[Atom],
) -> ExportsSpec {
  let mut exclude_exports = exports_spec.exclude_exports.clone().unwrap_or_default();
  exclude_exports.extend(extra_excludes.iter().cloned());
  ExportsSpec {
    exports: ExportsOfExportsSpec::UnknownExports,
    priority: exports_spec.priority,
    can_mangle: exports_spec.can_mangle,
    terminal_binding: exports_spec.terminal_binding,
    from: exports_spec.from.clone(),
    dependencies: exports_spec.dependencies.clone(),
    hide_export: exports_spec.hide_export.clone(),
    exclude_exports: Some(exclude_exports),
  }
}

fn local_known_exports_from_module(
  exports_info_artifact: &ExportsInfoArtifact,
  module_id: &ModuleIdentifier,
) -> Vec<Atom> {
  exports_info_artifact
    .get_exports_info_data(module_id)
    .exports()
    .values()
    .filter_map(|export_info| {
      let name = export_info.name()?;
      if name.as_str() == "default" || name.as_str() == "__esModule" {
        return None;
      }
      if export_info.target_is_set() || export_info.exports_info().is_some() {
        return None;
      }
      matches!(export_info.provided(), Some(ExportProvided::Provided)).then(|| name.clone())
    })
    .collect()
}

fn is_external_module(mg: &ModuleGraph, module_id: &ModuleIdentifier) -> bool {
  mg.module_by_identifier(module_id)
    .is_some_and(|module| module.as_external_module().is_some())
}

#[plugin]
#[derive(Debug, Default)]
pub struct ParallelFlagDependencyExportsPlugin;

#[plugin_hook(CompilationFinishModules for ParallelFlagDependencyExportsPlugin, stage = FLAG_DEPENDENCY_EXPORTS_STAGE)]
async fn finish_modules(
  &self,
  compilation: &Compilation,
  _async_modules_artifact: &mut AsyncModulesArtifact,
  exports_info_artifact: &mut ExportsInfoArtifact,
  _side_effects_state_artifact: &mut SideEffectsStateArtifact,
) -> Result<()> {
  let module_graph = compilation.get_module_graph();
  let modules: IdentifierSet = if let Some(mutations) = compilation
    .incremental
    .mutations_read(IncrementalPasses::FINISH_MODULES)
  {
    let modules = mutations.get_affected_modules_with_module_graph(module_graph);
    tracing::debug!(target: incremental::TRACING_TARGET, passes = %IncrementalPasses::FINISH_MODULES, %mutations, ?modules);
    let logger = compilation.get_logger("rspack.incremental.finishModules");
    logger.log(format!(
      "{} modules are affected, {} in total",
      modules.len(),
      module_graph.modules_len()
    ));
    modules
  } else {
    module_graph.modules_keys().copied().collect()
  };
  let module_graph_cache = compilation.module_graph_cache_artifact.clone();

  ParallelFlagDependencyExportsState::new(module_graph, &module_graph_cache, exports_info_artifact)
    .apply(modules);

  Ok(())
}

impl Plugin for ParallelFlagDependencyExportsPlugin {
  fn name(&self) -> &'static str {
    "ParallelFlagDependencyExportsPlugin"
  }

  fn apply(&self, ctx: &mut rspack_core::ApplyContext<'_>) -> Result<()> {
    ctx
      .compilation_hooks
      .finish_modules
      .tap(finish_modules::new(self));
    Ok(())
  }
}

fn collect_module_exports(
  module_id: &ModuleIdentifier,
  mg: &ModuleGraph,
  mg_cache: &ModuleGraphCacheArtifact,
  exports_info_artifact: &ExportsInfoArtifact,
) -> Option<CollectedModuleExports> {
  let mgm = mg.module_graph_module_by_identifier(module_id)?;
  let all_dependencies = mgm.all_dependencies();
  let mut needs_backtracking = false;
  let mut dependencies: Option<Vec<ModuleIdentifier>> = None;

  for id in all_dependencies {
    let dependency = mg.dependency_by_id(id);
    let Some(reexport_info) = dependency.get_reexport_info(mg, mg_cache, exports_info_artifact)
    else {
      continue;
    };
    if let ExportsSpecReexportInfo::Reexport(reexport_dependency) = reexport_info {
      needs_backtracking = true;
      push_unique_reexport_dependency(&mut dependencies, reexport_dependency);
    }
  }

  if needs_backtracking {
    return Some(CollectedModuleExports {
      specs: CollectedModuleExportsSpecs::Reexport,
      needs_backtracking,
      dependencies,
    });
  }

  let mut specs: Option<Vec<CollectedExportsSpec>> = None;
  for id in all_dependencies {
    let Some(exports_spec) =
      mg.dependency_by_id(id)
        .get_exports(mg, mg_cache, exports_info_artifact)
    else {
      continue;
    };
    if let Some(reexport_dependencies) = &exports_spec.dependencies
      && !reexport_dependencies.is_empty()
    {
      needs_backtracking = true;
      for dependency in reexport_dependencies.iter().copied() {
        push_unique_reexport_dependency(&mut dependencies, dependency);
      }
    }
    specs
      .get_or_insert_with(|| Vec::with_capacity(all_dependencies.len()))
      .push(CollectedExportsSpec {
        dep_id: *id,
        exports_spec,
      });
  }
  specs.map(|specs| CollectedModuleExports {
    specs: CollectedModuleExportsSpecs::Initial(specs),
    needs_backtracking,
    dependencies,
  })
}

fn push_unique_reexport_dependency(
  dependencies: &mut Option<Vec<ModuleIdentifier>>,
  dependency: ModuleIdentifier,
) {
  match dependencies {
    Some(dependencies) => {
      if !dependencies.contains(&dependency) {
        dependencies.push(dependency);
      }
    }
    None => {
      *dependencies = Some(vec![dependency]);
    }
  }
}

#[derive(Debug, Clone)]
struct DefaultExportInfo<'a> {
  can_mangle: Option<bool>,
  terminal_binding: bool,
  from: Option<&'a ModuleGraphConnection>,
  priority: Option<u8>,
}

struct ProcessExportsContext<'a> {
  mg: &'a ModuleGraph,
  exports_info_artifact: &'a ExportsInfoArtifact,
  module_id: &'a ModuleIdentifier,
}

struct ParsedExportSpec<'a> {
  name: &'a Atom,
  can_mangle: Option<bool>,
  terminal_binding: bool,
  exports: Option<&'a Vec<ExportNameOrSpec>>,
  from: Option<&'a ModuleGraphConnection>,
  from_export: Option<&'a Nullable<Vec<Atom>>>,
  priority: Option<u8>,
  hidden: bool,
  inlinable: Option<&'a EvaluatedInlinableValue>,
}

impl<'a> ParsedExportSpec<'a> {
  pub fn new(
    export_name_or_spec: &'a ExportNameOrSpec,
    global_export_info: &'a DefaultExportInfo,
  ) -> Self {
    match export_name_or_spec {
      ExportNameOrSpec::String(name) => Self {
        name,
        can_mangle: global_export_info.can_mangle,
        terminal_binding: global_export_info.terminal_binding,
        exports: None,
        from: global_export_info.from,
        from_export: None,
        priority: global_export_info.priority,
        hidden: false,
        inlinable: None,
      },
      ExportNameOrSpec::ExportSpec(spec) => Self {
        name: &spec.name,
        can_mangle: spec.can_mangle.or(global_export_info.can_mangle),
        terminal_binding: spec
          .terminal_binding
          .unwrap_or(global_export_info.terminal_binding),
        exports: spec.exports.as_ref(),
        from: spec.from.as_ref().or(global_export_info.from),
        from_export: spec.export.as_ref(),
        priority: spec.priority.or(global_export_info.priority),
        hidden: spec.hidden.unwrap_or(false),
        inlinable: spec.inlinable.as_ref(),
      },
    }
  }
}

enum DetachedExportsInfoPatch {
  Root(ExportsInfoData),
  Nested {
    root: ExportsInfoData,
    nested: FxHashMap<ExportsInfo, ExportsInfoData>,
  },
}

impl DetachedExportsInfoPatch {
  fn root(&self) -> &ExportsInfoData {
    match self {
      Self::Root(root) | Self::Nested { root, .. } => root,
    }
  }

  fn root_mut(&mut self) -> &mut ExportsInfoData {
    match self {
      Self::Root(root) | Self::Nested { root, .. } => root,
    }
  }

  fn get<'a>(
    &'a self,
    exports_info: ExportsInfo,
    artifact: &'a ExportsInfoArtifact,
  ) -> &'a ExportsInfoData {
    let root = self.root();
    if exports_info == root.id() {
      return root;
    }
    match self {
      Self::Root(_) => artifact.get_exports_info_by_id(&exports_info),
      Self::Nested { nested, .. } => nested
        .get(&exports_info)
        .unwrap_or_else(|| artifact.get_exports_info_by_id(&exports_info)),
    }
  }

  fn get_mut(
    &mut self,
    exports_info: ExportsInfo,
    artifact: &ExportsInfoArtifact,
  ) -> &mut ExportsInfoData {
    if exports_info == self.root().id() {
      return self.root_mut();
    }
    match self {
      Self::Root(_) => {
        unreachable!("root-only exports info patch cannot mutate nested exports info")
      }
      Self::Nested { nested, .. } => nested
        .entry(exports_info)
        .or_insert_with(|| artifact.get_exports_info_by_id(&exports_info).clone()),
    }
  }

  fn ensure_local_nested(&mut self, exports_info: ExportsInfo, artifact: &ExportsInfoArtifact) {
    if exports_info == self.root().id() {
      return;
    }
    match self {
      Self::Root(_) => {
        unreachable!("root-only exports info patch cannot cache nested exports info")
      }
      Self::Nested { nested, .. } => {
        nested
          .entry(exports_info)
          .or_insert_with(|| artifact.get_exports_info_by_id(&exports_info).clone());
      }
    }
  }

  fn nested_exports_info(
    &self,
    artifact: &ExportsInfoArtifact,
    mut exports_info: ExportsInfo,
    name: Option<&[Atom]>,
  ) -> Option<ExportsInfo> {
    for name in name.unwrap_or_default() {
      let data = self.get(exports_info, artifact);
      let export_info = data
        .named_exports(name)
        .unwrap_or_else(|| data.other_exports_info());
      exports_info = export_info.exports_info()?;
    }
    Some(exports_info)
  }

  fn insert_nested(&mut self, exports_info: ExportsInfo, exports_info_data: ExportsInfoData) {
    match self {
      Self::Root(_) => {
        unreachable!("root-only exports info patch cannot insert nested exports info")
      }
      Self::Nested { nested, .. } => {
        nested.insert(exports_info, exports_info_data);
      }
    }
  }
}

fn commit_detached_exports_info_patch(
  exports_info_artifact: &mut ExportsInfoArtifact,
  patch: DetachedExportsInfoPatch,
) {
  match patch {
    DetachedExportsInfoPatch::Root(root) => {
      exports_info_artifact.set_exports_info_by_id(root.id(), root);
    }
    DetachedExportsInfoPatch::Nested { root, nested } => {
      exports_info_artifact.set_exports_info_by_id(root.id(), root);
      exports_info_artifact.extend(nested);
    }
  }
}

fn process_exports_spec_root(
  mg: &ModuleGraph,
  exports_info_artifact: &ExportsInfoArtifact,
  module_id: &ModuleIdentifier,
  dep_id: DependencyId,
  export_desc: &ExportsSpec,
  exports_info: &mut ExportsInfoData,
) -> bool {
  let mut changed = false;

  let exports = &export_desc.exports;
  let global_can_mangle = &export_desc.can_mangle;
  let global_from = export_desc.from.as_ref();
  let global_priority = &export_desc.priority;
  let global_terminal_binding = export_desc.terminal_binding.unwrap_or(false);
  if let Some(hide_export) = &export_desc.hide_export {
    for name in hide_export.iter() {
      exports_info
        .ensure_owned_export_info(name)
        .unset_target(&dep_id);
    }
  }
  match exports {
    ExportsOfExportsSpec::UnknownExports => {
      changed |= exports_info.set_unknown_exports_provided(
        global_can_mangle.unwrap_or_default(),
        export_desc.exclude_exports.as_ref(),
        global_from.map(|_| dep_id),
        global_from.map(|_| dep_id),
        *global_priority,
      );
    }
    ExportsOfExportsSpec::NoExports => {}
    ExportsOfExportsSpec::Names(ele) => {
      changed |= merge_exports_root(
        mg,
        exports_info_artifact,
        module_id,
        exports_info,
        ele,
        DefaultExportInfo {
          can_mangle: *global_can_mangle,
          terminal_binding: global_terminal_binding,
          from: global_from,
          priority: *global_priority,
        },
        dep_id,
      );
    }
  }

  changed
}

fn process_exports_spec_detached(
  mg: &ModuleGraph,
  exports_info_artifact: &ExportsInfoArtifact,
  module_id: &ModuleIdentifier,
  dep_id: DependencyId,
  export_desc: &ExportsSpec,
  patch: &mut DetachedExportsInfoPatch,
) -> bool {
  let mut changed = false;

  let exports = &export_desc.exports;
  let global_can_mangle = &export_desc.can_mangle;
  let global_from = export_desc.from.as_ref();
  let global_priority = &export_desc.priority;
  let global_terminal_binding = export_desc.terminal_binding.unwrap_or(false);
  if let Some(hide_export) = &export_desc.hide_export {
    let exports_info = patch.get_mut(
      exports_info_artifact.get_exports_info(module_id),
      exports_info_artifact,
    );
    for name in hide_export.iter() {
      exports_info
        .ensure_owned_export_info(name)
        .unset_target(&dep_id);
    }
  }
  match exports {
    ExportsOfExportsSpec::UnknownExports => {
      changed |= patch
        .get_mut(
          exports_info_artifact.get_exports_info(module_id),
          exports_info_artifact,
        )
        .set_unknown_exports_provided(
          global_can_mangle.unwrap_or_default(),
          export_desc.exclude_exports.as_ref(),
          global_from.map(|_| dep_id),
          global_from.map(|_| dep_id),
          *global_priority,
        );
    }
    ExportsOfExportsSpec::NoExports => {}
    ExportsOfExportsSpec::Names(ele) => {
      let context = ProcessExportsContext {
        mg,
        exports_info_artifact,
        module_id,
      };
      changed |= merge_exports_detached(
        &context,
        patch,
        exports_info_artifact.get_exports_info(module_id),
        ele,
        DefaultExportInfo {
          can_mangle: *global_can_mangle,
          terminal_binding: global_terminal_binding,
          from: global_from,
          priority: *global_priority,
        },
        dep_id,
        false,
      );
    }
  }

  changed
}

fn merge_exports_root(
  mg: &ModuleGraph,
  exports_info_artifact: &ExportsInfoArtifact,
  module_id: &ModuleIdentifier,
  exports_info: &mut ExportsInfoData,
  exports: &[ExportNameOrSpec],
  global_export_info: DefaultExportInfo,
  dep_id: DependencyId,
) -> bool {
  let mut changed = false;
  for export_name_or_spec in exports {
    let ParsedExportSpec {
      name,
      can_mangle,
      terminal_binding,
      exports,
      from,
      from_export,
      priority,
      hidden,
      inlinable,
    } = ParsedExportSpec::new(export_name_or_spec, &global_export_info);

    debug_assert!(
      exports.is_none(),
      "root exports fast path should not receive nested exports"
    );

    {
      let export_info = exports_info.ensure_owned_export_info(name);
      changed |= set_export_base_info(export_info, can_mangle, terminal_binding, inlinable);
      changed |= set_export_target(
        export_info,
        from,
        from_export,
        priority,
        hidden,
        dep_id,
        name,
      );
    }

    let target_exports_info = {
      let export_info = exports_info
        .named_exports(name)
        .expect("should have named export");
      find_target_exports_info_root(
        mg,
        exports_info_artifact,
        module_id,
        exports_info,
        export_info,
      )
    };

    let export_info = exports_info
      .named_exports_mut(name)
      .expect("should have named export");
    let should_update_exports_info =
      target_exports_info.is_some() || !export_info.exports_info_owned();
    if export_info.exports_info() != target_exports_info && should_update_exports_info {
      export_info.set_exports_info(target_exports_info);
      changed = true;
    }
  }
  changed
}

fn merge_exports_detached(
  context: &ProcessExportsContext,
  patch: &mut DetachedExportsInfoPatch,
  exports_info: ExportsInfo,
  exports: &[ExportNameOrSpec],
  global_export_info: DefaultExportInfo,
  dep_id: DependencyId,
  in_nested_exports: bool,
) -> bool {
  let mut changed = false;
  for export_name_or_spec in exports {
    let ParsedExportSpec {
      name,
      can_mangle,
      terminal_binding,
      exports,
      from,
      from_export,
      priority,
      hidden,
      inlinable,
    } = ParsedExportSpec::new(export_name_or_spec, &global_export_info);

    {
      let export_info = patch
        .get_mut(exports_info, context.exports_info_artifact)
        .ensure_owned_export_info(name);
      changed |= set_export_base_info(export_info, can_mangle, terminal_binding, inlinable);
    }

    if let Some(exports) = exports {
      let nested_exports_info =
        ensure_nested_exports_info(patch, context.exports_info_artifact, exports_info, name);
      changed |= merge_exports_detached(
        context,
        patch,
        nested_exports_info,
        exports,
        global_export_info.clone(),
        dep_id,
        true,
      );
    }

    {
      let export_info = patch
        .get_mut(exports_info, context.exports_info_artifact)
        .named_exports_mut(name)
        .expect("should have named export");
      changed |= set_export_target(
        export_info,
        from,
        from_export,
        priority,
        hidden,
        dep_id,
        name,
      );
    }

    let target_exports_info = {
      let export_info = patch
        .get(exports_info, context.exports_info_artifact)
        .named_exports(name)
        .expect("should have named export");
      find_target_exports_info_detached(
        context.mg,
        context.exports_info_artifact,
        context.module_id,
        patch,
        export_info,
      )
    };

    let export_info = patch
      .get_mut(exports_info, context.exports_info_artifact)
      .named_exports_mut(name)
      .expect("should have named export");
    let should_update_exports_info = if in_nested_exports {
      export_info.exports_info_owned() && target_exports_info.is_some()
    } else {
      target_exports_info.is_some() || !export_info.exports_info_owned()
    };
    if export_info.exports_info() != target_exports_info && should_update_exports_info {
      export_info.set_exports_info(target_exports_info);
      changed = true;
    }
  }
  changed
}

fn ensure_nested_exports_info(
  patch: &mut DetachedExportsInfoPatch,
  exports_info: &ExportsInfoArtifact,
  parent_exports_info: ExportsInfo,
  name: &Atom,
) -> ExportsInfo {
  let existing_nested_exports_info = {
    let export_info = patch
      .get_mut(parent_exports_info, exports_info)
      .ensure_owned_export_info(name);
    export_info.exports_info_owned().then(|| {
      export_info
        .exports_info()
        .expect("should have exports_info when exports_info is owned")
    })
  };
  if let Some(nested_exports_info) = existing_nested_exports_info {
    patch.ensure_local_nested(nested_exports_info, exports_info);
    return nested_exports_info;
  }

  let mut new_exports_info = ExportsInfoData::default();
  let new_exports_info_id = new_exports_info.id();
  new_exports_info.set_has_provide_info();
  patch.insert_nested(new_exports_info_id, new_exports_info);

  let export_info = patch
    .get_mut(parent_exports_info, exports_info)
    .named_exports_mut(name)
    .expect("should have named export");
  export_info.set_exports_info(Some(new_exports_info_id));
  export_info.set_exports_info_owned(true);

  new_exports_info_id
}

fn set_export_base_info(
  export_info: &mut ExportInfoData,
  can_mangle: Option<bool>,
  terminal_binding: bool,
  inlinable: Option<&EvaluatedInlinableValue>,
) -> bool {
  let mut changed = false;
  if let Some(provided) = export_info.provided()
    && matches!(
      provided,
      ExportProvided::NotProvided | ExportProvided::Unknown
    )
  {
    export_info.set_provided(Some(ExportProvided::Provided));
    changed = true;
  }

  if Some(false) != export_info.can_mangle_provide() && can_mangle == Some(false) {
    export_info.set_can_mangle_provide(Some(false));
    changed = true;
  }

  if let Some(inlined) = inlinable
    && export_info.can_inline_provide().is_none()
  {
    export_info.set_can_inline_provide(Some(inlined.clone()));
    changed = true;
  }

  if terminal_binding && !export_info.terminal_binding() {
    export_info.set_terminal_binding(true);
    changed = true;
  }
  changed
}

fn set_export_target(
  export_info: &mut ExportInfoData,
  from: Option<&ModuleGraphConnection>,
  from_export: Option<&Nullable<Vec<Atom>>>,
  priority: Option<u8>,
  hidden: bool,
  dep_id: DependencyId,
  name: &Atom,
) -> bool {
  let mut changed = false;
  if let Some(from) = from {
    changed |= if hidden {
      export_info.unset_target(&dep_id)
    } else {
      let fallback = rspack_core::Nullable::Value(vec![name.clone()]);
      let export_name = if let Some(from) = from_export {
        Some(from)
      } else {
        Some(&fallback)
      };
      export_info.set_target(
        Some(dep_id),
        Some(from.dependency_id),
        export_name,
        priority,
      )
    }
  }
  changed
}

fn find_target_exports_info_root(
  mg: &ModuleGraph,
  exports_info_artifact: &ExportsInfoArtifact,
  module_id: &ModuleIdentifier,
  root_exports_info: &ExportsInfoData,
  export_info: &ExportInfoData,
) -> Option<ExportsInfo> {
  let target = get_target(
    export_info,
    mg,
    exports_info_artifact,
    &|_| true,
    &mut Default::default(),
  );

  if let Some(GetTargetResult::Target(target)) = target {
    if &target.module == module_id {
      return root_exports_info
        .get_nested_exports_info(exports_info_artifact, target.export.as_deref())
        .map(|data| data.id());
    }
    return exports_info_artifact
      .get_exports_info_data(&target.module)
      .get_nested_exports_info(exports_info_artifact, target.export.as_deref())
      .map(|data| data.id());
  }

  None
}

fn find_target_exports_info_detached(
  mg: &ModuleGraph,
  exports_info_artifact: &ExportsInfoArtifact,
  module_id: &ModuleIdentifier,
  patch: &DetachedExportsInfoPatch,
  export_info: &ExportInfoData,
) -> Option<ExportsInfo> {
  let target = get_target(
    export_info,
    mg,
    exports_info_artifact,
    &|_| true,
    &mut Default::default(),
  );

  if let Some(GetTargetResult::Target(target)) = target {
    let target_module_exports_info = exports_info_artifact.get_exports_info(&target.module);
    if &target.module == module_id {
      return patch.nested_exports_info(
        exports_info_artifact,
        target_module_exports_info,
        target.export.as_deref(),
      );
    }
    return exports_info_artifact
      .get_exports_info_data(&target.module)
      .get_nested_exports_info(exports_info_artifact, target.export.as_deref())
      .map(|data| data.id());
  }

  None
}
