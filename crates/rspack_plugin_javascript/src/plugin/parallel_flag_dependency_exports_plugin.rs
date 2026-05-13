use std::collections::VecDeque;

use rayon::prelude::*;
use rspack_collections::{IdentifierMap, IdentifierSet};
use rspack_core::{
  AsyncModulesArtifact, BuildMetaExportsType, Compilation, CompilationFinishModules, DependencyId,
  EvaluatedInlinableValue, ExportInfo, ExportInfoData, ExportNameOrSpec, ExportProvided,
  ExportSpec, ExportsInfo, ExportsInfoArtifact, ExportsInfoData, ExportsOfExportsSpec, ExportsSpec,
  GetTargetResult, Logger, ModuleGraph, ModuleGraphCacheArtifact, ModuleGraphConnection,
  ModuleIdentifier, Nullable, Plugin, SideEffectsStateArtifact, get_target,
  incremental::{self, IncrementalPasses},
};
use rspack_error::Result;
use rspack_hook::{plugin, plugin_hook};
use swc_core::ecma::atoms::Atom;

use super::flag_dependency_exports_plugin::FLAG_DEPENDENCY_EXPORTS_STAGE;

// This plugin computes provided exports in two phases:
//
// 1. Collect every dependency's `ExportsSpec` in parallel. While collecting, build a
//    static graph for internal reexports: `module -> modules it reexports from`.
// 2. Apply exports that do not need the graph result immediately. Non-nested
//    modules can be updated on cloned `ExportsInfoData` in parallel; nested specs
//    still use the mutable artifact path because they create/update child
//    `ExportsInfo`.
// 3. Walk the reexport graph from leaves to roots. Each wave contains modules
//    whose downstream exports are already final, so the wave can refresh and
//    apply its own exports in parallel.
// 4. Cycles cannot produce leaves. They fall back to fixed-point iteration, which
//    matches the old plugin's "repeat while changed" behavior for cyclic reexports.
//
// The important performance property is that non-nested internal reexport specs
// can opt out of phase 2 and only run during the graph walk. Other specs from
// the same module, such as local exports, are still applied immediately.
type ModuleExportsSpecs = Vec<CollectedExportsSpec>;
type RefreshedModuleExportsSpecs<'a> = Vec<RefreshedExportsSpec<'a>>;

struct CollectedModuleExports {
  specs: ModuleExportsSpecs,
  meta: ModuleExportsMeta,
  dependencies: Option<Vec<ModuleIdentifier>>,
}

struct CollectedExportsSpec {
  dep_id: DependencyId,
  exports_spec: ExportsSpec,
  apply_initial: bool,
  refresh_on_backtrack: bool,
}

enum RefreshedExportsSpec<'a> {
  Borrowed(DependencyId, &'a ExportsSpec),
  Owned(DependencyId, ExportsSpec),
}

impl<'a> RefreshedExportsSpec<'a> {
  fn dep_id(&self) -> DependencyId {
    match self {
      Self::Borrowed(dep_id, _) | Self::Owned(dep_id, _) => *dep_id,
    }
  }

  fn exports_spec(&self) -> &ExportsSpec {
    match self {
      Self::Borrowed(_, exports_spec) => exports_spec,
      Self::Owned(_, exports_spec) => exports_spec,
    }
  }
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

    let mut module_exports_specs = self.collect_exports_specs(&modules);
    let (dependency_graph, initial_modules) =
      build_reexport_dependency_graph(&module_exports_specs);
    self.process_initial_exports_specs(initial_modules.iter().filter_map(
      |(module_id, has_nested_exports)| {
        let collected_exports = module_exports_specs.get(module_id)?;
        Some((*module_id, &collected_exports.specs, *has_nested_exports))
      },
    ));
    module_exports_specs
      .retain(|module_id, _| dependency_graph.dependency_count.contains_key(module_id));

    let mut remaining_dependency_count = dependency_graph.dependency_count.clone();
    let mut queue = dependency_graph.leaf_modules.clone();

