import "./style.css";

it("should compile", () => {
	const links = document.getElementsByTagName("link");
	const css = links[1].sheet.css;

	expect(css).toContain("@import url(./basic.css)");
	expect(css).toContain(".class");
	expect(css).toContain("color: red");
	expect(css).toContain("Roboto");
	expect(css).toContain("image-set(");
	expect(css).toContain("url(");
});
