function runtimeProxyStaticWrite() {}

__rspack_runtime.d = runtimeProxyStaticWrite;

export const writeSyncedToRuntimeVariable =
  __rspack_runtime.d === runtimeProxyStaticWrite;