    while !queue.is_empty() {
      let ready_modules = std::mem::take(&mut queue);
      let mut batch = Vec::with_capacity(ready_modules.len());
      for module_id in ready_modules {
        batch.push(module_id);
      }

      let refreshed_exports_specs = self.collect_refreshed_modules_exports_specs(
        &batch,
        &module_exports_specs,
        &dependency_graph,
      );
      self.process_refreshed_module_exports_specs_batch(
        refreshed_exports_specs
          .iter()
          .map(|(module_id, specs, meta)| (*module_id, specs, meta.has_nested_exports)),
      );

      for module_id in batch {
        if let Some(dependents) = dependency_graph.reverse_dependencies.get(&module_id) {
          for dependent in dependents {
            let Some(count) = remaining_dependency_count.get_mut(dependent) else {
              continue;
            };
            *count -= 1;
            if *count == 0 {
              queue.push_back(*dependent);
            }
          }
        }
      }
    }

    let cyclic_modules = dependency_graph
      .dependency_count
      .iter()
      .filter(|(module_id, _)| {
        remaining_dependency_count
          .get(module_id)
          .is_some_and(|count| *count > 0)
      })
      .map(|(module_id, _)| *module_id)
      .collect::<Vec<_>>();
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

  fn process_initial_exports_specs<'b>(
    &mut self,
    modules: impl IntoIterator<Item = (ModuleIdentifier, &'b ModuleExportsSpecs, bool)>,
  ) {
    self.process_module_exports_specs_batch(modules);
  }

  fn process_refreshed_module_exports_specs(
    &mut self,
    module_id: &ModuleIdentifier,
    module_exports_specs: &IdentifierMap<CollectedModuleExports>,
  ) -> bool {
    let Some(collected_exports) = module_exports_specs.get(module_id) else {
      return false;
    };
    let mut refreshed_exports_specs = refresh_exports_specs_from_initial(
      self.mg,
      self.mg_cache,
      self.exports_info_artifact,
      &collected_exports.specs,
    );
    refreshed_exports_specs.sort_by_key(|exports_spec| {
      refreshed_exports_spec_order(self.mg, exports_spec.exports_spec())
    });
    self.process_refreshed_exports_specs(module_id, &refreshed_exports_specs)
  }

