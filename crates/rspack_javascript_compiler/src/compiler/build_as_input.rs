/**
 * Some code is modified based on swc's build input, minifier, and comment
 * preservation implementations.
 * Apache-2.0 licensed.
 */
use std::sync::Arc;

#[cfg(feature = "plugin")]
use anyhow::Context;
use anyhow::{Error, bail};
use swc_config::{
  is_module::IsModule,
  merge::Merge,
  types::{BoolOr, BoolOrDataConfig},
};
#[cfg(feature = "plugin")]
use swc_core::ecma::visit::{Fold, fold_pass, noop_fold_type};
use swc_core::{
  base::config::{
    Config, DecoratorVersion, InputSourceMap, JsMinifyCommentOption, JsMinifyOptions, ModuleConfig,
    Options as SwcOptions, OutputCharset, PluginConfig,
  },
  common::{
    BytePos, DUMMY_SP, FileName, Mark, SourceMap, Span,
    comments::{Comment, Comments, SingleThreadedComments},
    errors::Handler,
    pass::{Either, Optional},
    util::take::Take,
  },
  ecma::{
    ast::{EsVersion, Module, Pass, Program, Script, noop_pass},
    atoms::Atom,
    parser::Syntax,
    preset_env::{Caniuse, Config as PresetEnvConfig, EnvConfig, Feature},
    transforms::{
      base::{
        assumptions::Assumptions,
        fixer::{fixer, paren_remover},
        helpers,
        hygiene::{self, hygiene_with_config},
        resolver,
      },
      module as modules,
      optimization::{const_modules, json_parse, simplifier},
      proposal::{
        decorator_2022_03, decorator_2023_11, decorators,
        explicit_resource_management::explicit_resource_management, export_default_from,
        import_attributes,
      },
      react::{self, default_pragma, default_pragma_frag},
      typescript::{self, TsImportExportAssignConfig},
    },
    visit::{VisitMut, VisitMutWith, noop_visit_mut_type, visit_mut_pass},
  },
};
use swc_ecma_ext_transforms::jest;
#[cfg(all(feature = "plugin", not(target_arch = "wasm32")))]
use swc_ecma_loader::{
  resolve::Resolve,
  resolvers::{lru::CachingResolver, node::NodeModulesResolver},
};
use swc_ecma_minifier::{
  optimize,
  option::{ExtraOptions, MinifyOptions, terser::TerserTopLevelOptions},
};

#[derive(Debug)]
struct BuildInputConfig {
  env: Option<PresetEnvConfig>,
  jsc: swc_core::base::config::JscConfig,
  module: Option<ModuleConfig>,
  minify: bool,
  input_source_map: InputSourceMap,
  is_module: IsModule,
}

pub struct BuiltInput<P: Pass> {
  pub program: Program,
  pub pass: P,
  pub target: EsVersion,
  pub minify: bool,
  pub input_source_map: InputSourceMap,
  pub comments: Option<SingleThreadedComments>,
  pub preserve_comments: BoolOr<JsMinifyCommentOption>,
  pub output_charset: Option<OutputCharset>,
  pub output_preamble: String,
  pub codegen_inline_script: bool,
  pub unresolved_mark: Mark,
}

