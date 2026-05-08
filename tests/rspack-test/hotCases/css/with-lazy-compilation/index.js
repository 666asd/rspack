const getFile = name =>
	__non_webpack_require__("fs").readFileSync(
		__non_webpack_require__("path").join(__dirname, name),
		"utf-8"
	);

it("should work", async () => {
	const promise = import("./style.css");
	await new Promise(resolve => setTimeout(resolve, 1000));
	await NEXT_HMR();
	await promise;

	let href = window.document.getElementsByTagName("link")[0].href;
	expect(href).toBe("https://test.cases/path/style_css.css");
	href = href
		.replace(/^https:\/\/test\.cases\/path\//, "")
		.replace(/^https:\/\/example\.com\//, "");
	expect(getFile(href)).toContain("color: red;");

	await NEXT_HMR();

	href = window.document.getElementsByTagName("link")[0].href;
	expect(href).toContain("https://test.cases/path/style_css.css?hmr");
	href = href
		.replace(/^https:\/\/test\.cases\/path\//, "")
		.replace(/^https:\/\/example\.com\//, "")
		.split("?")[0];
	expect(getFile(href)).toContain("color: blue;");
});

module.hot.accept();
