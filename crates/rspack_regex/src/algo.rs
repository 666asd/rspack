use std::{borrow::Cow, fmt::Debug, hash::Hash};

use regex::RegexBuilder;
use regex_syntax::hir::{Hir, HirKind, Look, literal::ExtractKind};
use regress::Match;
use rspack_error::{Error, error};

#[derive(Clone)]
pub struct HashRegressRegex {
  pub regex: regress::Regex,
  expr: String,
  flags: String,
}

impl Hash for HashRegressRegex {
  fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
    self.expr.hash(state);
    self.flags.hash(state)
  }
}

impl Debug for HashRegressRegex {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    Debug::fmt(&self.regex, f)
  }
}

impl HashRegressRegex {
  pub(crate) fn new(expr: &str, flags: &str) -> Result<Self, Error> {
    match regress::Regex::with_flags(expr, flags) {
      Ok(regex) => Ok(Self {
        regex,
        expr: expr.to_string(),
        flags: flags.to_string(),
      }),
      Err(err) => Err(error!(
        "Can't construct regex `/{expr}/{flags}`, original error message: {err}"
      )),
    }
  }

  fn find(&self, text: &str) -> Option<Match> {
    self.regex.find(text)
  }
}

#[derive(Clone)]
pub struct HashRustRegex {
  pub regex: regex::Regex,
  expr: String,
  flags: String,
}

impl Hash for HashRustRegex {
  fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
    self.expr.hash(state);
    self.flags.hash(state)
  }
}

impl Debug for HashRustRegex {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    Debug::fmt(&self.regex, f)
  }
}

impl HashRustRegex {
  pub(crate) fn new(expr: &str, flags: &str) -> Result<Self, Error> {
    // Rust regex doesn't allow escaped slashes, but they are necessary in JS
    // regexp literals.
    let pattern = normalize_escaped_slashes(expr);
    let mut builder = RegexBuilder::new(&pattern);
    let has_ignore_case = flags.contains('i');
    let has_unicode = flags.contains('u');

    // Without the regex crate's `unicode-case` feature, case-insensitive
    // matching is only reliable for ASCII patterns in non-unicode mode.
    if has_ignore_case && has_unicode {
      return Err(error!("Unsupported regex flag combination `iu` for rust regex"));
    }
    if has_ignore_case {
      if !expr.is_ascii() {
        return Err(error!(
          "Unsupported non-ascii regex with `i` flag for rust regex"
        ));
      }
      builder.unicode(false);
    }

    for flag in flags.chars() {
      match flag {
        // Indices for substring matches are not relevant for test().
        'd' => {}
        // Global is the default for Rust regex matching.
        'g' => {}
        'i' => {
          builder.case_insensitive(true);
        }
        'm' => {
          builder.multi_line(true);
        }
        's' => {
          builder.dot_matches_new_line(true);
        }
        'u' => {
          builder.unicode(true);
        }
        // Sticky changes where matching may start, so keep it on Regress.
        _ => {
          return Err(error!("Unsupported regex flag `{flag}` for rust regex"));
        }
      }
    }
    match builder.build() {
      Ok(regex) => Ok(Self {
        regex,
        expr: expr.to_string(),
        flags: flags.to_string(),
      }),
      Err(err) => Err(error!(
        "Can't construct rust regex `/{expr}/{flags}`, original error message: {err}"
      )),
    }
  }
}

#[derive(Clone, Debug, Hash)]
pub enum Algo {
  /// Regress is considered having the same behaviors as RegExp in JS.
  /// But Regress has poor performance. To improve performance of regex matching,
  /// we would try to use some fast algo to do matching, when we detect some special pattern.
  /// See details at https://github.com/web-infra-dev/rspack/pull/3113
  EndWith {
    pats: Vec<String>,
    ignore_case: bool,
  },
  RustRegex(HashRustRegex),
  Regress(HashRegressRegex),
}

