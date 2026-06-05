import { foo } from "./lib";

it("should collect rsdoctor export usage graph", () => {
	expect(foo()).toBe(42);
});
