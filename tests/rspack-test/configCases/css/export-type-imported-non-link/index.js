import "./link-parent.css";
import "./link-parent-with-conditions.css";
import "./link-parent-with-same-child-conditions.css";
import { "style-parent-class" as styleParentClass } from "./style-parent.css";
import textChild from "./text-child.css";
import sheetChild from "./sheet-child.css";
import "./style-child.css";

it("should keep JS exports for non-link children imported by link css", () => {
	expect(textChild).toContain(".text-child");
	expect(textChild).not.toContain("@charset");
	expect(sheetChild).toBeInstanceOf(CSSStyleSheet);
});

it("should inject a text child imported by a style parent with nested indentation", () => {
	expect(styleParentClass).toBe("style-parent_css-style-parent-class");

	const allCSS = Array.from(document.getElementsByTagName("style")).map(
		style => style.textContent
	);
	const injected = allCSS.find(css => css.includes(".text-child"));

	expect(injected).toContain("@media screen{");
	expect(injected).not.toContain("@charset");
	expect(injected).toContain("  @supports (display: grid) {");
	expect(injected).toContain("    @layer theme {");
	expect(injected).toContain("      .text-child {");
	expect(injected).toContain("      \tcolor: blue;");
});