  fn collect_refreshed_modules_exports_specs<'b>(
    &self,
    module_ids: &[ModuleIdentifier],
    module_exports_specs: &'b IdentifierMap<CollectedModuleExports>,
    dependency_graph: &ReexportDependencyGraph,
  ) -> Vec<(
    ModuleIdentifier,
    RefreshedModuleExportsSpecs<'b>,
    ModuleExportsMeta,
  )> {
    module_ids
      .par_iter()
      .filter_map(|module_id| {
        let collected_exports = module_exports_specs.get(module_id)?;
        let mut refreshed_exports_specs = refresh_exports_specs_from_initial(
          self.mg,
          self.mg_cache,
          self.exports_info_artifact,
          &collected_exports.specs,
        );
        refreshed_exports_specs.sort_by_key(|exports_spec| {
          refreshed_exports_spec_order(self.mg, exports_spec.exports_spec())
        });
        let meta = dependency_graph
          .module_meta
          .get(module_id)
          .copied()
          .unwrap_or_default();
        Some((*module_id, refreshed_exports_specs, meta))
      })
      .collect()
  }

  // Apply a batch of exports specs. Non-nested specs are pure updates to the
  // module's own `ExportsInfoData`, so each module can be computed independently
  // and committed afterward. Nested specs may touch child `ExportsInfo` records
  // through the artifact and therefore stay on the sequential path.
  fn process_module_exports_specs_batch<'b>(
    &mut self,
    module_exports_specs: impl IntoIterator<Item = (ModuleIdentifier, &'b ModuleExportsSpecs, bool)>,
  ) {
    let (non_nested_modules, nested_modules): (Vec<_>, Vec<_>) = module_exports_specs
      .into_iter()
      .partition(|(_, _, has_nested_exports)| !has_nested_exports);

    let updated_exports_info = non_nested_modules
      .into_par_iter()
      .map(|(module_id, exports_specs, _)| {
        let mut exports_info = self
          .exports_info_artifact
          .get_exports_info_data(&module_id)
          .clone();
        for spec in exports_specs.iter().filter(|spec| spec.apply_initial) {
          process_exports_spec_without_nested(
            self.mg,
            self.exports_info_artifact,
            &module_id,
            spec.dep_id,
            &spec.exports_spec,
            &mut exports_info,
          );
        }
        exports_info
      })
      .collect::<Vec<_>>();

    for exports_info in updated_exports_info {
      self
        .exports_info_artifact
        .set_exports_info_by_id(exports_info.id(), exports_info);
    }

    for (module_id, exports_specs, _) in nested_modules {
      self.process_initial_exports_specs_for_module(&module_id, exports_specs);
    }
  }

  fn process_refreshed_module_exports_specs_batch<'b>(
    &mut self,
    module_exports_specs: impl IntoIterator<
      Item = (ModuleIdentifier, &'b RefreshedModuleExportsSpecs<'b>, bool),
    >,
  ) {
    let (non_nested_modules, nested_modules): (Vec<_>, Vec<_>) = module_exports_specs
      .into_iter()
      .partition(|(_, _, has_nested_exports)| !has_nested_exports);

    let updated_exports_info = non_nested_modules
      .into_par_iter()
      .map(|(module_id, exports_specs, _)| {
        let mut exports_info = self
          .exports_info_artifact
          .get_exports_info_data(&module_id)
          .clone();
        for exports_spec in exports_specs {
          process_exports_spec_without_nested(
            self.mg,
            self.exports_info_artifact,
            &module_id,
            exports_spec.dep_id(),
            exports_spec.exports_spec(),
            &mut exports_info,
          );
        }
        exports_info
      })
      .collect::<Vec<_>>();

    for exports_info in updated_exports_info {
      self
        .exports_info_artifact
        .set_exports_info_by_id(exports_info.id(), exports_info);
    }

    for (module_id, exports_specs, _) in nested_modules {
      self.process_refreshed_exports_specs(&module_id, exports_specs);
    }
  }

  fn process_initial_exports_specs_for_module(
    &mut self,
    module_id: &ModuleIdentifier,
    exports_specs: &ModuleExportsSpecs,
  ) -> bool {
    if exports_specs
      .iter()
      .filter(|spec| spec.apply_initial)
      .all(|spec| !spec.exports_spec.has_nested_exports())
    {
      let mut changed = false;
      let mut exports_info = self
        .exports_info_artifact
        .get_exports_info_data(module_id)
        .clone();
      for spec in exports_specs.iter().filter(|spec| spec.apply_initial) {
        let is_changed = process_exports_spec_without_nested(
          self.mg,
          self.exports_info_artifact,
          module_id,
          spec.dep_id,
          &spec.exports_spec,
          &mut exports_info,
        );
        changed |= is_changed;
      }
      self
        .exports_info_artifact
        .set_exports_info_by_id(exports_info.id(), exports_info);
      return changed;
    }

    let mut changed = false;
    for spec in exports_specs.iter().filter(|spec| spec.apply_initial) {
      let is_changed = process_exports_spec(
        self.mg,
        self.exports_info_artifact,
        module_id,
        spec.dep_id,
        &spec.exports_spec,
      );
      changed |= is_changed;
    }
    changed
  }

  fn process_refreshed_exports_specs(
    &mut self,
    module_id: &ModuleIdentifier,
    exports_specs: &[RefreshedExportsSpec<'_>],
  ) -> bool {
    if exports_specs
      .iter()
      .all(|exports_spec| !exports_spec.exports_spec().has_nested_exports())
    {
      let mut changed = false;
      let mut exports_info = self
        .exports_info_artifact
        .get_exports_info_data(module_id)
        .clone();
      for exports_spec in exports_specs {
        let is_changed = process_exports_spec_without_nested(
          self.mg,
          self.exports_info_artifact,
          module_id,
          exports_spec.dep_id(),
          exports_spec.exports_spec(),
          &mut exports_info,
        );
        changed |= is_changed;
      }
      self
        .exports_info_artifact
        .set_exports_info_by_id(exports_info.id(), exports_info);
      return changed;
    }

    let mut changed = false;
    for exports_spec in exports_specs {
      let is_changed = process_exports_spec(
        self.mg,
        self.exports_info_artifact,
        module_id,
        exports_spec.dep_id(),
        exports_spec.exports_spec(),
      );
      changed |= is_changed;
    }
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
  ) -> IdentifierMap<CollectedModuleExports> {
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
  module_meta: IdentifierMap<ModuleExportsMeta>,
  leaf_modules: VecDeque<ModuleIdentifier>,
}

#[derive(Clone, Copy, Default)]
struct ModuleExportsMeta {
  has_nested_exports: bool,
  needs_backtracking: bool,
}

fn build_reexport_dependency_graph(
  module_exports_specs: &IdentifierMap<CollectedModuleExports>,
) -> (ReexportDependencyGraph, Vec<(ModuleIdentifier, bool)>) {
  let mut reverse_dependencies = IdentifierMap::<IdentifierSet>::default();
  let mut module_meta = IdentifierMap::<ModuleExportsMeta>::default();
  let mut initial_modules = Vec::new();

  for (module_id, collected_exports) in module_exports_specs {
    let meta = collected_exports.meta;
    if collected_exports
      .specs
      .iter()
      .any(|spec| spec.apply_initial)
    {
      initial_modules.push((*module_id, meta.has_nested_exports));
    }
    if meta.has_nested_exports || meta.needs_backtracking {
      module_meta.insert(*module_id, meta);
    }
  }

  let graph_modules = module_meta
    .iter()
    .filter_map(|(module_id, meta)| meta.needs_backtracking.then_some(*module_id))
    .collect::<IdentifierSet>();

  let mut remaining_dependency_count =
    IdentifierMap::<usize>::with_capacity_and_hasher(graph_modules.len(), Default::default());
  let mut leaf_modules = VecDeque::new();
  for module_id in &graph_modules {
    let dependency_count = module_exports_specs
      .get(module_id)
      .and_then(|collected_exports| collected_exports.dependencies.as_ref())
      .map_or(0, |dependencies| match dependencies.as_slice() {
        [dependency] if graph_modules.contains(dependency) && dependency != module_id => {
          reverse_dependencies
            .entry(*dependency)
            .or_default()
            .insert(*module_id);
          1
        }
        [_] => 0,
        dependencies => {
          let mut seen = IdentifierSet::default();
          let mut count = 0;
          for dependency in dependencies
            .iter()
            .copied()
            .filter(|dependency| graph_modules.contains(dependency) && dependency != module_id)
          {
            if seen.insert(dependency) {
              count += 1;
              reverse_dependencies
                .entry(dependency)
                .or_default()
                .insert(*module_id);
            }
          }
          count
        }
      });
    remaining_dependency_count.insert(*module_id, dependency_count);
    if dependency_count == 0 {
      leaf_modules.push_back(*module_id);
    }
  }

  (
    ReexportDependencyGraph {
      reverse_dependencies,
      dependency_count: remaining_dependency_count,
      module_meta,
      leaf_modules,
    },
    initial_modules,
  )
}

fn refresh_exports_specs_from_initial<'a>(
  mg: &ModuleGraph,
  mg_cache: &ModuleGraphCacheArtifact,
  exports_info_artifact: &ExportsInfoArtifact,
  exports_specs: &'a ModuleExportsSpecs,
) -> RefreshedModuleExportsSpecs<'a> {
  let mut refreshed_exports_specs = Vec::with_capacity(exports_specs.len());
  for spec in exports_specs
    .iter()
    .filter(|spec| spec.refresh_on_backtrack)
  {
    let dep_id = spec.dep_id;
    let exports_spec = &spec.exports_spec;
    if exports_spec_needs_recollect(exports_spec) {
      let Some(exports_spec) =
        mg.dependency_by_id(&dep_id)
          .get_exports(mg, mg_cache, exports_info_artifact)
      else {
        continue;
      };
      push_refreshed_exports_spec(
        mg,
        exports_info_artifact,
        &mut refreshed_exports_specs,
        dep_id,
        RefreshedExportsSpec::Owned(dep_id, exports_spec),
      );
      continue;
    }
    push_refreshed_exports_spec(
      mg,
      exports_info_artifact,
      &mut refreshed_exports_specs,
      dep_id,
      RefreshedExportsSpec::Borrowed(dep_id, exports_spec),
    );
  }

  refreshed_exports_specs
}