#[allow(clippy::too_many_arguments)]
pub fn build_as_input<'a, P>(
  options: &SwcOptions,
  cm: &Arc<SourceMap>,
  base: &FileName,
  parse: impl FnOnce(Syntax, EsVersion, IsModule) -> Result<(Program, bool), Error>,
  handler: &Handler,
  config: Option<Config>,
  comments: Option<&'a SingleThreadedComments>,
  custom_before_pass: impl FnOnce(&Program) -> P,
) -> Result<BuiltInput<Box<dyn 'a + Pass>>, Error>
where
  P: 'a + Pass,
{
  let BuildInputConfig {
    env,
    jsc,
    module,
    minify,
    input_source_map,
    is_module,
  } = compute_build_input_config(options, base, &config);

  let swc_core::base::config::JscConfig {
    assumptions,
    transform,
    syntax,
    target,
    loose,
    keep_class_names,
    minify: mut js_minify,
    experimental,
    preserve_all_comments,
    output,
    base_url,
    paths,
    rewrite_relative_import_extensions,
    preserve_symlinks,
    ..
  } = jsc;
  let loose = loose.into_bool();
  let preserve_all_comments = preserve_all_comments.into_bool();
  let keep_class_names = keep_class_names.into_bool();

  let mut assumptions = assumptions.unwrap_or_else(|| {
    if loose {
      Assumptions::all()
    } else {
      Assumptions::default()
    }
  });

  let unresolved_mark = options.unresolved_mark.unwrap_or_default();
  let top_level_mark = options.top_level_mark.unwrap_or_default();

  if target.is_some() && env.is_some() {
    bail!("`env` and `jsc.target` cannot be used together");
  }

  let es_version = target.unwrap_or_default();
  let syntax = syntax.unwrap_or_default();
  let (mut program, flow_strip_script_like_module) = parse(syntax, es_version, is_module)?;
  let mut transform = transform.into_inner().unwrap_or_default();

  if syntax.typescript() {
    assumptions.set_class_methods |= !transform.use_define_for_class_fields.into_bool();
  }

  assumptions.set_public_class_fields |= !transform.use_define_for_class_fields.into_bool();

  program.visit_mut_with(&mut resolver(
    unresolved_mark,
    top_level_mark,
    syntax.typescript(),
  ));

  let default_top_level = program.is_module() && !flow_strip_script_like_module;

  js_minify = normalize_js_minify_options(js_minify, default_top_level, module.as_ref());

  let preserve_comments = if preserve_all_comments {
    BoolOr::Bool(true)
  } else {
    js_minify
      .as_ref()
      .map(|v| match v.format.comments.clone().into_inner() {
        Some(v) => v,
        None => BoolOr::Bool(true),
      })
      .unwrap_or_else(|| {
        BoolOr::Data(if minify {
          JsMinifyCommentOption::PreserveSomeComments
        } else {
          JsMinifyCommentOption::PreserveAllComments
        })
      })
  };

  if syntax.typescript() {
    transform.legacy_decorator = true.into();
  }
  let optimizer = transform.optimizer;

  let const_modules = {
    let enabled = transform.const_modules.is_some();
    let config = transform.const_modules.unwrap_or_default();

    Optional::new(const_modules(cm.clone(), config.globals), enabled)
  };

  let json_parse_pass = optimizer
    .as_ref()
    .and_then(|v| v.jsonify)
    .as_ref()
    .map(|cfg| json_parse(cfg.min_cost));

  let simplifier_pass = {
    if let Some(ref opts) = optimizer.as_ref().and_then(|o| o.simplify) {
      match opts {
        swc_core::base::config::SimplifyOption::Bool(allow_simplify) => {
          if *allow_simplify {
            Some(simplifier(unresolved_mark, Default::default()))
          } else {
            None
          }
        }
        swc_core::base::config::SimplifyOption::Json(cfg) => Some(simplifier(
          unresolved_mark,
          swc_core::ecma::transforms::optimization::simplify::Config {
            dce: swc_core::ecma::transforms::optimization::simplify::dce::Config {
              preserve_imports_with_side_effects: cfg.preserve_imports_with_side_effects,
              ..Default::default()
            },
            ..Default::default()
          },
        )),
      }
    } else {
      None
    }
  };

  let optimization = optimizer
    .and_then(|o| o.globals)
    .map(|opts| opts.build(cm, handler));

  let pass = (
    const_modules,
    optimization,
    Optional::new(export_default_from(), syntax.export_default_from()),
    simplifier_pass,
    json_parse_pass,
  );

  let import_export_assign_config = match &module {
    Some(ModuleConfig::Es6(..)) => TsImportExportAssignConfig::EsNext,
    Some(ModuleConfig::CommonJs(..))
    | Some(ModuleConfig::Amd(..))
    | Some(ModuleConfig::Umd(..)) => TsImportExportAssignConfig::Preserve,
    Some(ModuleConfig::NodeNext(..)) => TsImportExportAssignConfig::NodeNext,
    _ => TsImportExportAssignConfig::Classic,
  };

  let verbatim_module_syntax = transform.verbatim_module_syntax.into_bool();
  let ts_enum_is_mutable = transform.ts_enum_is_mutable.into_bool();

  let charset = output.charset.or_else(|| {
    if js_minify.as_ref()?.format.ascii_only {
      Some(OutputCharset::Ascii)
    } else {
      None
    }
  });

  let codegen_inline_script = js_minify.as_ref().is_some_and(|v| v.format.inline_script);

  let preamble = if !output.preamble.is_empty() {
    output.preamble.clone()
  } else {
    js_minify
      .as_ref()
      .map(|v| v.format.preamble.clone())
      .unwrap_or_default()
  };

  let target = es_version;
  let inject_helpers = !options.skip_helper_injection;
  let fixer_enabled = !options.disable_fixer;
  let hygiene_config = if options.disable_hygiene {
    None
  } else {
    Some(hygiene::Config {
      keep_class_names,
      ..Default::default()
    })
  };
  let env: Option<EnvConfig> = env.map(Into::into);

  let feature_config = env.as_ref().map(|e| e.get_feature_config());

  let (need_analyzer, import_interop, ignore_dynamic) = match &module {
    Some(ModuleConfig::CommonJs(c)) => (true, c.import_interop(), c.ignore_dynamic),
    Some(ModuleConfig::Amd(c)) => (true, c.config.import_interop(), c.config.ignore_dynamic),
    Some(ModuleConfig::Umd(c)) => (true, c.config.import_interop(), c.config.ignore_dynamic),
    Some(ModuleConfig::SystemJs(_))
    | Some(ModuleConfig::Es6(..))
    | Some(ModuleConfig::NodeNext(..))
    | None => (false, true.into(), true),
  };

  let compat_pass = {
    if let Some(env_config) = env {
      Either::Left(swc_core::ecma::preset_env::transform_from_env(
        unresolved_mark,
        comments.map(|v| v as &dyn Comments),
        env_config,
        assumptions,
      ))
    } else {
      Either::Right(swc_core::ecma::preset_env::transform_from_es_version(
        unresolved_mark,
        comments.map(|v| v as &dyn Comments),
        target,
        assumptions,
        loose,
      ))
    }
  };

  let is_mangler_enabled = js_minify
    .as_ref()
    .map(|v| v.mangle.is_obj() || v.mangle.is_true())
    .unwrap_or(false);

  let paths = paths.into_iter().collect();
  let resolver = ModuleConfig::get_resolver(
    &base_url,
    paths,
    base,
    module.as_ref(),
    preserve_symlinks.into_bool(),
  );

  let rewrite_import_pass: Box<dyn Pass> = {
    let swc_import_rewriter: Box<dyn Pass> = match resolver.clone() {
      Some((base, resolver)) => match &module {
        None | Some(ModuleConfig::Es6(..) | ModuleConfig::NodeNext(..)) => {
          Box::new(modules::rewriter::import_rewriter(base, resolver))
        }
        _ => Box::new(noop_pass()),
      },
      None => Box::new(noop_pass()),
    };

    let typescript_import_rewriter = Optional::new(
      modules::rewriter::typescript_import_rewriter(),
      rewrite_relative_import_extensions.into_bool(),
    );

    Box::new((swc_import_rewriter, typescript_import_rewriter))
  };

  let module_pass: Box<dyn Pass> = Box::new((
    Optional::new(
      modules::import_analysis::import_analyzer(import_interop, ignore_dynamic),
      need_analyzer,
    ),
    rewrite_import_pass,
    Optional::new(helpers::inject_helpers(unresolved_mark), inject_helpers),
    ModuleConfig::build(
      cm.clone(),
      comments.map(|v| v as &dyn Comments),
      module.clone(),
      unresolved_mark,
      resolver.clone(),
      |f| {
        feature_config
          .as_ref()
          .map_or_else(|| target.caniuse(f), |env| env.caniuse(f))
      },
    ),
  ));

  let built_pass = (
    pass,
    Optional::new(
      paren_remover(comments.map(|v| v as &dyn Comments)),
      fixer_enabled,
    ),
    compat_pass,
    module_pass,
    MinifierPass {
      options: js_minify.clone(),
      cm: cm.clone(),
      comments: comments.map(|v| v as &dyn Comments),
      top_level_mark,
    },
    Optional::new(
      hygiene_with_config(swc_core::ecma::transforms::base::hygiene::Config {
        top_level_mark,
        ..hygiene_config.clone().unwrap_or_default()
      }),
      hygiene_config.is_some() && !is_mangler_enabled,
    ),
    Optional::new(fixer(comments.map(|v| v as &dyn Comments)), fixer_enabled),
  );

  let keep_import_attributes = experimental.keep_import_attributes.into_bool();
  let run_plugin_first = experimental.run_plugin_first.into_bool();
  let disable_builtin_transforms_for_internal_testing = experimental
    .disable_builtin_transforms_for_internal_testing
    .into_bool();

  let mut plugin_transforms = Some(build_plugin_transforms(
    options,
    base,
    handler,
    experimental.plugins,
    experimental.plugin_env_vars,
    experimental.cache_root,
    comments,
    cm.clone(),
    unresolved_mark,
  )?);

  let pass: Box<dyn Pass> = if disable_builtin_transforms_for_internal_testing {
    plugin_transforms.take().expect("plugin pass should exist")
  } else {
    let jsx_enabled = syntax.jsx() && transform.react.runtime != Some(react::Runtime::Preserve);

    let decorator_pass: Box<dyn Pass> = match transform.decorator_version.unwrap_or_default() {
      DecoratorVersion::V202112 => Box::new(decorators(decorators::Config {
        legacy: transform.legacy_decorator.into_bool(),
        emit_metadata: transform.decorator_metadata.into_bool(),
        use_define_for_class_fields: !assumptions.set_public_class_fields,
      })),
      DecoratorVersion::V202203 => Box::new(decorator_2022_03::decorator_2022_03()),
      DecoratorVersion::V202311 => Box::new(decorator_2023_11::decorator_2023_11()),
    };

    Box::new((
      (
        if run_plugin_first {
          plugin_transforms.take()
        } else {
          None
        },
        Optional::new(decorator_pass, syntax.decorators()),
        Optional::new(
          explicit_resource_management(),
          syntax.explicit_resource_management(),
        ),
        Optional::new(import_attributes(), !keep_import_attributes),
      ),
      {
        let native_class_properties = !assumptions.set_public_class_fields
          && feature_config.as_ref().map_or_else(
            || target.caniuse(Feature::ClassProperties),
            |env| env.caniuse(Feature::ClassProperties),
          );

        let ts_config = typescript::Config {
          import_export_assign_config,
          verbatim_module_syntax,
          native_class_properties,
          ts_enum_is_mutable,
          flow_syntax: syntax.flow(),
          ..Default::default()
        };

        (
          Optional::new(
            typescript::typescript(ts_config, unresolved_mark, top_level_mark),
            syntax.typescript() && !jsx_enabled,
          ),
          Optional::new(
            typescript::tsx::<Option<&dyn Comments>>(
              cm.clone(),
              ts_config,
              typescript::TsxConfig {
                pragma: Some(
                  transform
                    .react
                    .pragma
                    .clone()
                    .unwrap_or_else(default_pragma),
                ),
                pragma_frag: Some(
                  transform
                    .react
                    .pragma_frag
                    .clone()
                    .unwrap_or_else(default_pragma_frag),
                ),
              },
              comments.map(|v| v as _),
              unresolved_mark,
              top_level_mark,
            ),
            syntax.typescript() && jsx_enabled,
          ),
        )
      },
      (
        plugin_transforms.take(),
        custom_before_pass(&program),
        Optional::new(
          react::react::<&dyn Comments>(
            cm.clone(),
            comments.map(|v| v as _),
            transform.react,
            top_level_mark,
            unresolved_mark,
          ),
          jsx_enabled,
        ),
        built_pass,
        Optional::new(jest::jest(), transform.hidden.jest.into_bool()),
        Optional::new(
          dropped_comments_preserver(comments.cloned()),
          preserve_all_comments,
        ),
      ),
    ))
  };

  Ok(BuiltInput {
    program,
    minify,
    pass,
    target: es_version,
    input_source_map,
    comments: comments.cloned(),
    preserve_comments,
    output_charset: charset,
    output_preamble: preamble,
    codegen_inline_script,
    unresolved_mark,
  })
}

