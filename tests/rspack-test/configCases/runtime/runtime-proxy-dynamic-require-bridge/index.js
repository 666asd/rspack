const runtimeKey = "d";

export const dynamicRequireBridgeType = typeof __webpack_require__[runtimeKey];

it("should bridge dynamic __webpack_require__ access", () => {
	expect(dynamicRequireBridgeType).toBe("function");
});
