use std::{
  borrow::Cow,
  hash::Hasher,
  path::Path,
  sync::{Arc, LazyLock},
};

use cow_utils::CowUtils;
use heck::{ToKebabCase, ToLowerCamelCase};
use regex::{Captures, Regex};
use rspack_core::{
  ChunkGraph, Compilation, CompilerOptions, CssExport, CssExportsConvention, GenerateContext,
  LocalIdentName, ModuleArgument, ModuleCodeTemplate, PathData, RESERVED_IDENTIFIER, ResourceData,
  RuntimeGlobals, RuntimeSpec, UsedNameItem,
  rspack_sources::{ConcatSource, RawStringSource},
  to_identifier,
};
use rspack_error::{Diagnostic, Error, Result, Severity};
use rspack_hash::{HashDigest, HashFunction, HashSalt, RspackHash};
use rspack_util::{
  atom::Atom,
  fx_hash::{FxIndexMap, FxIndexSet},
  identifier::make_paths_relative,
  itoa, json_stringify_str,
};
use rustc_hash::FxHashSet as HashSet;

use crate::runtime::CSS_MODULE_EXPORTS_RENDERED_TEMPLATE_ID;

pub const AUTO_PUBLIC_PATH_PLACEHOLDER: &str = "__RSPACK_PLUGIN_CSS_AUTO_PUBLIC_PATH__";
pub static LEADING_DIGIT_REGEX: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^((-?[0-9])|--)").expect("Invalid regexp"));
static CSS_PREPARE_ID_LEADING_REGEX: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^([.-]|[^a-z0-9_-])+").expect("Invalid regexp"));
static CSS_PREPARE_ID_REST_REGEX: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"[^a-z0-9@_-]+").expect("Invalid regexp"));

fn css_prepare_id(v: &str) -> String {
  let without_leading = CSS_PREPARE_ID_LEADING_REGEX.replace(v, "");
  CSS_PREPARE_ID_REST_REGEX
    .replace_all(&without_leading, "_")
    .into_owned()
}

#[derive(Debug, Clone)]
pub struct LocalIdentOptions<'a> {
  relative_resource: String,
  local_name_ident: &'a LocalIdentName,
  compiler_options: &'a CompilerOptions,
  local_ident_hash_digest: Option<&'a HashDigest>,
  local_ident_hash_digest_length: Option<usize>,
  local_ident_hash_function: Option<&'a HashFunction>,
  local_ident_hash_salt: Option<&'a HashSalt>,
}

impl<'a> LocalIdentOptions<'a> {
  pub fn new(
    resource_data: &ResourceData,
    local_name_ident: &'a LocalIdentName,
    compiler_options: &'a CompilerOptions,
    local_ident_hash_digest: Option<&'a HashDigest>,
    local_ident_hash_digest_length: Option<usize>,
    local_ident_hash_function: Option<&'a HashFunction>,
    local_ident_hash_salt: Option<&'a HashSalt>,
  ) -> Self {
    let relative_resource =
      make_paths_relative(&compiler_options.context, resource_data.resource());
    Self {
      relative_resource,
      local_name_ident,
      compiler_options,
      local_ident_hash_digest,
      local_ident_hash_digest_length,
      local_ident_hash_function,
      local_ident_hash_salt,
    }
  }

