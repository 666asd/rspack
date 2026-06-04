use rspack_cacheable::{enable_cacheable as cacheable, from_bytes, to_bytes, with::AsPreset};
use simd_json::OwnedValue as Value;

#[cacheable]
#[derive(Debug, PartialEq)]
struct Module {
  #[cacheable(with=AsPreset)]
  options: Value,
}

#[test]
fn test_preset_simd_json() {
  let module = Module {
    options: simd_json::from_reader("{\"id\":1}".as_bytes()).unwrap(),
  };

  let bytes = to_bytes(&module, &()).unwrap();
  let new_module: Module = from_bytes(&bytes, &()).unwrap();
  assert_eq!(module, new_module);
}
