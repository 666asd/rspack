import sheet from "./basic.css" with { type: "css" };
import sheetAssert from "./basic.css" assert { type: "css" };

it("should import CSS as CSSStyleSheet with 'with' syntax", () => {
	expect(sheet).toBeInstanceOf(CSSStyleSheet);
	expect(sheet._cssText).toContain(".test");
	expect(sheet._cssText).toContain("color: red");
	expect(sheet._cssText).toContain("background: blue");
});

it("should import CSS as CSSStyleSheet with 'assert' syntax", () => {
	expect(sheetAssert).toBeInstanceOf(CSSStyleSheet);
	expect(sheetAssert._cssText).toContain(".test");
});

it("should be able to adopt the stylesheet", () => {
	expect(sheet).toBeInstanceOf(CSSStyleSheet);
});

it("should handle @import in CSSStyleSheet", () => {
	expect(sheet._cssText).toContain(".imported");
	expect(sheet._cssText).toContain("color: green");
	expect(sheet._cssText).toContain("font-style: italic");
});

it("should handle url() for images in CSSStyleSheet", () => {
	expect(sheet._cssText).toContain(".with-image");
	expect(sheet._cssText).toContain("background-image: url(");
	expect(sheet._cssText).toContain("width: 100px");
	expect(sheet._cssText).toContain("height: 100px");
});

it("should handle CSS nesting in CSSStyleSheet", () => {
	expect(sheet._cssText).toContain("& .child");
	expect(sheet._cssText).toContain("color: green");
});