  pub async fn get_local_ident(&self, local: &str) -> Result<String> {
    let output = &self.compiler_options.output;
    let hash_function = self
      .local_ident_hash_function
      .unwrap_or(&output.hash_function);
    let hash_salt = self.local_ident_hash_salt.unwrap_or(&output.hash_salt);
    let hash_digest = self.local_ident_hash_digest.unwrap_or(&output.hash_digest);
    let hash_digest_length = self
      .local_ident_hash_digest_length
      .unwrap_or(output.hash_digest_length);
    let hash = {
      let mut hasher = RspackHash::with_salt(hash_function, hash_salt);
      hasher.write(self.relative_resource.as_bytes());
      let contains_local = self
        .local_name_ident
        .template
        .template()
        .map(|t| t.contains("[local]"))
        .unwrap_or_default();
      if !contains_local {
        hasher.write(local.as_bytes());
      }
      let hash = hasher.digest(hash_digest);
      hash.rendered(hash_digest_length).to_string()
    };
    let id = css_prepare_id(if self.compiler_options.mode.is_development() {
      &self.relative_resource
    } else {
      &hash
    });
    let local_ident = LocalIdentNameRenderOptions {
      path_data: PathData::default()
        .filename(&self.relative_resource)
        .hash(&hash)
        .id(&id),
      local,
      unique_name: &output.unique_name,
      folder: Path::new(&self.relative_resource)
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or(""),
    }
    .render_local_ident_name(self.local_name_ident)
    .await?;
    Ok(
      LEADING_DIGIT_REGEX
        .replace(&local_ident, "_${1}")
        .into_owned(),
    )
  }
}

struct LocalIdentNameRenderOptions<'a> {
  path_data: PathData<'a>,
  local: &'a str,
  unique_name: &'a str,
  folder: &'a str,
}

impl LocalIdentNameRenderOptions<'_> {
  pub async fn render_local_ident_name(self, local_ident_name: &LocalIdentName) -> Result<String> {
    let raw = local_ident_name
      .template
      .render(self.path_data, None)
      .await?;
    let s: &str = raw.as_ref();

    Ok(
      s.cow_replace("[uniqueName]", self.unique_name)
        .cow_replace("[local]", self.local)
        .cow_replace("[folder]", self.folder)
        .into_owned(),
    )
  }
}

static UNESCAPE_CSS_IDENT_REGEX: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"([^a-zA-Z0-9_\u0081-\uffff-])").expect("invalid regex"));

pub fn escape_css(s: &str) -> Cow<'_, str> {
  UNESCAPE_CSS_IDENT_REGEX.replace_all(s, |s: &Captures| format!("\\{}", &s[0]))
}

pub(crate) fn export_locals_convention(
  key: &str,
  locals_convention: &CssExportsConvention,
) -> Vec<String> {
  let mut res = Vec::with_capacity(3);
  if locals_convention.as_is() {
    res.push(key.to_string());
  }
  if locals_convention.camel_case() {
    res.push(key.to_lower_camel_case());
  }
  if locals_convention.dashes() {
    res.push(key.to_kebab_case());
  }
  res
}

pub(crate) fn render_css_module_template(
  compilation: &Compilation,
  key: &str,
  params: serde_json::Value,
) -> Result<String> {
  compilation
    .runtime_template
    .create_runtime_code_template()
    .render(key, Some(params))
}

pub(crate) struct CssModuleExportsRenderOptions<'a> {
  pub is_inject_call: bool,
  pub css_inject_style: Option<&'a str>,
  pub module_id: Option<&'a str>,
  pub css_text: Option<&'a str>,
  pub css_style_sheet: Option<&'a str>,
  pub exports_body: Option<&'a str>,
  pub with_hmr: bool,
  pub ns_obj: &'a str,
  pub left: &'a str,
  pub module_argument: Option<&'a str>,
  pub assignment: Option<&'a str>,
  pub right: &'a str,
  pub default_target: Option<&'a str>,
  pub default_export: Option<&'a str>,
  pub accept_hmr: bool,
}

pub(crate) fn render_css_modules_exports_module_code(
  compilation: &Compilation,
  options: CssModuleExportsRenderOptions<'_>,
) -> Result<String> {
  render_css_module_template(
    compilation,
    CSS_MODULE_EXPORTS_RENDERED_TEMPLATE_ID,
    serde_json::json!({
      "_is_inject_call": options.is_inject_call,
      "_css_inject_style": options.css_inject_style.unwrap_or_default(),
      "_module_id": options.module_id.unwrap_or_default(),
      "_css_text": options.css_text.unwrap_or_default(),
      "_has_css_style_sheet_init": options.css_style_sheet.is_some(),
      "_css_style_sheet": options.css_style_sheet.unwrap_or_default(),
      "_has_exports": options.exports_body.is_some(),
      "_exports_body": options.exports_body.unwrap_or_default(),
      "_with_hmr": options.with_hmr,
      "_ns_obj": options.ns_obj,
      "_left": options.left,
      "_module_argument": options.module_argument.unwrap_or_default(),
      "_assignment": options.assignment.unwrap_or_default(),
      "_right": options.right,
      "_default_target": options.default_target.unwrap_or_default(),
      "_default_export": options.default_export.unwrap_or_default(),
      "_accept_hmr": options.accept_hmr,
    }),
  )
}

