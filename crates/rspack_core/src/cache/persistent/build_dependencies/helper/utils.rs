use rspack_paths::{Utf8Component, Utf8Path};

/// Check if a path is a node package path.
pub fn is_node_package_path(path: &Utf8Path) -> bool {
  let mut result = false;
  for comp in path.components() {
    if let Utf8Component::Normal(comp) = comp {
      if comp == "node_modules" {
        result = true;
      }
      if comp.starts_with('.') {
        result = false;
      }
    }
  }
  result
}

#[cfg(test)]
mod test {
  use rspack_paths::{ArcPath, Utf8PathBuf};

  use super::is_node_package_path;

  fn generate_arc_path(path: &str) -> ArcPath {
    ArcPath::from(Utf8PathBuf::from(path))
  }

  #[test]
  fn check_is_node_package() {
    assert!(!is_node_package_path(&generate_arc_path(
      "/root/a/index.js"
    )),);
    assert!(is_node_package_path(&generate_arc_path(
      "/root/node_modules/a/index.js"
    )),);
    assert!(!is_node_package_path(&generate_arc_path(
      "/root/node_modules/.a/index.js"
    )),);
    assert!(is_node_package_path(&generate_arc_path(
      "/root/node_modules/.a/node_modules/a/index.js"
    )),);
  }
}