#[allow(clippy::too_many_arguments)]
#[cfg(feature = "plugin")]
fn build_plugin_transforms<'a>(
  options: &SwcOptions,
  base: &FileName,
  _handler: &Handler,
  plugins: Option<Vec<PluginConfig>>,
  plugin_env_vars: Option<Vec<Atom>>,
  cache_root: Option<String>,
  comments: Option<&SingleThreadedComments>,
  cm: Arc<SourceMap>,
  unresolved_mark: Mark,
) -> Result<Box<dyn 'a + Pass>, Error> {
  let transform_filename = match base {
    FileName::Real(path) => path.as_os_str().to_str().map(String::from),
    FileName::Custom(filename) => Some(filename.to_owned()),
    _ => None,
  };
  let transform_metadata_context = Arc::new(
    swc_core::common::plugin::metadata::TransformPluginMetadataContext::new(
      transform_filename,
      options.env_name.to_owned(),
      None,
    ),
  );

  #[cfg(not(target_arch = "wasm32"))]
  {
    let plugin_runtime = Arc::new(rspack_util::swc::runtime::WasmtimeRuntime);

    if let Some(plugins) = &plugins {
      compile_wasm_plugins(cache_root.as_deref(), plugins, &*plugin_runtime)
        .context("Failed to compile wasm plugins")?;
    }

    Ok(Box::new(wasm_plugins(
      plugins,
      plugin_env_vars,
      transform_metadata_context,
      comments.cloned(),
      cm,
      unresolved_mark,
      plugin_runtime,
    )))
  }

  #[cfg(target_arch = "wasm32")]
  {
    let _ = (
      options,
      base,
      plugin_env_vars,
      cache_root,
      comments,
      cm,
      unresolved_mark,
      transform_metadata_context,
    );
    if plugins.is_some() {
      _handler.warn(
        "Currently @swc/wasm does not support plugins, plugin transform will be skipped. Refer https://github.com/swc-project/swc/issues/3934 for the details.",
      );
    }

    Ok(Box::new(noop_pass()))
  }
}