#[allow(clippy::too_many_arguments)]
pub fn css_modules_exports_to_string<'a>(
  exports: FxIndexMap<&'a str, &'a FxIndexSet<CssExport>>,
  module: &dyn rspack_core::Module,
  compilation: &Compilation,
  runtime: Option<&RuntimeSpec>,
  runtime_template: &mut ModuleCodeTemplate,
  ns_obj: &str,
  left: &str,
  right: &str,
  with_hmr: bool,
) -> Result<String> {
  let exports_body = stringified_exports(exports, compilation, runtime_template, module, runtime)?;
  let module_argument = runtime_template.render_module_argument(ModuleArgument::Module);
  render_css_modules_exports_module_code(
    compilation,
    CssModuleExportsRenderOptions {
      is_inject_call: false,
      css_inject_style: None,
      module_id: None,
      css_text: None,
      css_style_sheet: None,
      exports_body: Some(&exports_body),
      with_hmr,
      ns_obj,
      left,
      module_argument: Some(&module_argument),
      assignment: Some("exports"),
      right,
      default_target: None,
      default_export: None,
      accept_hmr: false,
    },
  )
}

pub fn stringified_exports<'a>(
  exports: FxIndexMap<&'a str, &'a FxIndexSet<CssExport>>,
  compilation: &Compilation,
  runtime_template: &mut ModuleCodeTemplate,
  module: &dyn rspack_core::Module,
  runtime: Option<&RuntimeSpec>,
) -> Result<String> {
  let mut stringified_exports = String::new();
  let module_graph = compilation.get_module_graph();
  let exports_info = compilation
    .exports_info_artifact
    .get_exports_info_data(&module.identifier());
  for (key, elements) in exports {
    let export_info = exports_info.get_read_only_export_info(&Atom::from(key));
    let used_name = export_info.get_used_name(None, runtime);
    let used_name = match used_name {
      Some(UsedNameItem::Str(name)) => name.to_string(),
      _ => key.to_string(),
    };

    let content = elements
      .iter()
      .map(
        |CssExport {
           ident,
           from,
           id: _,
           orig_name: _,
         }|
         -> Result<String> {
          Ok(match from {
            None => json_stringify_str(ident),
            Some(from_name) => {
              let from = module
                .get_dependencies()
                .iter()
                .find_map(|id| {
                  let dependency = module_graph.dependency_by_id(id);
                  let request = if let Some(d) = dependency.as_module_dependency() {
                    Some(d.request())
                  } else {
                    dependency.as_context_dependency().map(|d| d.request())
                  };
                  if let Some(request) = request
                    && request == from_name
                  {
                    return module_graph.module_graph_module_by_dependency_id(id);
                  }
                  None
                })
                .expect("should have css from module");

              let from_exports_info = compilation
                .exports_info_artifact
                .get_exports_info_data(&from.module_identifier);
              let from_used_name = match from_exports_info
                .get_read_only_export_info(&Atom::from(ident.as_str()))
                .get_used_name(None, runtime)
              {
                Some(UsedNameItem::Str(name)) => json_stringify_str(&unescape(name.as_str())),
                _ => json_stringify_str(&unescape(ident)),
              };

              let from = rspack_util::json_stringify(
                ChunkGraph::get_module_id(&compilation.module_ids_artifact, from.module_identifier)
                  .expect("should have module"),
              );
              format!(
                "{}({from})[{from_used_name}]",
                runtime_template.render_runtime_globals(&RuntimeGlobals::REQUIRE)
              )
            }
          })
        },
      )
      .collect::<Result<Vec<_>>>()?
      .join(" + \" \" + ");
    stringified_exports.push_str(&format!(
      "  {}: {},\n",
      json_stringify_str(&used_name),
      content
    ));
  }

  Ok(stringified_exports)
}