impl Algo {
  pub(crate) fn new(expr: &str, flags: &str) -> Result<Algo, Error> {
    if let Some(algo) = Self::try_compile_to_end_with_fast_path(expr, flags) {
      Ok(algo)
    } else {
      HashRustRegex::new(expr, flags)
        .map(Algo::RustRegex)
        .or_else(|_| HashRegressRegex::new(expr, flags).map(Algo::Regress))
    }
  }

  pub(crate) fn new_rust_regex(expr: &str, flags: &str) -> Result<Algo, Error> {
    HashRustRegex::new(expr, flags).map(Algo::RustRegex)
  }

  fn try_compile_to_end_with_fast_path(expr: &str, flags: &str) -> Option<Algo> {
    // Only optimize when flags are a subset of those that do not affect simple
    // suffix semantics for the inputs we care about (paths/extensions).
    // - 'g' doesn't affect a single `test()` call.
    // - 'i' is handled explicitly via `ignore_case`.
    // - 'y' (sticky) changes the allowed start position of matches, so we must
    //   conservatively bail out of this fast path when it is present.
    let mut ignore_case = false;
    for flag in flags.chars() {
      match flag {
        'i' => {
          ignore_case = true;
        }
        'g' => {}
        // Any other flag (including 'y' sticky) is unsupported for the fast
        // path; fall back to Regress for full JS semantics.
        _ => {
          return None;
        }
      }
    }

    let hir = regex_syntax::parse(expr).ok()?;
    let seq = regex_syntax::hir::literal::Extractor::new()
      .kind(ExtractKind::Suffix)
      .extract(&hir);
    if is_ends_with_regex(&hir) && seq.is_exact() {
      let literals = seq.literals()?;
      let mut pats = Vec::with_capacity(literals.len());

      if ignore_case {
        // Only use case-insensitive fast path when all suffix literals are ASCII.
        for item in literals.iter() {
          let bytes = item.as_bytes();
          if !bytes.iter().all(u8::is_ascii) {
            return None;
          }
          pats.push(String::from_utf8_lossy(bytes).to_string());
        }
      } else {
        for item in literals.iter() {
          pats.push(String::from_utf8_lossy(item.as_bytes()).to_string());
        }
      }

      Some(Algo::EndWith { pats, ignore_case })
    } else {
      None
    }
  }

  pub(crate) fn test(&self, str: &str) -> bool {
    match self {
      Algo::RustRegex(regex) => regex.regex.is_match(str),
      Algo::Regress(regex) => regex.find(str).is_some(),
      Algo::EndWith { pats, ignore_case } => {
        if *ignore_case {
          pats
            .iter()
            .any(|pat| ends_with_ascii_case_insensitive(str, pat))
        } else {
          pats.iter().any(|pat| str.ends_with(pat))
        }
      }
    }
  }
}

fn normalize_escaped_slashes(pattern: &str) -> Cow<'_, str> {
  if !pattern.contains("\\/") {
    return Cow::Borrowed(pattern);
  }

  let mut normalized = String::with_capacity(pattern.len());
  let mut chars = pattern.chars().peekable();
  let mut in_character_class = false;

  while let Some(ch) = chars.next() {
    match ch {
      '\\' => {
        if chars.peek() == Some(&'/') && !in_character_class {
          chars.next();
          normalized.push('/');
        } else {
          normalized.push(ch);
          if let Some(next) = chars.next() {
            normalized.push(next);
          }
        }
      }
      '[' if !in_character_class => {
        in_character_class = true;
        normalized.push(ch);
      }
      ']' if in_character_class => {
        in_character_class = false;
        normalized.push(ch);
      }
      _ => {
        normalized.push(ch);
      }
    }
  }

  Cow::Owned(normalized)
}

fn is_ends_with_regex(hir: &Hir) -> bool {
  if let HirKind::Concat(list) = hir.kind() {
    list[0].kind() != &HirKind::Look(Look::Start)
      && list[list.len() - 1].kind() == &HirKind::Look(Look::End)
  } else {
    false
  }
}

