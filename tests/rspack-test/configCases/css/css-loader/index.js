import * as basic from "./basic.module.css";
import * as styles from "./classes.module.css";
import * as styles1 from "./composes-multiple.module.css";
import * as styles3 from "./composes-global.module.css";
import * as styles4 from "./scope-at-rule.module.css";
import * as styles5 from "./nesting.module.css";
import * as styles6 from "./prefer-relative.module.css";
import * as styles7 from "./animation-name.module.css";
import * as styles8 from "./at-sign-in-package-name.module.css";
import * as styles9 from "./resolving-from-node_modules.module.css";
import * as styles10 from "./local-Ident-name.module.css";
import * as styles11 from "./local-Ident-name.module.css?local-ident-name-1";
import * as styles12 from "./local-Ident-name.module.css?local-ident-name-2";
import * as styles13 from "./local-Ident-name.module.css?local-ident-name-3";
import * as styles14 from "./local-Ident-name.module.css?local-ident-name-4";
import * as styles15 from "./local-Ident-name.module.css?local-ident-name-5";
import * as styles16 from "./local-Ident-name.module.css?local-ident-name-6";
import * as styles17 from "./local-Ident-name.module.css?local-ident-name-7";
import * as styles18 from "./local-Ident-name.module.css?local-ident-name-8";
import * as styles19 from "./local-Ident-name.module.css?local-ident-name-9";
import * as stylesHash10 from "./local-Ident-name.module.css?local-ident-name-10";
import * as stylesHash11 from "./local-Ident-name.module.css?local-ident-name-11";
import * as stylesHash12 from "./local-Ident-name.module.css?local-ident-name-12";
import * as stylesHash13 from "./local-Ident-name.module.css?local-ident-name-13";
import * as stylesHash14 from "./local-Ident-name.module.css?local-ident-name-14";
import * as styles20 from "./order.module.css";
import * as styles21 from "./dedup.module.css";
import * as styles22 from "./composes-from-less.module.css";
import * as styles23 from "./tilde.module.css";
import * as styles24 from "./icss.module.css";
import * as styles25 from "./empty.module.css";
import * as styles26 from "./component-name.module.css";
import * as styles27 from "./composes-chain.module.css";
import * as styles28 from "./file.with.many.dots.in.name.module.css";
import * as styles29 from "./composes-duplicate.module.css";
import * as styles30 from "./keyframes-leak-scope.module.css";
import * as styles31 from "./path-placeholder.module.css";
import * as styles32 from "./at-value-extra.module.css";

const EXPORT_TYPE = process.env.EXPORT_TYPE;

// Read `default` via Reflect.get so HarmonyImportSpecifier analysis does not
// flag a missing default export warning for exportTypes without one.
const DEFAULT_KEY = "default";
const getDefault = ns => Reflect.get(ns, DEFAULT_KEY);
const snapshotFile = name => `${__SNAPSHOT__}/${EXPORT_TYPE}-${name}.txt`;
const expectSnapshot = (name, value) =>
	expect(value).toMatchFileSnapshotSync(snapshotFile(name));

const classes = ns => {
	const out = {};
	for (const key of Object.keys(ns)) {
		if (key === "default") continue;
		out[key] = ns[key];
	}
	return out;
};

