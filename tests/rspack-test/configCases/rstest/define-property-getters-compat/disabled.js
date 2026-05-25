it('should allow disabling define property getters compatibility', () => {
  expect(__webpack_require__.d).toBeDefined();
  expect(__webpack_require__.rstest_define_property_getters).toBeUndefined();
});