#[allow(clippy::too_many_arguments)]
#[cfg(not(feature = "plugin"))]
fn build_plugin_transforms<'a>(
  _options: &SwcOptions,
  _base: &FileName,
  handler: &Handler,
  plugins: Option<Vec<PluginConfig>>,
  _plugin_env_vars: Option<Vec<Atom>>,
  _cache_root: Option<String>,
  _comments: Option<&SingleThreadedComments>,
  _cm: Arc<SourceMap>,
  _unresolved_mark: Mark,
) -> Result<Box<dyn 'a + Pass>, Error> {
  if plugins.is_some() {
    handler
      .warn("Plugin is not supported with current @swc/core. Plugin transform will be skipped.");
  }

  Ok(Box::new(noop_pass()))
}

#[cfg(feature = "plugin")]
fn wasm_plugins(
  configured_plugins: Option<Vec<PluginConfig>>,
  plugin_env_vars: Option<Vec<Atom>>,
  metadata_context: Arc<swc_core::common::plugin::metadata::TransformPluginMetadataContext>,
  comments: Option<SingleThreadedComments>,
  source_map: Arc<SourceMap>,
  unresolved_mark: Mark,
  plugin_runtime: Arc<dyn swc_core::plugin_runner::runtime::Runtime>,
) -> impl Pass {
  fold_pass(WasmPlugins {
    plugins: configured_plugins,
    plugin_env_vars: plugin_env_vars.map(Arc::new),
    metadata_context,
    comments,
    source_map,
    unresolved_mark,
    plugin_runtime,
  })
}