pub fn css_modules_exports_to_concatenate_module_string<'a>(
  exports: FxIndexMap<&'a str, &'a FxIndexSet<CssExport>>,
  module: &dyn rspack_core::Module,
  generate_context: &mut GenerateContext,
  concate_source: &mut ConcatSource,
) -> Result<()> {
  let GenerateContext {
    compilation,
    concatenation_scope,
    runtime,
    runtime_template,
    ..
  } = generate_context;
  let Some(scope) = concatenation_scope else {
    return Ok(());
  };
  let module_graph = compilation.get_module_graph();
  let mut used_identifiers = HashSet::default();
  let exports_info = compilation
    .exports_info_artifact
    .get_exports_info_data(&module.identifier());
  for (key, elements) in exports {
    let export_info = exports_info.get_read_only_export_info(&Atom::from(key));
    let used_name = export_info.get_used_name(None, *runtime);
    let used_name = match used_name {
      Some(UsedNameItem::Str(name)) => name.to_string(),
      _ => key.to_string(),
    };

    let content = elements
      .iter()
      .map(
        |CssExport {
           ident,
           from,
           id: _,
           orig_name: _,
         }|
         -> Result<String> {
          Ok(match from {
            None => json_stringify_str(ident),
            Some(from_name) => {
              let from = module
                .get_dependencies()
                .iter()
                .find_map(|id| {
                  let dependency = module_graph.dependency_by_id(id);
                  let request = if let Some(d) = dependency.as_module_dependency() {
                    Some(d.request())
                  } else {
                    dependency.as_context_dependency().map(|d| d.request())
                  };
                  if let Some(request) = request
                    && request == from_name
                  {
                    return module_graph.module_graph_module_by_dependency_id(id);
                  }
                  None
                })
                .expect("should have css from module");

              let from_exports_info = compilation
                .exports_info_artifact
                .get_exports_info_data(&from.module_identifier);
              let from_used_name = match from_exports_info
                .get_read_only_export_info(&Atom::from(ident.as_str()))
                .get_used_name(None, *runtime)
              {
                Some(UsedNameItem::Str(name)) => json_stringify_str(&name),
                _ => json_stringify_str(ident),
              };

              let from = rspack_util::json_stringify(
                ChunkGraph::get_module_id(&compilation.module_ids_artifact, from.module_identifier)
                  .expect("should have module"),
              );
              format!(
                "{}({from})[{from_used_name}]",
                runtime_template.render_runtime_globals(&RuntimeGlobals::REQUIRE)
              )
            }
          })
        },
      )
      .collect::<Result<Vec<_>>>()?
      .join(" + \" \" + ");
    let mut identifier: Cow<'_, str> = Cow::Owned(to_identifier(&used_name).into_owned());
    if RESERVED_IDENTIFIER.contains(identifier.as_ref()) {
      identifier = Cow::Owned(format!("_{identifier}"));
    }
    let mut i = 0;
    while used_identifiers.contains(&identifier) {
      let mut i_buffer = itoa::Buffer::new();
      let i_str = i_buffer.format(i);
      identifier = Cow::Owned(format!("{identifier}{i_str}"));
      i += 1;
    }
    // TODO: conditional support `const or var` after we finished runtimeTemplate utils
    concate_source.add(RawStringSource::from(format!(
      "var {identifier} = {content};\n"
    )));
    used_identifiers.insert(identifier.clone());
    scope.register_export(key.into(), identifier.into_owned());
  }
  Ok(())
}

static STRING_MULTILINE: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"\\[\n\r\f]").expect("Invalid RegExp"));

static TRIM_WHITE_SPACES: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"(^[ \t\n\r\f]*|[ \t\n\r\f]*$)").expect("Invalid RegExp"));

