import * as styles from "./style.module.css";

it("should HMR a CSS module that uses composes", async () => {
	expect(styles).toMatchObject({
		button: "style_module_css-button shared_module_css-shared"
	});

	const links = window.document.getElementsByTagName("link");
	expect(getLinkSheet(links[0])).toContain("color: red;");

	await NEXT_HMR();

	const updatedLinks = window.document.getElementsByTagName("link");
	expect(getLinkSheet(updatedLinks[0])).toContain("color: green;");
	expect(styles).toMatchObject({
		button: "style_module_css-button shared_module_css-shared"
	});
});

module.hot.accept(["./style.module.css", "./shared.module.css"]);
