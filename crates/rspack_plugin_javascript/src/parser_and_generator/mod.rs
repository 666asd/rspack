use std::{
  borrow::Cow,
  sync::{Arc, LazyLock},
};

use regex::Regex;
use rspack_cacheable::{cacheable, cacheable_dyn, with::Skip};
use rspack_core::{
  AsyncDependenciesBlockIdentifier, BuildMetaExportsType, COLLECTED_TYPESCRIPT_INFO_PARSE_META_KEY,
  ChunkGraph, CollectedTypeScriptInfo, Compilation, ConcatenatedModuleIdentReference,
  ConcatenatedModuleScopeInfo, DependenciesBlock, DependencyId, GenerateContext, IdentCollector,
  Module, ModuleArgument, ModuleCodeTemplate, ModuleGraph, ModuleType, ParseContext, ParseResult,
  ParserAndGenerator, RuntimeGlobals, RuntimeVariable, SideEffectsBailoutItem, SourceType,
  TemplateContext, TemplateReplaceSource,
  diagnostics::map_box_diagnostics_to_module_parse_diagnostics,
  remove_bom, render_init_fragments,
  rspack_sources::{BoxSource, ReplaceSource, Source, SourceExt},
};
use rspack_error::{Diagnostic, IntoTWithDiagnosticArray, Result, TWithDiagnosticArray};
use rspack_javascript_compiler::JavaScriptCompiler;
use rustc_hash::FxHashSet as HashSet;
use swc_core::{
  base::config::IsModule,
  common::{BytePos, SyntaxContext, comments::SingleThreadedComments, input::SourceFileInput},
  ecma::{
    ast::{
      self, Decl, ExportDecl, ExportSpecifier, ImportSpecifier, ModuleDecl, ModuleExportName,
      ModuleItem, Pat, Program, Stmt,
    },
    parser::{EsSyntax, Syntax, lexer::Lexer},
    transforms::base::fixer::paren_remover,
    visit::{Visit, VisitWith, noop_visit_type},
  },
};

use crate::{
  BoxJavascriptParserPlugin,
  dependency::ESMCompatibilityDependency,
  visitors::{ScanDependenciesResult, scan_dependencies, semicolon, swc_visitor::resolver},
};

fn module_type_to_is_module(value: &ModuleType) -> IsModule {
  // parser options align with webpack
  match value {
    ModuleType::JsEsm => IsModule::Bool(true),
    ModuleType::JsDynamic => IsModule::Bool(false),
    _ => IsModule::Unknown,
  }
}

#[derive(Debug)]
pub struct ParserRuntimeRequirementsData {
  pub module: String,
  pub rspack_module: String,
  pub exports: String,
  pub require: String,
  pub require_regex: &'static LazyLock<Regex>,
  pub module_cache: String,
  pub entry_module_id: String,
}

static LEGACY_REQUIRE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new("__webpack_require__\\s*(!?\\.)").expect("should init `REQUIRE_FUNCTION_REGEX`")
});

