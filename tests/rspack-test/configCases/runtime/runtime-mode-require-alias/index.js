const direct = __webpack_require__("./lib");
const passedRequire = __webpack_require__;
const viaValue = passedRequire("./lib");

export const requireType = typeof __webpack_require__;
export const directValue = direct.value;
export const passedValue = viaValue.value;
export const dynamic = __webpack_require__["notAHelper"];

it("keeps the module-facing require alias callable", () => {
  expect(requireType).toBe("function");
  expect(directValue).toBe(42);
  expect(passedValue).toBe(42);
});