#[cfg(feature = "plugin")]
struct WasmPlugins {
  plugins: Option<Vec<PluginConfig>>,
  plugin_env_vars: Option<Arc<Vec<Atom>>>,
  metadata_context: Arc<swc_core::common::plugin::metadata::TransformPluginMetadataContext>,
  comments: Option<SingleThreadedComments>,
  source_map: Arc<SourceMap>,
  unresolved_mark: Mark,
  plugin_runtime: Arc<dyn swc_core::plugin_runner::runtime::Runtime>,
}

#[cfg(feature = "plugin")]
impl WasmPlugins {
  fn apply(&mut self, program: Program) -> Result<Program, Error> {
    if self
      .plugins
      .as_ref()
      .is_none_or(|plugins| plugins.is_empty())
    {
      return Ok(program);
    }

    let filename = self.metadata_context.filename.clone();
    self
      .apply_inner(program)
      .with_context(|| format!("failed to invoke plugin on '{filename:?}'"))
  }

  #[cfg(not(target_arch = "wasm32"))]
  fn apply_inner(&mut self, program: Program) -> Result<Program, Error> {
    let should_enable_comments_proxy = self.comments.is_some();

    swc_core::plugin::proxies::COMMENTS.set(
      &swc_core::plugin::proxies::HostCommentsStorage {
        inner: self.comments.clone(),
      },
      || {
        let program = swc_core::common::plugin::serialized::VersionedSerializable::new(program);
        let mut serialized =
          swc_core::common::plugin::serialized::PluginSerializedBytes::try_serialize(&program)?;

        if let Some(plugins) = &mut self.plugins {
          for plugin in plugins.drain(..) {
            let plugin_module_bytes = swc_core::base::config::PLUGIN_MODULE_CACHE
              .inner
              .get()
              .expect("plugin module cache should be initialized")
              .lock()
              .get(&*self.plugin_runtime, &plugin.0)
              .expect("plugin module should be loaded");

            let plugin_name = plugin_module_bytes.get_module_name().to_string();

            let mut transform_plugin_executor =
              swc_core::plugin_runner::create_plugin_transform_executor(
                &self.source_map,
                &self.unresolved_mark,
                &self.metadata_context,
                self.plugin_env_vars.clone(),
                plugin_module_bytes,
                Some(plugin.1),
                self.plugin_runtime.clone(),
              );

            serialized = transform_plugin_executor
              .transform(&serialized, Some(should_enable_comments_proxy))
              .with_context(|| {
                format!(
                  "failed to invoke `{}` as js transform plugin at {}",
                  &plugin.0, plugin_name
                )
              })?;
          }
        }

        serialized.deserialize().map(|program| program.into_inner())
      },
    )
  }

