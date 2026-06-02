const direct = __webpack_require__("./lib");
const passedRequire = __webpack_require__;

export const requireType = typeof __webpack_require__;
export const directValue = direct.value;
export const passedRequireType = typeof passedRequire;
export const dynamic = __webpack_require__["notAHelper"];

it("keeps the module-facing require alias as the context require", () => {
  expect(requireType).toBe("function");
  expect(passedRequireType).toBe("function");
  expect(directValue).toBe(42);
});