fn ends_with_ascii_case_insensitive(s: &str, pat: &str) -> bool {
  let s_bytes = s.as_bytes();
  let pat_bytes = pat.as_bytes();
  let s_len = s_bytes.len();
  let pat_len = pat_bytes.len();

  if pat_len > s_len {
    return false;
  }

  let start = s_len - pat_len;
  for i in 0..pat_len {
    let sc = s_bytes[start + i].to_ascii_lowercase();
    let pc = pat_bytes[i].to_ascii_lowercase();
    if sc != pc {
      return false;
    }
  }
  true
}

#[cfg(test)]
mod test_algo {
  use super::*;

  impl Algo {
    fn end_with_pats(&self) -> std::collections::HashSet<&str> {
      match self {
        Algo::EndWith { pats, .. } => pats.iter().map(|s| s.as_str()).collect(),
        Algo::Regress(_) | Algo::RustRegex(_) => panic!("expect EndWith"),
      }
    }

    fn is_end_with(&self) -> bool {
      matches!(self, Self::EndWith { .. })
    }

    fn is_regress(&self) -> bool {
      matches!(self, Self::Regress(..))
    }

    fn is_rust_regex(&self) -> bool {
      matches!(self, Self::RustRegex(..))
    }
  }

  #[test]
  fn should_use_end_with_algo_with_i_flag() {
    assert!(Algo::new("\\.js$", "").unwrap().is_end_with());
    assert!(Algo::new("\\.js$", "i").unwrap().is_end_with());
  }

  #[test]
  fn end_with_ignore_case_matches_ascii_suffix() {
    let algo = Algo::new("\\.js$", "i").unwrap();
    assert!(algo.is_end_with());
    assert!(algo.test("file.js"));
    assert!(algo.test("file.JS"));
    assert!(algo.test("file.Js"));
    assert!(algo.test("file.jS"));
    assert!(!algo.test("filejsx"));
  }

  #[test]
  fn end_with_ignore_case_matches_regress_for_ascii() {
    let algo = Algo::new("\\.js$", "i").unwrap();
    let regress = HashRegressRegex::new("\\.js$", "i").unwrap();
    let samples = [
      "", "file", "file.js", "file.JS", "file.Js", "file.jS", "FILE.JS", "foo.jsx", "foojson",
      "foo.JSON",
    ];

    for s in samples {
      assert_eq!(algo.test(s), regress.find(s).is_some(), "mismatch on {s}");
    }
  }

  #[test]
  fn correct_end_with() {
    use std::collections::HashSet;
    let algo = Algo::new("\\.js$", "").unwrap();
    assert_eq!(algo.end_with_pats(), HashSet::from([".js"]));
    let algo = Algo::new("\\.(jsx?|tsx?)$", "").unwrap();
    assert_eq!(
      algo.end_with_pats(),
      HashSet::from([".jsx", ".tsx", ".js", ".ts"])
    );
    let algo = Algo::new("\\.(svg|png)$", "").unwrap();
    assert_eq!(algo.end_with_pats(), HashSet::from([".svg", ".png"]));
  }

  #[test]
  fn check_slow_path() {
    // this is a full match
    assert!(Algo::new("^\\.(svg|png)$", "").unwrap().is_rust_regex());
    // wildcard match
    assert!(Algo::new("\\..(svg|png)$", "").unwrap().is_rust_regex());
  }

  #[test]
  fn should_try_rust_regex_before_regress() {
    let algo = Algo::new("^foo.*bar$", "").unwrap();
    assert!(algo.is_rust_regex());
    assert!(algo.test("foo/bar"));
    assert!(!algo.test("foo/bar/baz"));
  }

  #[test]
  fn turbopack_es_regex_matches_simple() {
    let algo = Algo::new("a", "").unwrap();
    assert!(algo.is_rust_regex());
    assert!(algo.test("a"));
  }

  #[test]
  fn turbopack_es_regex_matches_negative_lookahead() {
    let algo = Algo::new("a(?!b)", "").unwrap();
    assert!(algo.is_regress());
    assert!(!algo.test("ab"));
    assert!(algo.test("ac"));
  }

