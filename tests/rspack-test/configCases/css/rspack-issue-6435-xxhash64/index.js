import * as classes from "./style.module.css";
import legacyClasses from "./legacy/index.css";

it("should have consistent hash", () => {
	expect(classes["container-main"]).toBe(`${/* xxhash64("./style.module.css") */ "niEgo5"}-container-main`)
	expect(legacyClasses["legacy-main"]).toBe(`${/* xxhash64("./legacy/index.css") */ "wa3b7a"}-legacy-main`)
});
