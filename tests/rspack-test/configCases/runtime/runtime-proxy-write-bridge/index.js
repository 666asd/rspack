export const value = 1;

import {
	requireWriteSyncedToProxy,
	proxyWriteSyncedToRequire,
	proxyWriteUpdatedNonce
} from "./writer";
import "./shadow";

it("should sync runtime proxy write bridge", () => {
	expect(requireWriteSyncedToProxy).toBe(true);
	expect(proxyWriteSyncedToRequire).toBe(true);
	expect(proxyWriteUpdatedNonce).toBe(true);
});
