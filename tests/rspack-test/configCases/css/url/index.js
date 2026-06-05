import "./style.css";

it(`should work with URLs in CSS`, () => {
	const links = document.getElementsByTagName("link");
	const css = [];

	// Skip first because import it by default
	for (const link of links.slice(1)) {
		css.push(link.sheet.css);
	}

	expect(css.length).toBeGreaterThan(0);
	const content = css.join("\n");
	expect(content).toContain("url(");
	expect(content).toContain("img.");
	expect(content).toContain("font.");
	expect(content).toContain("data:image/svg+xml");
	expect(content).toContain("https://raw.githubusercontent.com/webpack/media/master/logo/icon.png");
	expect(content).toContain("//raw.githubusercontent.com/webpack/media/master/logo/icon.png");
});