fn push_refreshed_exports_spec<'a>(
  mg: &ModuleGraph,
  exports_info_artifact: &ExportsInfoArtifact,
  refreshed_exports_specs: &mut RefreshedModuleExportsSpecs<'a>,
  dep_id: DependencyId,
  exports_spec: RefreshedExportsSpec<'a>,
) {
  let exports_spec_ref = exports_spec.exports_spec();
  let explicit_exports =
    known_exports_for_internal_unknown_reexport(mg, exports_info_artifact, exports_spec_ref);
  let Some((from, from_module, explicit_exports)) = explicit_exports else {
    refreshed_exports_specs.push(exports_spec);
    return;
  };

  refreshed_exports_specs.push(RefreshedExportsSpec::Owned(
    dep_id,
    unknown_exports_spec_with_extra_excludes(exports_spec_ref, &explicit_exports),
  ));
  refreshed_exports_specs.push(RefreshedExportsSpec::Owned(
    dep_id,
    known_exports_spec_for_internal_unknown_reexport(
      from,
      from_module,
      exports_spec_ref.priority,
      explicit_exports,
    ),
  ));
}

fn exports_spec_needs_recollect(exports_spec: &ExportsSpec) -> bool {
  exports_spec.hide_export.is_some()
    || matches!(exports_spec.exports, ExportsOfExportsSpec::Names(_))
      && exports_spec
        .dependencies
        .as_ref()
        .is_some_and(|dependencies| !dependencies.is_empty())
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

fn refreshed_exports_spec_order(mg: &ModuleGraph, exports_spec: &ExportsSpec) -> u8 {
  let is_external_unknown = matches!(exports_spec.exports, ExportsOfExportsSpec::UnknownExports)
    && exports_spec
      .from
      .as_ref()
      .is_some_and(|from| is_external_module(mg, from.module_identifier()));
  if is_external_unknown { 1 } else { 0 }
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
  let mut res = Vec::with_capacity(all_dependencies.len());
  let mut meta = ModuleExportsMeta::default();
  let mut dependencies: Option<Vec<ModuleIdentifier>> = None;
  for id in all_dependencies.iter().copied() {
    let Some((exports_spec, reexport_info)) = mg
      .dependency_by_id(&id)
      .get_exports_with_reexport_info(mg, mg_cache, exports_info_artifact)
    else {
      continue;
    };
    let apply_initial = !reexport_info.can_skip_initial_exports();
    if reexport_info.has_nested_exports || reexport_info.needs_backtracking {
      meta.has_nested_exports |= reexport_info.has_nested_exports;
      meta.needs_backtracking |= reexport_info.needs_backtracking;
      if let Some(reexport_dependencies) = reexport_info.dependencies {
        dependencies
          .get_or_insert_with(Default::default)
          .extend(reexport_dependencies);
      }
    }
    res.push(CollectedExportsSpec {
      dep_id: id,
      exports_spec,
      apply_initial,
      refresh_on_backtrack: reexport_info.needs_backtracking,
    });
  }
  (!res.is_empty()).then_some(CollectedModuleExports {
    specs: res,
    meta,
    dependencies,
  })
}

#[derive(Debug, Clone)]
struct DefaultExportInfo<'a> {
  can_mangle: Option<bool>,
  terminal_binding: bool,
  from: Option<&'a ModuleGraphConnection>,
  priority: Option<u8>,
}

