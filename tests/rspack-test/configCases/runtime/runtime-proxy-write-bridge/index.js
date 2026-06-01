export const value = 1;

import {
	contextWriteSynced,
	contextWriteUpdatedNonce
} from "./writer";

it("should sync runtime proxy write bridge", () => {
	expect(contextWriteSynced).toBe(true);
	expect(contextWriteUpdatedNonce).toBe(true);
});
