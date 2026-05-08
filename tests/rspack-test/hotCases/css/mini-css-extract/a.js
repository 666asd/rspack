import "./a.css";

const getFile = name =>
	__non_webpack_require__("fs").readFileSync(
		__non_webpack_require__("path").join(__dirname, name),
		"utf-8"
	);

it("should work", async () => {
	expect(getFile("main.css")).toContain("color: red;");
	await NEXT_HMR();
	expect(getFile("main.css")).toContain("color: blue;");
});

module.hot.accept();
