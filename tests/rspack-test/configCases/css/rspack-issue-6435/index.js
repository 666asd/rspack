import * as classes from "./style.module.css";
import legacyClasses from "./legacy/index.css";

it("should have consistent hash", () => {
	expect(classes["container-main"]).toBe(`${/* md4("./style.module.css") */ "LMAE6z"}-container-main`)
	expect(legacyClasses["legacy-main"]).toBe(`${/* md4("./legacy/index.css") */ "T0xX55"}-legacy-main`)
});