  #[cfg(target_arch = "wasm32")]
  fn apply_inner(&mut self, program: Program) -> Result<Program, Error> {
    Ok(program)
  }
}

#[cfg(feature = "plugin")]
impl Fold for WasmPlugins {
  noop_fold_type!();

  fn fold_module(&mut self, module: Module) -> Module {
    match self.apply(Program::Module(module)) {
      Ok(program) => program.expect_module(),
      Err(error) => {
        swc_core::common::errors::HANDLER.with(|handler| {
          handler.err_with_code(
            &error.to_string(),
            swc_core::common::errors::DiagnosticId::Error("plugin".into()),
          );
        });
        Module::default()
      }
    }
  }

  fn fold_script(&mut self, script: Script) -> Script {
    match self.apply(Program::Script(script)) {
      Ok(program) => program.expect_script(),
      Err(error) => {
        swc_core::common::errors::HANDLER.with(|handler| {
          handler.err_with_code(
            &error.to_string(),
            swc_core::common::errors::DiagnosticId::Error("plugin".into()),
          );
        });
        Script::default()
      }
    }
  }
}

#[cfg(all(feature = "plugin", not(target_arch = "wasm32")))]
fn compile_wasm_plugins(
  cache_root: Option<&str>,
  plugins: &[PluginConfig],
  plugin_runtime: &dyn swc_core::plugin_runner::runtime::Runtime,
) -> Result<(), Error> {
  let plugin_resolver = CachingResolver::new(
    40,
    NodeModulesResolver::new(swc_ecma_loader::TargetEnv::Node, Default::default(), true),
  );

  swc_core::base::config::init_plugin_module_cache_once(true, cache_root);

  let mut inner_cache = swc_core::base::config::PLUGIN_MODULE_CACHE
    .inner
    .get()
    .expect("plugin module cache should be initialized")
    .lock();

  for plugin_config in plugins {
    let plugin_name = &plugin_config.0;

    if !inner_cache.contains(plugin_runtime, plugin_name) {
      let resolved_path = plugin_resolver
        .resolve(
          &FileName::Real(std::path::PathBuf::from(plugin_name)),
          plugin_name,
        )
        .with_context(|| format!("failed to resolve plugin path: {plugin_name}"))?;

      let path = if let FileName::Real(value) = resolved_path.filename {
        value
      } else {
        bail!("Failed to resolve plugin path: {resolved_path:?}");
      };

      inner_cache.store_bytes_from_path(plugin_runtime, &path, plugin_name)?;
    }
  }

  Ok(())
}

fn compute_build_input_config(
  options: &SwcOptions,
  base: &FileName,
  config: &Option<Config>,
) -> BuildInputConfig {
  let input_source_map = merged_input_source_map(options, config);

  let mut cfg = options.config.clone();
  cfg.input_source_map = None;

  let mut fallback_config = config.clone().unwrap_or_default();
  fallback_config.input_source_map = None;
  cfg.merge(fallback_config);

  if let FileName::Real(base) = base {
    cfg.adjust(base);
  }

  let Config {
    env,
    jsc,
    module,
    minify,
    is_module,
    ..
  } = cfg;

  BuildInputConfig {
    env,
    jsc,
    module,
    minify: minify.into_bool(),
    input_source_map,
    is_module: is_module.unwrap_or_default(),
  }
}

