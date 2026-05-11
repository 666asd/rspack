"use strict";

module.exports = {
	moduleScope(scope, options, env) {
		if (env === "node" || !scope.window?.document) return;

		const link = scope.window.document.createElement("link");
		link.rel = "stylesheet";
		link.href = "bundle0.css";
		scope.window.document.head.appendChild(link);
	}
};
