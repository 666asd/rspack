import "./index.css";

const findOurLink = () => {
	const link = [...window.document.getElementsByTagName("link")].find(
		item =>
			item.rel === "stylesheet" &&
			item.href &&
			item.href.includes("bundle.css")
	);
	expect(link).toBeDefined();
	return link;
};

it("should not touch non-stylesheet, data:, or anchor links during CSS HMR", async () => {
	const head = window.document.head;
	expect(getLinkSheet(findOurLink())).toContain("color: red;");

	const iconWithDataUrl = window.document.createElement("link");
	iconWithDataUrl.rel = "shortcut icon";
	iconWithDataUrl.href = "data:;base64,=";
	head.appendChild(iconWithDataUrl);

	const iconWithAnchor = window.document.createElement("link");
	iconWithAnchor.rel = "shortcut icon";
	iconWithAnchor.href = "#href";
	head.appendChild(iconWithAnchor);

	const iconWithoutHref = window.document.createElement("link");
	iconWithoutHref.rel = "shortcut icon";
	head.appendChild(iconWithoutHref);

	await NEXT_HMR();
	
	expect(getLinkSheet(findOurLink())).toContain("color: blue;");
	expect(iconWithDataUrl.parentNode).toBe(head);
	expect(iconWithAnchor.parentNode).toBe(head);
	expect(iconWithoutHref.parentNode).toBe(head);

	await NEXT_HMR();

	expect(getLinkSheet(findOurLink())).toContain("color: yellow;");
	expect(iconWithDataUrl.parentNode).toBe(head);
	expect(iconWithAnchor.parentNode).toBe(head);
	expect(iconWithoutHref.parentNode).toBe(head);
});

module.hot.accept();