fn merged_input_source_map(options: &SwcOptions, config: &Option<Config>) -> InputSourceMap {
  let mut input_source_map = options.config.input_source_map.clone();
  if let Some(config) = config {
    input_source_map.merge(config.input_source_map.clone());
  }
  input_source_map.unwrap_or_default()
}

fn normalize_js_minify_options(
  mut js_minify: Option<JsMinifyOptions>,
  default_top_level: bool,
  module: Option<&ModuleConfig>,
) -> Option<JsMinifyOptions> {
  js_minify = js_minify.map(|mut c| {
    let compress = c
      .compress
      .unwrap_as_option(|default| match default {
        Some(true) => Some(Default::default()),
        _ => None,
      })
      .map(|mut c| {
        if c.toplevel.is_none() {
          c.toplevel = Some(TerserTopLevelOptions::Bool(default_top_level));
        }

        if matches!(
          module,
          None | Some(ModuleConfig::Es6(..) | ModuleConfig::NodeNext(..))
        ) {
          c.module = true;
        }

        c
      })
      .map(BoolOrDataConfig::from_obj)
      .unwrap_or_else(|| BoolOrDataConfig::from_bool(false));

    let mangle = c
      .mangle
      .unwrap_as_option(|default| match default {
        Some(true) => Some(Default::default()),
        _ => None,
      })
      .map(|mut c| {
        if c.top_level.is_none() {
          c.top_level = Some(default_top_level);
        }

        c
      })
      .map(BoolOrDataConfig::from_obj)
      .unwrap_or_else(|| BoolOrDataConfig::from_bool(false));

    if c.toplevel.is_none() {
      c.toplevel = Some(default_top_level);
    }

    JsMinifyOptions {
      compress,
      mangle,
      ..c
    }
  });

  if js_minify.is_some() && js_minify.as_ref().expect("checked above").keep_fnames {
    js_minify = js_minify.map(|c| {
      let compress = c
        .compress
        .unwrap_as_option(|default| match default {
          Some(true) => Some(Default::default()),
          _ => None,
        })
        .map(|mut c| {
          c.keep_fnames = true;
          c
        })
        .map(BoolOrDataConfig::from_obj)
        .unwrap_or_else(|| BoolOrDataConfig::from_bool(false));
      let mangle = c
        .mangle
        .unwrap_as_option(|default| match default {
          Some(true) => Some(Default::default()),
          _ => None,
        })
        .map(|mut c| {
          c.keep_fn_names = true;
          c
        })
        .map(BoolOrDataConfig::from_obj)
        .unwrap_or_else(|| BoolOrDataConfig::from_bool(false));
      JsMinifyOptions {
        compress,
        mangle,
        ..c
      }
    });
  }

  if js_minify.is_some() && js_minify.as_ref().expect("checked above").keep_classnames {
    js_minify = js_minify.map(|c| {
      let compress = c
        .compress
        .unwrap_as_option(|default| match default {
          Some(true) => Some(Default::default()),
          _ => None,
        })
        .map(|mut c| {
          c.keep_classnames = true;
          c
        })
        .map(BoolOrDataConfig::from_obj)
        .unwrap_or_else(|| BoolOrDataConfig::from_bool(false));
      let mangle = c
        .mangle
        .unwrap_as_option(|default| match default {
          Some(true) => Some(Default::default()),
          _ => None,
        })
        .map(|mut c| {
          c.keep_class_names = true;
          c
        })
        .map(BoolOrDataConfig::from_obj)
        .unwrap_or_else(|| BoolOrDataConfig::from_bool(false));
      JsMinifyOptions {
        compress,
        mangle,
        ..c
      }
    });
  }

  js_minify
}

struct MinifierPass<'a> {
  options: Option<JsMinifyOptions>,
  cm: Arc<SourceMap>,
  comments: Option<&'a dyn Comments>,
  top_level_mark: Mark,
}

