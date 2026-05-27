function escapeRegExp(string) {
  return string.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function isAsyncModule(content, moduleId) {
  const regex = new RegExp(`\\"${escapeRegExp(moduleId)}\\".*\\(.*\\).*\\{\\s([\\S\\s]*?)__rspack_require\\.r\\(__rspack_exports\\);`)
  const result = regex.exec(content)
  try {
    const [, header] = result;
    return header.includes("__rspack_require.a(")
  } catch (e) {
    console.log(content, moduleId, result)
    throw e;
  }
}

export function hasAsyncModuleRuntime(content) {
  const comment = "// " + ["webpack", "runtime", "async_module"].join("/");
  return content.includes(comment)
}