fn collect_concatenated_module_scope_info(
  program: &ast::Program,
  global_ctxt: SyntaxContext,
  _module_ctxt: SyntaxContext,
) -> ConcatenatedModuleScopeInfo {
  type ScopedName = (swc_core::atoms::Atom, SyntaxContext);

  #[derive(Default)]
  struct TopLevelBindingCollector {
    bindings: HashSet<ScopedName>,
    import_locals: HashSet<ScopedName>,
    ignored_specifier_ranges: HashSet<(u32, u32)>,
  }

  impl TopLevelBindingCollector {
    fn collect_program(&mut self, program: &Program) {
      match program {
        Program::Module(module) => {
          for item in &module.body {
            self.collect_module_item(item);
          }
        }
        Program::Script(script) => {
          for stmt in &script.body {
            self.collect_stmt(stmt);
          }
        }
      }
    }

    fn collect_module_item(&mut self, item: &ModuleItem) {
      match item {
        ModuleItem::ModuleDecl(decl) => self.collect_module_decl(decl),
        ModuleItem::Stmt(stmt) => self.collect_stmt(stmt),
      }
    }

    fn collect_module_decl(&mut self, decl: &ModuleDecl) {
      match decl {
        ModuleDecl::Import(import) => {
          for specifier in &import.specifiers {
            self.collect_import_specifier(specifier);
          }
        }
        ModuleDecl::ExportDecl(ExportDecl { decl, .. }) => self.collect_decl(decl),
        ModuleDecl::ExportNamed(export) => {
          for specifier in &export.specifiers {
            self.collect_export_specifier(specifier);
          }
        }
        _ => {}
      }
    }

    fn collect_stmt(&mut self, stmt: &Stmt) {
      match stmt {
        Stmt::Decl(decl) => self.collect_decl(decl),
        Stmt::Labeled(labeled) => self.collect_stmt(&labeled.body),
        _ => {}
      }
    }

    fn collect_decl(&mut self, decl: &Decl) {
      match decl {
        Decl::Class(class) => self.collect_ident(&class.ident),
        Decl::Fn(function) => self.collect_ident(&function.ident),
        Decl::Var(var) => {
          for declarator in &var.decls {
            self.collect_pat(&declarator.name);
          }
        }
        _ => {}
      }
    }

    fn collect_pat(&mut self, pat: &Pat) {
      match pat {
        Pat::Ident(ident) => self.collect_ident(&ident.id),
        Pat::Array(array) => {
          for elem in array.elems.iter().flatten() {
            self.collect_pat(elem);
          }
        }
        Pat::Object(object) => {
          for prop in &object.props {
            match prop {
              ast::ObjectPatProp::KeyValue(prop) => self.collect_pat(&prop.value),
              ast::ObjectPatProp::Assign(prop) => self.collect_ident(&prop.key),
              ast::ObjectPatProp::Rest(prop) => self.collect_pat(&prop.arg),
            }
          }
        }
        Pat::Rest(rest) => self.collect_pat(&rest.arg),
        Pat::Assign(assign) => self.collect_pat(&assign.left),
        Pat::Expr(_) | Pat::Invalid(_) => {}
      }
    }

    fn collect_import_specifier(&mut self, specifier: &ImportSpecifier) {
      let local = match specifier {
        ImportSpecifier::Named(named) => &named.local,
        ImportSpecifier::Default(default) => &default.local,
        ImportSpecifier::Namespace(namespace) => &namespace.local,
      };
      let name = (local.sym.clone(), local.ctxt);
      self.bindings.insert(name.clone());
      self.import_locals.insert(name);
      self
        .ignored_specifier_ranges
        .insert((local.span.lo.0, local.span.hi.0));
    }

    fn collect_export_specifier(&mut self, specifier: &ExportSpecifier) {
      match specifier {
        ExportSpecifier::Named(named) => {
          self.collect_module_export_name(&named.orig);
          if let Some(exported) = &named.exported {
            self.collect_module_export_name(exported);
          }
        }
        ExportSpecifier::Default(default) => {
          self
            .ignored_specifier_ranges
            .insert((default.exported.span.lo.0, default.exported.span.hi.0));
        }
        ExportSpecifier::Namespace(namespace) => {
          self.collect_module_export_name(&namespace.name);
        }
      }
    }

    fn collect_module_export_name(&mut self, name: &ModuleExportName) {
      if let ModuleExportName::Ident(ident) = name {
        self
          .ignored_specifier_ranges
          .insert((ident.span.lo.0, ident.span.hi.0));
      }
    }

    fn collect_ident(&mut self, ident: &ast::Ident) {
      self.bindings.insert((ident.sym.clone(), ident.ctxt));
    }
  }

  impl Visit for TopLevelBindingCollector {
    noop_visit_type!();

    fn visit_import_specifier(&mut self, specifier: &ImportSpecifier) {
      match specifier {
        ImportSpecifier::Named(named) => {
          self.collect_import_specifier(&ImportSpecifier::Named(named.clone()));
        }
        ImportSpecifier::Default(default) => {
          self.collect_import_specifier(&ImportSpecifier::Default(default.clone()));
        }
        ImportSpecifier::Namespace(namespace) => {
          self.collect_import_specifier(&ImportSpecifier::Namespace(namespace.clone()));
        }
      }
    }
  }

  let mut top_level_binding_collector = TopLevelBindingCollector::default();
  top_level_binding_collector.collect_program(program);
  let top_level_bindings = top_level_binding_collector.bindings;
  let import_locals = top_level_binding_collector.import_locals;
  let ignored_specifier_ranges = top_level_binding_collector.ignored_specifier_ranges;

  let mut collector = IdentCollector::default();
  program.visit_with(&mut collector);

  let mut info = ConcatenatedModuleScopeInfo::default();

  for ident in collector.ids {
    let scoped_name = (ident.id.sym.clone(), ident.id.ctxt);
    if ignored_specifier_ranges.contains(&(ident.id.span.lo.0, ident.id.span.hi.0)) {
      continue;
    }
    if import_locals.contains(&scoped_name) {
      info
        .import_references
        .entry(ident.id.sym.clone())
        .or_default()
        .push(ConcatenatedModuleIdentReference {
          range: ident.id.span.into(),
          shorthand: ident.shorthand,
        });
      continue;
    }
    let is_top_level = top_level_bindings.contains(&scoped_name);
    if ident.id.ctxt == global_ctxt || !is_top_level || ident.is_class_expr_with_ident {
      info.all_used_names.insert(ident.id.sym.clone());
    }

    if is_top_level && !ident.is_class_expr_with_ident {
      let name = ident.id.sym.clone();
      if !info.top_level_references.contains_key(&name) {
        info.top_level_order.push(name.clone());
      }
      info
        .top_level_references
        .entry(name)
        .or_default()
        .push(ConcatenatedModuleIdentReference {
          range: ident.id.span.into(),
          shorthand: ident.shorthand,
        });
    }
  }

  info
}