impl Pass for MinifierPass<'_> {
  fn process(&mut self, n: &mut Program) {
    if let Some(options) = &self.options {
      let opts = MinifyOptions {
        compress: options
          .compress
          .clone()
          .unwrap_as_option(|default| match default {
            Some(true) => Some(Default::default()),
            _ => None,
          })
          .map(|mut v| {
            if v.const_to_let.is_none() {
              v.const_to_let = Some(true);
            }
            if v.toplevel.is_none() && n.is_module() {
              v.toplevel = Some(TerserTopLevelOptions::Bool(true));
            }

            if n.is_script() {
              v.module = false;
            }

            v.into_config(self.cm.clone())
          }),
        mangle: options
          .mangle
          .clone()
          .unwrap_as_option(|default| match default {
            Some(true) => Some(Default::default()),
            _ => None,
          }),
        ..Default::default()
      };

      if opts.compress.is_none() && opts.mangle.is_none() {
        return;
      }

      n.visit_mut_with(&mut hygiene_with_config(
        swc_core::ecma::transforms::base::hygiene::Config {
          top_level_mark: self.top_level_mark,
          ..Default::default()
        },
      ));

      let unresolved_mark = Mark::new();
      let top_level_mark = Mark::new();

      n.visit_mut_with(&mut resolver(unresolved_mark, top_level_mark, false));

      *n = optimize(
        n.take(),
        self.cm.clone(),
        self.comments.as_ref().map(|v| v as &dyn Comments),
        None,
        &opts,
        &ExtraOptions {
          unresolved_mark,
          top_level_mark,
          mangle_name_cache: None,
        },
      )
    }
  }
}

fn dropped_comments_preserver(comments: Option<SingleThreadedComments>) -> impl Pass {
  visit_mut_pass(DroppedCommentsPreserver {
    comments,
    is_first_span: true,
    known_spans: Vec::new(),
  })
}

struct DroppedCommentsPreserver {
  comments: Option<SingleThreadedComments>,
  is_first_span: bool,
  known_spans: Vec<Span>,
}

type CommentEntries = Vec<(BytePos, Vec<Comment>)>;

impl VisitMut for DroppedCommentsPreserver {
  noop_visit_mut_type!(fail);

  fn visit_mut_module(&mut self, module: &mut Module) {
    module.visit_mut_children_with(self);
    self.known_spans.sort_by_key(|span_a| span_a.lo);
    self.shift_comments_to_known_spans();
  }

  fn visit_mut_script(&mut self, script: &mut Script) {
    script.visit_mut_children_with(self);
    self.known_spans.sort_by_key(|span_a| span_a.lo);
    self.shift_comments_to_known_spans();
  }

  fn visit_mut_span(&mut self, span: &mut Span) {
    if span.is_dummy() || self.is_first_span {
      self.is_first_span = false;
      return;
    }

    self.known_spans.push(*span);
    span.visit_mut_children_with(self)
  }
}

impl DroppedCommentsPreserver {
  fn shift_comments_to_known_spans(&self) {
    if let Some(comments) = &self.comments {
      let trailing_comments = self.shift_leading_comments(comments);

      self.shift_trailing_comments(trailing_comments);
    }
  }

  fn collect_existing_comments(&self, comments: &SingleThreadedComments) -> CommentEntries {
    let (mut leading_comments, mut trailing_comments) = comments.borrow_all_mut();
    let mut existing_comments: CommentEntries = leading_comments
      .drain()
      .chain(trailing_comments.drain())
      .collect();

    existing_comments.sort_by_key(|(bp_a, _)| *bp_a);

    existing_comments
  }

  fn shift_leading_comments(&self, comments: &SingleThreadedComments) -> CommentEntries {
    let mut existing_comments = self.collect_existing_comments(comments);

    existing_comments.sort_by_key(|(bp_a, _)| *bp_a);

    for span in self.known_spans.iter() {
      let cut_point = existing_comments.partition_point(|(bp, _)| *bp <= span.lo);
      let collected_comments = existing_comments
        .drain(..cut_point)
        .flat_map(|(_, c)| c)
        .collect::<Vec<Comment>>();
      comments.add_leading_comments(span.lo, collected_comments)
    }

    existing_comments
  }

  fn shift_trailing_comments(&self, remaining_comment_entries: CommentEntries) {
    let last_trailing = self
      .known_spans
      .iter()
      .max_by_key(|span| span.hi)
      .cloned()
      .unwrap_or(DUMMY_SP);

    self.comments.add_trailing_comments(
      last_trailing.hi,
      remaining_comment_entries
        .into_iter()
        .flat_map(|(_, c)| c)
        .collect(),
    );
  }
}
