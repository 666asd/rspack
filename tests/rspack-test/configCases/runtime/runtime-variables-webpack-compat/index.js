const value = require("./dep");

it("should run with rspack runtime variables", () => {
  expect(value).toBe(1);
  expect(typeof __rspack_require).toBe("function");
  expect(typeof __rspack_modules).toBe("object");
  expect(typeof __rspack_module.id).not.toBe("undefined");
});
