const prod = process.env.NODE_ENV === "production";

it("should allow to create css modules", async () => {
  const { default: x } = await import("./use-style.js");
	expect(x.global).toBe("global");
	expect(x.class).toBeTruthy();
	expect(x.local).toContain("style_module_css-local1");
	expect(x.local2).toContain("style_module_css-local5");
	expect(x.ident).toBe("ident");
	expect(x.keyframes).toBeTruthy();
	expect(x.animation).toBeTruthy();
	expect(x.vars).toContain("style_module_css-local-color");
	expect(x.media).toBeTruthy();
	expect(x.supports).toBeTruthy();
	expect(x.cssModuleWithCustomFileExtension).toBeTruthy();
	expect(x.notAValidCssModuleExtension).toBe(true);
	expect(x.UsedClassName).toBeTruthy();
	expect(x.exportLocalVarsShouldCleanup).toBe("false false");

	const fs = __non_webpack_require__("fs");
	const path = __non_webpack_require__("path");
	const cssOutputFilename = prod ? "142.bundle1.css" : "use-style_js.bundle0.css";

	const cssContent = fs.readFileSync(
		path.join(__dirname, cssOutputFilename),
		"utf-8"
	);
	expect(cssContent).not.toContain(".my-app--");
	expect(cssContent).toContain("color: red");
	expect(cssContent).toContain("color: green");
	expect(cssContent).toContain("@media screen and (max-width: 600px)");
	expect(cssContent).toContain("@supports (display: grid)");
	expect(cssContent).toContain("animation");
});
