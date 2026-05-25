it('should keep legacy define property getters calls compatible by default', () => {
  expect(__webpack_require__.d).toBeDefined();
  expect(__webpack_require__.rstest_define_property_getters).toBeDefined();

  const exports = {};
  __webpack_require__.d(exports, {
    answer: () => 42,
  });

  expect(exports.answer).toBe(42);
});
