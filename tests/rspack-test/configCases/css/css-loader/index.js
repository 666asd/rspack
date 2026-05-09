import * as basic from "./basic.module.css";
import * as styles from "./classes.module.css";
import * as styles1 from "./composes-multiple.module.css";
import * as styles3 from "./composes-global.module.css";
import * as styles4 from "./scope-at-rule.module.css";
import * as styles5 from "./nesting.module.css";
import * as styles6 from "./prefer-relative.module.css";
import * as styles7 from "./animation-name.module.css";
import * as styles8 from "./at-sign-in-package-name.module.css";
import * as styles9 from "./resolving-from-node_modules.module.css";
import * as styles10 from "./local-Ident-name.module.css";
import * as styles11 from "./local-Ident-name.module.css?local-ident-name-1";
import * as styles12 from "./local-Ident-name.module.css?local-ident-name-2";
import * as styles13 from "./local-Ident-name.module.css?local-ident-name-3";
import * as styles14 from "./local-Ident-name.module.css?local-ident-name-4";
import * as styles15 from "./local-Ident-name.module.css?local-ident-name-5";
import * as styles16 from "./local-Ident-name.module.css?local-ident-name-6";
import * as styles17 from "./local-Ident-name.module.css?local-ident-name-7";
import * as styles18 from "./local-Ident-name.module.css?local-ident-name-8";
import * as styles19 from "./local-Ident-name.module.css?local-ident-name-9";
import * as styles20 from "./order.module.css";
import * as styles21 from "./dedup.module.css";
import * as styles22 from "./composes-from-less.module.css";
import * as styles23 from "./tilde.module.css";
import * as styles24 from "./icss.module.css";

it("should work", () => {
	const links = Array.from(document.getElementsByTagName("link"));
	const css = [];

	// Skip first because import it by default
	for (const link of links.slice(1)) {
		css.push(getLinkSheet(link));
	}

	const snapshots = [
		["basic", basic],
		["classes", styles],
		["composes-multiple", styles1],
		["composes-global", styles3],
		["scope-at-rule", styles4],
		["nesting", styles5],
		["prefer-relative", styles6],
		["animation-name", styles7],
		["at-sign-in-package-name", styles8],
		["resolving-from-node-modules", styles9],
		["local-ident-name", styles10],
		["local-ident-name-1", styles11],
		["local-ident-name-2", styles12],
		["local-ident-name-3", styles13],
		["local-ident-name-4", styles14],
		["local-ident-name-5", styles15],
		["local-ident-name-6", styles16],
		["local-ident-name-7", styles17],
		["local-ident-name-8", styles18],
		["local-ident-name-9", styles19],
		["order", styles20],
		["dedup", styles21],
		["composes-from-less", styles22],
		["tilde", styles23],
		["icss", styles24],
		["css", css],
	];

	for (const [name, value] of snapshots) {
		expect(value).toMatchFileSnapshotSync(`${__SNAPSHOT__}/${name}.txt`);
	}
});
