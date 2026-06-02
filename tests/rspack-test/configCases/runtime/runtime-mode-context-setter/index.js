__webpack_nonce__ = "nonce-value";
export const value = __webpack_nonce__;

it("keeps runtime API assignment readable", () => {
  expect(value).toBe("nonce-value");
});
