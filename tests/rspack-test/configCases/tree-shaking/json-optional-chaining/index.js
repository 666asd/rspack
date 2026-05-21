import pkg from "./package.json";

it("should tree shake json properties used through optional chaining", () => {
	expect(pkg?.version).toBe("1.0.0");

	const source = __non_webpack_require__("fs").readFileSync(__filename, "utf-8");
	const unused = ["should", "be", "shaken"].join("-");
	expect(source).not.toContain(unused);
});
