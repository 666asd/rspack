import "./index.css";

it("should work with HMR for chained @imports", async () => {
	const links = window.document.getElementsByTagName("link");
	expect(getLinkSheet(links[0])).toContain("border: 1px solid red;");
	expect(getLinkSheet(links[0])).toContain("background: red;");

	await NEXT_HMR();

	const updatedLinks = window.document.getElementsByTagName("link");
	expect(getLinkSheet(updatedLinks[0])).toContain("border: 1px solid blue;");
	expect(getLinkSheet(updatedLinks[0])).toContain("background: green;");
});

module.hot.accept();
