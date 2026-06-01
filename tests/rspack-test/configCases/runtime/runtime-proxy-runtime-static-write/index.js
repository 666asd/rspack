function runtimeProxyStaticWrite() {}

__rspack_context.d = runtimeProxyStaticWrite;

export const writeSyncedToRuntimeVariable =
  __rspack_context.d === runtimeProxyStaticWrite;
