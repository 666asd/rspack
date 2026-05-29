import { useMemo } from "./reexport";

it("should import by export require", () => {
	expect(useMemo).toBe("useMemo");
});

it("should flag other unused items with __rspack_unused_export", () => {
	const mainFile = require("fs").readFileSync(__filename, "utf-8");
	for (let i of ["useState", "useEffect"]) {
		expect(
			new RegExp(`__rspack_unused_export(?:_\\d+_\\d+__)? = "${i}"`).test(
				mainFile
			)
		).toBeTruthy();
	}
	expect(mainFile).toContain('"user binding"');
});
