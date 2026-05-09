it("should compile and load style on demand", async () => {
	const x = await import("./style.css");
  expect(x).toEqual(nsObj({}));
	const style = getComputedStyle(document.body);
	expect(style.getPropertyValue("background")).toBe("red");
	expect(style.getPropertyValue("margin")).toBe("10px");
});