fn process_exports_spec(
  mg: &ModuleGraph,
  exports_info_artifact: &mut ExportsInfoArtifact,
  module_id: &ModuleIdentifier,
  dep_id: DependencyId,
  export_desc: &ExportsSpec,
) -> bool {
  let mut changed = false;

  let exports = &export_desc.exports;
  let global_can_mangle = &export_desc.can_mangle;
  let global_from = export_desc.from.as_ref();
  let global_priority = &export_desc.priority;
  let global_terminal_binding = export_desc.terminal_binding.unwrap_or(false);
  if let Some(hide_export) = &export_desc.hide_export {
    let exports_info = exports_info_artifact.get_exports_info_data_mut(module_id);
    for name in hide_export.iter() {
      exports_info.ensure_export_info(name);
    }
    for name in hide_export.iter() {
      exports_info
        .named_exports_mut(name)
        .expect("should have named export")
        .unset_target(&dep_id);
    }
  }
  match exports {
    ExportsOfExportsSpec::UnknownExports => {
      changed |= exports_info_artifact
        .get_exports_info_data_mut(module_id)
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
      changed |= merge_exports(
        mg,
        exports_info_artifact,
        module_id,
        exports_info_artifact.get_exports_info(module_id),
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

fn process_exports_spec_without_nested(
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
      changed |= merge_exports_without_nested(
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

fn merge_exports(
  mg: &ModuleGraph,
  exports_info_artifact: &mut ExportsInfoArtifact,
  module_id: &ModuleIdentifier,
  exports_info: ExportsInfo,
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

    let export_info = exports_info
      .as_data_mut(exports_info_artifact)
      .ensure_export_info(name);
    changed |= set_export_base_info(
      export_info.as_data_mut(exports_info_artifact),
      can_mangle,
      terminal_binding,
      inlinable,
    );

    if let Some(exports) = exports {
      changed |= merge_nested_exports(
        mg,
        exports_info_artifact,
        module_id,
        export_info.clone(),
        exports,
        global_export_info.clone(),
        dep_id,
      );
    }

    changed |= set_export_target(
      export_info.as_data_mut(exports_info_artifact),
      from,
      from_export,
      priority,
      hidden,
      dep_id,
      name,
    );

    let (target_exports_info, _) = find_target_exports_info(
      mg,
      exports_info_artifact,
      export_info.as_data(exports_info_artifact),
    );

    let export_info_data = export_info.as_data_mut(exports_info_artifact);
    if export_info_data.exports_info_owned()
      && export_info_data.exports_info() != target_exports_info
      && let Some(target_exports_info) = target_exports_info
    {
      export_info_data.set_exports_info(Some(target_exports_info));
      changed = true;
    }
  }
  changed
}

fn merge_exports_without_nested(
  mg: &ModuleGraph,
  exports_info_artifact: &ExportsInfoArtifact,
  _module_id: &ModuleIdentifier,
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
      from,
      from_export,
      priority,
      hidden,
      inlinable,
      ..
    } = ParsedExportSpec::new(export_name_or_spec, &global_export_info);

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

    let (target_exports_info, _) = find_target_exports_info(mg, exports_info_artifact, export_info);

    if export_info.exports_info() != target_exports_info {
      export_info.set_exports_info(target_exports_info);
      changed = true;
    }
  }
  changed
}

fn merge_nested_exports(
  mg: &ModuleGraph,
  exports_info_artifact: &mut ExportsInfoArtifact,
  module_id: &ModuleIdentifier,
  export_info: ExportInfo,
  exports: &[ExportNameOrSpec],
  global_export_info: DefaultExportInfo,
  dep_id: DependencyId,
) -> bool {
  let nested_exports_info = if export_info
    .as_data(exports_info_artifact)
    .exports_info_owned()
  {
    export_info
      .as_data(exports_info_artifact)
      .exports_info()
      .expect("should have exports_info when exports_info is true")
  } else {
    let export_info = export_info.as_data_mut(exports_info_artifact);
    let new_exports_info = ExportsInfoData::default();
    let new_exports_info_id = new_exports_info.id();
    export_info.set_exports_info(Some(new_exports_info_id));
    export_info.set_exports_info_owned(true);
    exports_info_artifact.set_exports_info_by_id(new_exports_info_id, new_exports_info);

    new_exports_info_id
      .as_data_mut(exports_info_artifact)
      .set_has_provide_info();
    new_exports_info_id
  };

  merge_exports(
    mg,
    exports_info_artifact,
    module_id,
    nested_exports_info,
    exports,
    global_export_info,
    dep_id,
  )
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

fn find_target_exports_info(
  mg: &ModuleGraph,
  exports_info_artifact: &ExportsInfoArtifact,
  export_info: &ExportInfoData,
) -> (Option<ExportsInfo>, Option<ModuleIdentifier>) {
  let target = get_target(
    export_info,
    mg,
    exports_info_artifact,
    &|_| true,
    &mut Default::default(),
  );

  let mut target_exports_info = None;
  let mut target_module = None;
  if let Some(GetTargetResult::Target(target)) = target {
    let target_module_exports_info = exports_info_artifact.get_exports_info_data(&target.module);
    target_exports_info = target_module_exports_info
      .get_nested_exports_info(exports_info_artifact, target.export.as_deref())
      .map(|data| data.id());
    target_module = Some(target.module);
  }

  (target_exports_info, target_module)
}