impl ParserRuntimeRequirementsData {
  pub fn new(runtime_template: &ModuleCodeTemplate) -> Self {
    let require_name =
      runtime_template.render_runtime_globals_without_adding(&RuntimeGlobals::REQUIRE);
    let module_name =
      runtime_template.render_runtime_globals_without_adding(&RuntimeGlobals::MODULE);
    let exports_name =
      runtime_template.render_runtime_globals_without_adding(&RuntimeGlobals::EXPORTS);
    let module_cache_name =
      runtime_template.render_runtime_globals_without_adding(&RuntimeGlobals::MODULE_CACHE);
    let entry_module_id_name =
      runtime_template.render_runtime_globals_without_adding(&RuntimeGlobals::ENTRY_MODULE_ID);
    let rspack_module_name = runtime_template.render_runtime_variable(&RuntimeVariable::Module);
    Self {
      require_regex: &LEGACY_REQUIRE_REGEX,
      module: module_name,
      rspack_module: rspack_module_name,
      exports: exports_name,
      require: require_name,
      module_cache: module_cache_name,
      entry_module_id: entry_module_id_name,
    }
  }

  pub fn module_argument(&self, module_argument: &ModuleArgument) -> String {
    match module_argument {
      ModuleArgument::Module => self.module.clone(),
      ModuleArgument::RspackModule => self.rspack_module.clone(),
    }
  }
}

#[cacheable]
#[derive(Default)]
pub struct JavaScriptParserAndGenerator {
  #[cacheable(with=Skip)]
  parser_plugins: Vec<BoxJavascriptParserPlugin>,
}

impl std::fmt::Debug for JavaScriptParserAndGenerator {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("JavaScriptParserAndGenerator")
      .field("parser_plugins", &"...")
      .finish()
  }
}

impl JavaScriptParserAndGenerator {
  pub fn add_parser_plugin(&mut self, parser_plugin: BoxJavascriptParserPlugin) {
    self.parser_plugins.push(parser_plugin);
  }

  fn source_block(
    &self,
    compilation: &Compilation,
    block_id: &AsyncDependenciesBlockIdentifier,
    source: &mut TemplateReplaceSource,
    context: &mut TemplateContext,
  ) {
    let module_graph = compilation.get_module_graph();
    let block = module_graph
      .block_by_id(block_id)
      .expect("should have block");
    //    let block = block_id.expect_get(compilation);
    block.get_dependencies().iter().for_each(|dependency_id| {
      self.source_dependency(compilation, dependency_id, source, context)
    });
    block
      .get_blocks()
      .iter()
      .for_each(|block_id| self.source_block(compilation, block_id, source, context));
  }

  fn source_dependency(
    &self,
    compilation: &Compilation,
    dependency_id: &DependencyId,
    source: &mut TemplateReplaceSource,
    context: &mut TemplateContext,
  ) {
    if let Some(dependency) = compilation
      .get_module_graph()
      .dependency_by_id(dependency_id)
      .as_dependency_code_generation()
    {
      if let Some(template) = dependency
        .dependency_template()
        .and_then(|template_type| compilation.get_dependency_template(template_type))
      {
        template.render(dependency, source, context)
      } else {
        panic!(
          "Can not find dependency template of {:?}",
          dependency.dependency_template()
        );
      }
    }
  }
}

static SOURCE_TYPES: &[SourceType; 1] = &[SourceType::JavaScript];

