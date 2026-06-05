import { bar, unused } from "./shared";

export function foo() {
	return bar();
}

export function unusedFoo() {
	return unused();
}
