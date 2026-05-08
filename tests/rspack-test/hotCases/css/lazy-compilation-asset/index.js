import "./style.css";

it("should make asset available in both CSS and lazy JS chunk", async () => {
	const promise = import("./mod.js");
	await new Promise(resolve => setTimeout(resolve, 1000));
	await NEXT_HMR();
	const mod = await promise;
	expect(mod.default).toContain("file.txt");
});

module.hot.accept();