#[cacheable_dyn]
#[async_trait::async_trait]
impl ParserAndGenerator for JavaScriptParserAndGenerator {
  fn source_types(&self, _module: &dyn Module, _module_graph: &ModuleGraph) -> &[SourceType] {
    SOURCE_TYPES
  }

  fn size(&self, module: &dyn Module, _source_type: Option<&SourceType>) -> f64 {
    module.source().map_or(0, |source| source.size()) as f64
  }

  #[tracing::instrument("JavaScriptParser:parse", skip_all,fields(
    resource = parse_context.resource_data.resource()
  ))]
  async fn parse<'a>(
    &mut self,
    parse_context: ParseContext<'a>,
  ) -> Result<TWithDiagnosticArray<ParseResult>> {
    let ParseContext {
      source,
      module_type,
      module_layer,
      resource_data,
      compiler_options,
      runtime_template,
      factory_meta,
      build_info,
      build_meta,
      module_identifier,
      loaders,
      module_parser_options,
      mut parse_meta,
      ..
    } = parse_context;
    let mut diagnostics: Vec<Diagnostic> = vec![];

    if let Some(collected_ts_info) = parse_meta.remove(COLLECTED_TYPESCRIPT_INFO_PARSE_META_KEY)
      && let Ok(collected_ts_info) =
        (collected_ts_info as Box<dyn std::any::Any>).downcast::<CollectedTypeScriptInfo>()
    {
      build_info.collected_typescript_info = Some(*collected_ts_info);
    }

    let default_with_diagnostics = |source: Arc<dyn Source>, diagnostics: Vec<Diagnostic>| {
      Ok(
        ParseResult {
          source,
          dependencies: vec![],
          blocks: vec![],
          presentational_dependencies: vec![],
          code_generation_dependencies: vec![],
          side_effects_bailout: None,
        }
        .with_diagnostic(map_box_diagnostics_to_module_parse_diagnostics(
          diagnostics,
          loaders,
        )),
      )
    };

    let source = remove_bom(source);
    let source_string = source.source().into_string_lossy();

    let comments = SingleThreadedComments::default();
    let target = ast::EsVersion::EsNext;

    let jsx = module_parser_options
      .and_then(|options| options.get_javascript())
      .and_then(|options| options.jsx)
      .unwrap_or(false);

    let parser_lexer = Lexer::new(
      Syntax::Es(EsSyntax {
        jsx,
        allow_return_outside_function: matches!(
          module_type,
          ModuleType::JsDynamic | ModuleType::JsAuto
        ),
        explicit_resource_management: true,
        import_attributes: true,
        ..Default::default()
      }),
      target,
      SourceFileInput::new(
        &source_string,
        BytePos(1),
        BytePos(source_string.len() as u32 + 1),
      ),
      Some(&comments),
    );

    let javascript_compiler = JavaScriptCompiler::new();

    let (mut ast, tokens) = match javascript_compiler.parse_with_lexer(
      &source_string,
      parser_lexer,
      module_type_to_is_module(module_type),
      Some(comments.clone()),
      true,
    ) {
      Ok(ast) => ast,
      Err(e) => {
        diagnostics.append(&mut e.into_inner().into_iter().map(|e| e.into()).collect());
        return default_with_diagnostics(source, diagnostics);
      }
    };

    let mut semicolons = Default::default();
    let mut concatenated_module_global_ctxt = SyntaxContext::empty();
    let mut concatenated_module_ctxt = SyntaxContext::empty();
    ast.transform(|program, context| {
      program.visit_mut_with(&mut paren_remover(Some(&comments)));
      concatenated_module_global_ctxt =
        concatenated_module_global_ctxt.apply_mark(context.unresolved_mark);
      concatenated_module_ctxt = concatenated_module_ctxt.apply_mark(context.top_level_mark);
      program.visit_mut_with(&mut resolver(
        context.unresolved_mark,
        context.top_level_mark,
        false,
      ));
      program.visit_with(&mut semicolon::InsertedSemicolons::new(
        &mut semicolons,
        // safety: it's safe to assert tokens is some since we pass with_tokens = true
        tokens.as_deref().expect("should get tokens from parser"),
      ));
    });
    ast.visit(|program, _| {
      build_info.concatenated_module_scope_info = Some(collect_concatenated_module_scope_info(
        program.get_inner_program(),
        concatenated_module_global_ctxt,
        concatenated_module_ctxt,
      ));
    });

    let unresolved_mark = ast.get_context().unresolved_mark;
    let parser_runtime_requirements = ParserRuntimeRequirementsData::new(runtime_template);

    let ScanDependenciesResult {
      dependencies,
      blocks,
      presentational_dependencies,
      mut warning_diagnostics,
      mut side_effects_item,
    } = match ast.visit(|program, _| {
      scan_dependencies(
        &source_string,
        program,
        resource_data,
        compiler_options,
        module_type,
        module_layer,
        factory_meta,
        build_meta,
        build_info,
        module_identifier,
        module_parser_options,
        &mut semicolons,
        unresolved_mark,
        &mut self.parser_plugins,
        parse_meta,
        &parser_runtime_requirements,
      )
    }) {
      Ok(result) => result,
      Err(mut e) => {
        diagnostics.append(&mut e);
        return default_with_diagnostics(source, diagnostics);
      }
    };
    diagnostics.append(&mut warning_diagnostics);
    let mut side_effects_bailout = None;

    if compiler_options.optimization.side_effects.is_true() {
      let has_side_effects = side_effects_item.is_some();
      build_meta.side_effect_free = Some(!has_side_effects);
      if has_side_effects {
        build_info.deferred_pure_checks.clear();
      }
      side_effects_bailout = side_effects_item.take().and_then(|item| -> Option<_> {
        let msg = item.loc?.to_string();
        Some(SideEffectsBailoutItem { msg, ty: item.ty })
      });
    }

    Ok(
      ParseResult {
        source,
        dependencies,
        blocks,
        presentational_dependencies,
        code_generation_dependencies: vec![],
        side_effects_bailout,
      }
      .with_diagnostic(map_box_diagnostics_to_module_parse_diagnostics(
        diagnostics,
        loaders,
      )),
    )
  }

  async fn generate(
    &self,
    source: &BoxSource,
    module: &dyn Module,
    generate_context: &mut GenerateContext,
  ) -> Result<BoxSource> {
    if matches!(
      generate_context.requested_source_type,
      SourceType::JavaScript
    ) {
      let mut source = ReplaceSource::new(source.clone());
      let compilation = generate_context.compilation;
      let mut init_fragments = vec![];
      let mut context = TemplateContext {
        compilation,
        module,
        init_fragments: &mut init_fragments,
        runtime: generate_context.runtime,
        concatenation_scope: generate_context.concatenation_scope.take(),
        data: generate_context.data,
        runtime_template: generate_context.runtime_template,
      };

      module.get_dependencies().iter().for_each(|dependency_id| {
        self.source_dependency(compilation, dependency_id, &mut source, &mut context)
      });

      if let Some(dependencies) = module.get_presentational_dependencies() {
        dependencies.iter().for_each(|dependency| {
          if let Some(template) = dependency
            .dependency_template()
            .and_then(|template_type| compilation.get_dependency_template(template_type))
          {
            template.render(dependency.as_ref(), &mut source, &mut context)
          } else {
            panic!(
              "Can not find dependency template of {:?}",
              dependency.dependency_template()
            );
          }
        });
      };

      module
        .get_blocks()
        .iter()
        .for_each(|block_id| self.source_block(compilation, block_id, &mut source, &mut context));
      if let Some(scope) = context.concatenation_scope.as_deref() {
        scope.apply_top_level_renames(module, &mut source);
      }
      generate_context.concatenation_scope = context.concatenation_scope.take();
      render_init_fragments(source.boxed(), init_fragments, generate_context)
    } else {
      panic!(
        "Unsupported source type: {:?}",
        generate_context.requested_source_type
      )
    }
  }

  fn get_concatenation_bailout_reason(
    &self,
    module: &dyn rspack_core::Module,
    _mg: &ModuleGraph,
    _cg: &ChunkGraph,
  ) -> Option<Cow<'static, str>> {
    // Only ES modules are valid for optimization
    if module.build_meta().exports_type != BuildMetaExportsType::Namespace {
      return Some("Module is not an ECMAScript module".into());
    }

    if let Some(deps) = module.get_presentational_dependencies() {
      if !deps.iter().any(|dep| {
        // https://github.com/webpack/webpack/blob/b9fb99c63ca433b24233e0bbc9ce336b47872c08/lib/javascript/JavascriptGenerator.js#L65-L74
        dep
          .as_any()
          .downcast_ref::<ESMCompatibilityDependency>()
          .is_some()
      }) {
        return Some("Module is not an ECMAScript module".into());
      }
    } else {
      return Some("Module is not an ECMAScript module".into());
    }

    if let Some(bailout) = module.build_info().module_concatenation_bailout.as_deref() {
      return Some(format!("Module uses {bailout}").into());
    }
    None
  }
}
