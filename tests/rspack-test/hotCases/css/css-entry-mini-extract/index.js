const getFile = name =>
	__non_webpack_require__("fs").readFileSync(
		__non_webpack_require__("path").join(__dirname, name),
		"utf-8"
	);

it("should work", async () => {
	expect(getFile("css-entry.css")).toContain("color: red;");

	await NEXT_HMR();
	expect(getFile("css-entry.css")).toContain("color: blue;");

	await NEXT_HMR();
	expect(getFile("css-entry.css")).toContain("color: green;");
});

module.hot.accept();
