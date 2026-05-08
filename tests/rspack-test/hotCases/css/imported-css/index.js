import "./index.css";

it("should work", async () => {
	const links = window.document.getElementsByTagName("link");
	expect(getLinkSheet(links[0])).toContain("color: green;");

	await NEXT_HMR();
	expect(getLinkSheet(window.document.getElementsByTagName("link")[0])).toContain("color: blue;");

	await NEXT_HMR();
	expect(getLinkSheet(window.document.getElementsByTagName("link")[0])).toContain("color: yellow;");
});

module.hot.accept();