static UNESCAPE: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"\\([0-9a-fA-F]{1,6}[ \t\n\r\f]?|[\s\S])").expect("Invalid RegExp"));

static DATA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?i)data:").expect("Invalid RegExp"));

// `\/foo` in css should be treated as `foo` in js
pub fn unescape(s: &str) -> Cow<'_, str> {
  UNESCAPE.replace_all(s.as_ref(), |caps: &Captures| {
    caps
      .get(0)
      .and_then(|m| {
        let m = m.as_str();
        if m.len() > 2 {
          if let Ok(r_u32) = u32::from_str_radix(m[1..].trim(), 16)
            && let Some(ch) = char::from_u32(r_u32)
          {
            return Some(format!("{ch}"));
          }
          None
        } else {
          Some(m[1..2].to_string())
        }
      })
      .unwrap_or(caps[0].to_string())
  })
}

static WHITE_OR_BRACKET_REGEX: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r#"[\n\t ()'"\\]"#).expect("Invalid Regexp"));
static QUOTATION_REGEX: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r#"[\n"\\]"#).expect("Invalid Regexp"));
static APOSTROPHE_REGEX: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r#"[\n'\\]"#).expect("Invalid Regexp"));

pub fn css_escape_string(s: &str) -> String {
  let mut count_white_or_bracket = 0;
  let mut count_quotation = 0;
  let mut count_apostrophe = 0;
  for c in s.chars() {
    match c {
      '\t' | '\n' | ' ' | '(' | ')' => count_white_or_bracket += 1,
      '"' => count_quotation += 1,
      '\'' => count_apostrophe += 1,
      _ => {}
    }
  }
  if count_white_or_bracket < 2 {
    WHITE_OR_BRACKET_REGEX
      .replace_all(s, |caps: &Captures| format!("\\{}", &caps[0]))
      .into_owned()
  } else if count_quotation <= count_apostrophe {
    format!(
      "\"{}\"",
      QUOTATION_REGEX.replace_all(s, |caps: &Captures| format!("\\{}", &caps[0]))
    )
  } else {
    format!(
      "\'{}\'",
      APOSTROPHE_REGEX.replace_all(s, |caps: &Captures| format!("\\{}", &caps[0]))
    )
  }
}

pub fn normalize_url(s: &str) -> String {
  let result = STRING_MULTILINE.replace_all(s, "");
  let result = TRIM_WHITE_SPACES.replace_all(&result, "");
  let result = unescape(&result);

  if DATA.is_match(&result) {
    return result.to_string();
  }
  if result.contains('%')
    && let Ok(r) = urlencoding::decode(&result)
  {
    return r.to_string();
  }

  result.to_string()
}

#[allow(clippy::rc_buffer)]
pub fn css_parsing_traceable_error(
  source_code: Arc<String>,
  start: css_module_lexer::Pos,
  end: css_module_lexer::Pos,
  message: impl Into<String>,
  severity: Severity,
) -> Error {
  let mut error = Error::from_string(
    Some(source_code.to_string()),
    start as usize,
    end as usize,
    match severity {
      Severity::Error => "CSS parse error".to_string(),
      Severity::Warning => "CSS parse warning".to_string(),
    },
    message.into(),
  );
  error.severity = severity;
  error
}

pub fn replace_module_request_prefix<'s>(
  specifier: &'s str,
  diagnostics: &mut Vec<Diagnostic>,
  source_code: impl Fn() -> Arc<String>,
  start: css_module_lexer::Pos,
  end: css_module_lexer::Pos,
) -> &'s str {
  if let Some(specifier) = specifier.strip_prefix('~') {
    let mut error = css_parsing_traceable_error(
      source_code(),
      start,
      end,
      "'@import' or 'url()' with a request starts with '~' is deprecated.".to_string(),
      Severity::Warning,
    );
    error.help = Some("Remove '~' from the request.".into());
    diagnostics.push(error.into());
    specifier
  } else {
    specifier
  }
}
