macro_rules! case_entry {
  ($register:expr) => {
    use criterion::criterion_group;
    use rspack_benchmark::Criterion;

    pub fn bench(c: &mut Criterion) {
      ($register)(c);
    }

    criterion_group!(case, bench);
  };
}

pub mod build_chunk_graph;
pub mod build_module_graph;
pub mod bundle_basic_react_development;
pub mod bundle_basic_react_production_sourcemap;
pub mod bundle_threejs_development;
pub mod bundle_threejs_production_sourcemap;
pub mod module_graph_api;
pub mod scan_dependencies;