  #[test]
  fn turbopack_invalid_regex() {
    assert!(Algo::new("*", "").is_err());
  }

  #[test]
  fn rust_regex_path_supports_safe_js_flags() {
    let algo = Algo::new("^bar.baz$", "dgmsu").unwrap();
    assert!(algo.is_rust_regex());
    assert!(algo.test("foo\nbar\nbaz\nqux"));
    assert!(!algo.test("foo\nbar\nbaz!"));
  }

  #[test]
  fn sticky_flag_should_fallback_to_regress() {
    let algo = Algo::new("\\.js$", "y").unwrap();
    assert!(algo.is_regress());
  }

  #[test]
  fn should_fallback_to_regress_for_js_only_regex_syntax() {
    let algo = Algo::new("(?<=foo)bar", "").unwrap();
    assert!(algo.is_regress());
    assert!(algo.test("foobar"));
    assert!(!algo.test("bar"));
  }

  #[test]
  fn anchored_sticky_flag_should_fallback_to_regress() {
    let algo = Algo::new("^foo", "y").unwrap();
    assert!(algo.is_regress());
  }

  #[test]
  fn unicode_sets_flag_should_fallback_to_regress() {
    let algo = Algo::new("[a--b]", "v").unwrap();
    assert!(algo.is_regress());
  }

  #[test]
  fn unicode_flag_should_use_rust_regex() {
    let algo = Algo::new("^foo$", "u").unwrap();
    assert!(algo.is_rust_regex());
  }

  #[test]
  fn indices_flag_should_use_rust_regex() {
    let algo = Algo::new("^foo$", "d").unwrap();
    assert!(algo.is_rust_regex());
  }

  #[test]
  fn escaped_slash_should_use_rust_regex() {
    let algo = Algo::new("foo\\/bar", "").unwrap();
    assert!(algo.is_rust_regex());
    assert!(algo.test("foo/bar"));
  }

  #[test]
  fn escaped_slash_in_character_class_should_keep_backslash_alternative() {
    let algo = Algo::new("[\\\\/]", "").unwrap();
    assert!(algo.is_rust_regex());
    assert!(algo.test("/"));
    assert!(algo.test("\\"));
  }

  #[test]
  fn non_ascii_ignore_case_should_fallback_to_regress() {
    let algo = Algo::new("é", "i").unwrap();
    assert!(algo.is_regress());
  }

  #[test]
  fn ignore_case_with_unicode_flag_should_fallback_to_regress() {
    let algo = Algo::new("^foo$", "iu").unwrap();
    assert!(algo.is_regress());
  }

  #[test]
  fn ignore_case_flag_should_use_rust_regex_outside_fast_path() {
    let algo = Algo::new("^foo$", "i").unwrap();
    assert!(algo.is_rust_regex());
  }

  #[test]
  fn check_rust_regex_path() {
    assert!(
      Algo::new_rust_regex("^\\.(svg|png)$", "")
        .unwrap()
        .is_rust_regex()
    );
    assert!(Algo::new_rust_regex("\\.js$", "").unwrap().is_rust_regex());
  }

  #[test]
  fn rust_regex_flags() {
    let regex = Algo::new_rust_regex("foo", "g").unwrap();
    assert!(regex.test("foo"));
  }

  #[test]
  fn sticky_flag_should_not_use_end_with_fast_path() {
    // In JS, `/\.js$/y.test("foo.js")` is false because the sticky flag forces
    // the match to start at lastIndex (0 by default), so the suffix-only check
    // is not semantically correct. We therefore fall back to Regress.
    let algo = Algo::new("\\.js$", "y").unwrap();
    let regress = HashRegressRegex::new("\\.js$", "y").unwrap();

    assert!(algo.is_regress());
    let samples = ["foo.js", "bar.jsx", ".js", "js"];
    for s in samples {
      assert_eq!(algo.test(s), regress.find(s).is_some(), "mismatch on {s}");
    }
  }
}
