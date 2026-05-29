const value = require("./dep");
const { rspackExportConflict } = require("./rspack-exports-conflict");

it("should run with rspack runtime variables", () => {
  expect(value).toBe(1);
  expect(DEFINED_DEP).toBe(1);
  expect(rspackExportConflict).toBe(2);
});
