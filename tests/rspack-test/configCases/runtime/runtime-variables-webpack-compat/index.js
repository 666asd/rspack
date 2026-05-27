const value = require("./dep");

it("should run with rspack runtime variables", () => {
  expect(value).toBe(1);
  expect(DEFINED_DEP).toBe(1);
});
