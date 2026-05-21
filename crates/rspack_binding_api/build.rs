fn main() {
  // We deliberately do not call `napi_build::setup()` here because this is a
  // [lib] crate, not a cdylib. `napi_build` would otherwise emit
  // `cargo:rustc-cdylib-link-arg=...` on macOS and cargo warns that the
  // directive is wasted. Replicate only the `rerun-if-env-changed` bits so the
  // `#[napi]` proc macros still rebuild when the napi CLI bumps
  // `NAPI_FORCE_BUILD_RSPACK_BINDING_API` (e.g. via `--no-dts-cache`).
  println!("cargo:rerun-if-env-changed=DEBUG_GENERATED_CODE");
  println!("cargo:rerun-if-env-changed=TYPE_DEF_TMP_PATH");
  println!("cargo:rerun-if-env-changed=CARGO_CFG_NAPI_RS_CLI_VERSION");
  println!("cargo::rerun-if-env-changed=NAPI_DEBUG_GENERATED_CODE");
  println!("cargo::rerun-if-env-changed=NAPI_TYPE_DEF_TMP_FOLDER");
  println!("cargo::rerun-if-env-changed=NAPI_FORCE_BUILD_RSPACK_BINDING_API");

  println!("cargo::rustc-check-cfg=cfg(tokio_unstable)");
}
