const prod = process.env.NODE_ENV === "production";

it("should allow to create css modules", async () => {
	prod
		? __non_webpack_require__("./407.bundle1.js")
		: __non_webpack_require__("./use-style_js.bundle0.js");
	const { default: x } = await import("./use-style.js");
	expect(x).toMatchSnapshot(prod ? "prod" : "dev");
});

it("should allow to process tailwind as global css", async () => {
	prod
		? __non_webpack_require__("./163.bundle1.js")
	 	: __non_webpack_require__("./tailwind_min_css.bundle0.js");
	await import("./tailwind.min.css");
});