it(`should export CSS module class names (${EXPORT_TYPE})`, () => {
	expectSnapshot("basic", classes(basic));
	expectSnapshot("classes", classes(styles));
	expectSnapshot("composes-multiple", classes(styles1));
	expectSnapshot("composes-global", classes(styles3));
	expectSnapshot("scope-at-rule", classes(styles4));
	expectSnapshot("nesting", classes(styles5));
	expectSnapshot("prefer-relative", classes(styles6));
	expectSnapshot("animation-name", classes(styles7));
	expectSnapshot("at-sign-in-package-name", classes(styles8));
	expectSnapshot("resolving-from-node-modules", classes(styles9));
	expectSnapshot("local-ident-name", classes(styles10));
	expectSnapshot("local-ident-name-1", classes(styles11));
	expectSnapshot("local-ident-name-2", classes(styles12));
	expectSnapshot("local-ident-name-3", classes(styles13));
	expectSnapshot("local-ident-name-4", classes(styles14));
	expectSnapshot("local-ident-name-5", classes(styles15));
	expectSnapshot("local-ident-name-6", classes(styles16));
	expectSnapshot("local-ident-name-7", classes(styles17));
	expectSnapshot("local-ident-name-8", classes(styles18));
	expectSnapshot("local-ident-name-9", classes(styles19));
	expectSnapshot("local-ident-name-10", classes(stylesHash10));
	expectSnapshot("local-ident-name-11", classes(stylesHash11));
	expectSnapshot("local-ident-name-12", classes(stylesHash12));
	expectSnapshot("local-ident-name-13", classes(stylesHash13));
	expectSnapshot("local-ident-name-14", classes(stylesHash14));
	expectSnapshot("order", classes(styles20));
	expectSnapshot("dedup", classes(styles21));
	expectSnapshot("composes-from-less", classes(styles22));
	expectSnapshot("tilde", classes(styles23));
	expectSnapshot("icss", classes(styles24));
	expectSnapshot("empty", classes(styles25));
	expectSnapshot("component-name", classes(styles26));
	expectSnapshot("composes-chain", classes(styles27));
	expectSnapshot("many-dots", classes(styles28));
	expectSnapshot("composes-duplicate", classes(styles29));
	expectSnapshot("keyframes-leak-scope", classes(styles30));
	expectSnapshot("path-placeholder", classes(styles31));
	expectSnapshot("at-value-extra", classes(styles32));
});

if (EXPORT_TYPE === "link") {
	it("should load extracted CSS chunk via <link> tag (link)", () => {
		const links = Array.from(document.getElementsByTagName("link"));
		const css = [];

		for (const link of links.slice(1)) {
			css.push(link.sheet.css);
		}

		expectSnapshot("links", css);
	});

	it("should not provide a `default` export for the link exportType", () => {
		expect(Object.keys(basic).includes("default")).toBe(false);
	});

	it("should expose a class literally named `default` (link)", () => {
		expect(typeof styles.default).toBe("string");
		expect(styles.default).toContain("default");
	});
}

if (EXPORT_TYPE === "text") {
	it("should export CSS text as the `default` export (text)", () => {
		const basicDefault = getDefault(basic);
		expect(typeof basicDefault).toBe("string");
		expect(basicDefault).toContain("basic_module_css-a");
	});

	it("should not produce a separate CSS chunk for the text exportType", () => {
		const links = document.getElementsByTagName("link");
		expect(links.length).toBeLessThanOrEqual(1);
	});
}

if (EXPORT_TYPE === "css-style-sheet") {
	it("should export a CSSStyleSheet as the `default` export (css-style-sheet)", () => {
		const basicDefault = getDefault(basic);
		expect(basicDefault).toBeInstanceOf(CSSStyleSheet);
		expect(basicDefault.cssRules.length).toBeGreaterThan(0);
	});

	it("should not produce a separate CSS chunk for the css-style-sheet exportType", () => {
		const links = document.getElementsByTagName("link");
		expect(links.length).toBeLessThanOrEqual(1);
	});
}

if (EXPORT_TYPE === "style") {
	it("should inject CSS via <style> tags (style)", () => {
		const styleTags = document.getElementsByTagName("style");
		expect(styleTags.length).toBeGreaterThan(0);

		const allCSS = Array.from(styleTags).map(s => s.textContent);
		expect(allCSS.some(c => c.includes("basic_module_css-a"))).toBe(true);
	});

	it("should not provide a `default` export for the style exportType", () => {
		expect(Object.keys(basic).includes("default")).toBe(false);
	});

	it("should expose a class literally named `default` (style)", () => {
		expect(typeof styles.default).toBe("string");
		expect(styles.default).toContain("default");
	});

	it("should not produce a separate CSS chunk for the style exportType", () => {
		const links = document.getElementsByTagName("link");
		expect(links.length).toBeLessThanOrEqual(1);
	});
}
