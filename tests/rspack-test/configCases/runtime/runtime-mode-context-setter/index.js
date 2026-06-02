__webpack_nonce__ = "nonce-value";
__webpack_public_path__ = "/assets/";
export const value = __webpack_nonce__;
export const publicPath = __webpack_public_path__;

it("keeps runtime API assignment readable", () => {
  expect(value).toBe("nonce-value");
  expect(publicPath).toBe("/assets/");
});
