import "./index.css";

it("should work", async () => {
	const links = window.document.getElementsByTagName("link");
	expect(links.length).toBe(1);
	await NEXT_HMR();
});

module.hot.accept();
---
it("should work", () => {
	const links = window.document.getElementsByTagName("link");
	expect(links.length).toBe(0);
});
