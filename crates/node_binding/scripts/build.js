const path = require("node:path");
const { tmpdir } = require("node:os");
const {
	existsSync,
	chmodSync,
	mkdtempSync,
	readFileSync,
	renameSync,
	symlinkSync,
	writeFileSync
} = require("node:fs");
const { values, positionals } = require("node:util").parseArgs({
	args: process.argv.slice(2),
	options: {
		profile: {
			type: "string"
		},
	},
	strict: true,
	allowPositionals: true
});

const { spawn, spawnSync } = require("node:child_process");

const NAPI_BINDING_DTS = "napi-binding.d.ts"
const SAFE_ICF_PROFILES = new Set(["release", "ci"]);
const CARGO_SAFELY_EXIT_CODE = 0;

const watch = process.argv.includes("--watch");

build().then((value) => {
	// Regarding cargo's non-zero exit code as an error.
	if (value !== CARGO_SAFELY_EXIT_CODE) {
		process.exit(value);
	}
}).catch(err => {
	console.error(err);
	process.exit(1);
});

function addSafeIcfRustflags(rustflags, profile, target) {
	if (!SAFE_ICF_PROFILES.has(profile)) {
		return;
	}

	const normalizedTarget = target || hostRustTarget();

	if (normalizedTarget.includes("wasm")) {
		return;
	}

	if (normalizedTarget.includes("windows-msvc")) {
		// MSVC's /OPT:ICF folds identical COMDAT functions.
		rustflags.push("-Clink-arg=/OPT:ICF");
		return;
	}

	if (normalizedTarget === "aarch64-apple-darwin") {
		// --icf=safe for Mach-O requires lld; keep x64 macOS on the system linker.
		// Some native dependencies still pass clang-driver-style -Wl arguments, so
		// route rust-lld through a small wrapper that translates them for ld64.lld.
		rustflags.push(
			"-Zunstable-options",
			`-Clinker=${darwinRustLldWrapperPath()}`,
			"-Clinker-flavor=darwin-lld",
			"-Clink-arg=--icf=safe"
		);
		return;
	}

	if (normalizedTarget.includes("linux")) {
		if (!process.env.USE_ZIG) {
			// Some cross GCC wrappers used by napi-cross do not support -fuse-ld=lld.
			// Use GCC's -B search prefix so the driver selects an ICF-capable linker as `ld`.
			rustflags.push(`-Clink-arg=-B${linuxIcfLinkerSearchPath(normalizedTarget)}`);
		}

		rustflags.push("-Clink-arg=-Wl,--icf=safe");
	}
}

function darwinRustLldWrapperPath() {
	const linkerPath = rustLldPath();
	if (!linkerPath) {
		throw new Error("safe ICF on macOS arm64 requires rust-lld");
	}

	console.log(`Using ${linkerPath} for safe ICF`);
	const linkerDir = mkdtempSync(path.join(tmpdir(), "rspack-icf-linker-"));
	const wrapperPath = path.join(linkerDir, "rust-lld-darwin");
	writeFileSync(
		wrapperPath,
		`#!/usr/bin/env bash
set -euo pipefail

args=()
for arg in "$@"; do
\tcase "$arg" in
\t\t-Wl,*) IFS=',' read -ra parts <<< "\${arg#-Wl,}"; args+=("\${parts[@]}") ;;
\t\t-Wl|-Xlinker) ;;
\t\t*) args+=("$arg") ;;
\tesac
done

exec ${shellQuote(linkerPath)} "\${args[@]}"
`
	);
	chmodSync(wrapperPath, 0o755);
	return wrapperPath;
}

function linuxIcfLinkerSearchPath(target) {
	const linkerPath = linuxIcfLinkerPath(target);
	if (!linkerPath) {
		throw new Error("safe ICF on Linux requires ld.gold, ld.lld, or rust-lld");
	}

	console.log(`Using ${linkerPath} for safe ICF`);
	const linkerDir = mkdtempSync(path.join(tmpdir(), "rspack-icf-linker-"));
	symlinkSync(linkerPath, path.join(linkerDir, "ld"));
	return `${linkerDir}${path.sep}`;
}

function linuxIcfLinkerPath(target) {
	if (target === "x86_64-unknown-linux-gnu") {
		const goldPath = commandPath("ld.gold");
		if (goldPath) {
			return goldPath;
		}
	}

	return commandPath("ld.lld") || rustLldPath();
}

function rustLldPath() {
	const sysroot = commandOutput("rustc", ["--print", "sysroot"]);
	const version = commandOutput("rustc", ["-vV"]);
	const host = version?.match(/^host: (.+)$/m)?.[1];
	if (!sysroot || !host) {
		return null;
	}

	const lldPath = path.join(sysroot, "lib", "rustlib", host, "bin", "rust-lld");
	return existsSync(lldPath) ? lldPath : null;
}

function commandPath(command) {
	return commandOutput("which", [command])?.split(/\r?\n/)[0] || null;
}

function commandOutput(command, args) {
	const result = spawnSync(command, args, { encoding: "utf8" });
	return result.status === 0 ? result.stdout.trim() : null;
}

function shellQuote(value) {
	return `'${value.replaceAll("'", "'\\''")}'`;
}

function hostRustTarget() {
	const arch = {
		arm64: "aarch64",
		ia32: "i686",
		x64: "x86_64"
	}[process.arch] || process.arch;

	if (process.platform === "darwin") {
		return `${arch}-apple-darwin`;
	}
	if (process.platform === "win32") {
		return `${arch}-pc-windows-msvc`;
	}
	if (process.platform === "freebsd") {
		return `${arch}-unknown-freebsd`;
	}
	return `${arch}-unknown-linux-gnu`;
}

async function build() {
	return new Promise((resolve, reject) => {
		const args = [
			"build",
			"--platform",
			"--dts",
			NAPI_BINDING_DTS,
			"--no-js",
			// "--no-const-enum",
			"--no-dts-header",
			"--pipe",
			`"node ${path.resolve(__dirname, "dts-header.js")}"`
		];
		const rustflags = []
		const features = [];
		const envs = { ...process.env };
		const use_build_std = values.profile === "release"
			|| values.profile === "release-debug"
			|| values.profile === "release-wasi"
			|| values.profile === "profiling";

		if (values.profile) {
			args.push("--profile", values.profile);
		}
		if (watch) {
			args.push("--watch");
		}
		if (process.env.USE_NAPI_CROSS) {
			args.push("--use-napi-cross");
		}
		if (process.env.USE_ZIG) {
			args.push("--cross-compile");
		}
		if (process.env.RUST_TARGET) {
			args.push("--target", process.env.RUST_TARGET);
		}
		if (!process.env.DISABLE_PLUGIN) {
			args.push("--no-default-features");
			features.push("plugin");
		}
		if (process.env.RSPACK_TARGET_BROWSER) {
			features.push("browser")
		}
		if (values.profile !== "release") {
			features.push("perfetto");
		}
		args.push("--no-dts-cache");
		if (process.env.SFTRACE) {
			features.push("sftrace-setup");
			rustflags.push("-Zinstrument-xray=always");
		}
		if (process.env.ALLOCATIVE) {
			features.push("allocative");
			rustflags.push("--cfg=allocative");
		}
		if (process.env.TRACY) {
			features.push("tracy-client");
		}
		addSafeIcfRustflags(rustflags, values.profile, process.env.RUST_TARGET);
		if (values.profile === "release") {
			features.push("info-level");
			rustflags.push("-Zlocation-detail=none");
			if (process.env.RUST_TARGET && !process.env.RUST_TARGET.includes("windows-msvc")) {
				rustflags.push("-Cforce-unwind-tables=no");
			}
		} else {
			// enable unwind-table for backtrace for non-release profile
			if (!process.env.RUST_TARGET || (process.env.RUST_TARGET && !process.env.RUST_TARGET.includes("windows-msvc"))) {
				rustflags.push("-Cforce-unwind-tables=yes");
			}

		}
		if (features.length) {
			args.push(`--features ${features.join(",")}`);
		}

		if (positionals.length > 0 || rustflags.length > 0 || use_build_std) {
			// napi need `--` to separate options and positional arguments.
			args.push("--");

			if (rustflags.length > 0) {
				const flag = rustflags.map(f => `\\"${f}\\"`).join(",");
				args.push("--config");
				args.push(`"target.'cfg(all())'.rustflags = [${flag}]"`)
			}

			if (use_build_std) {
				// allows to optimize std with current compile arguments
				// and avoids std code generate unwind table to save size.
				args.push("-Zbuild-std=panic_abort,std");
			}

			if (positionals.length > 0) {
				args.push(...positionals);
			}
		}

		console.log(`Run command: napi ${args.join(" ")}`);

		const cp = spawn("napi", args, {
			stdio: "inherit",
			shell: true,
			env: envs,
		});

		cp.on("error", reject);
		cp.on("exit", (code) => {
			if (code === CARGO_SAFELY_EXIT_CODE) {
				// Fix an issue where napi cli does not generate `string_enum` with `enum`s.
				const dts = path.resolve(__dirname, "..", NAPI_BINDING_DTS);
				writeFileSync(dts,
					readFileSync(dts, "utf8")
						.replaceAll("const enum", "enum")
						// Remove the NormalModule type declaration generated by N-API.
						// We manually declare the NormalModule type in banner.d.ts
						// This allows users to extend NormalModule with static methods through type augmentation.
						.replaceAll(/export\s+declare\s+class\s+NormalModule\s*\{([\s\S]*?)\}\s*(?=\n\s*(?:export|declare|class|$))/g, "")
				);

				// For browser wasm, we rename the artifacts to distinguish them from node wasm
				if (process.env.RSPACK_TARGET_BROWSER) {
					renameSync("rspack.wasm32-wasi.debug.wasm", "rspack.browser.debug.wasm")
					renameSync("rspack.wasm32-wasi.wasm", "rspack.browser.wasm")
				}

				if (process.env.TRACY) {
					// split debug symbols for tracy
					spawnSync('dsymutil', [
						path.resolve(__dirname, "..", "rspack.darwin-arm64.node")
					], {
						stdio: "inherit",
						shell: true,
					})
				}
			}
			resolve(code);
		});
	});
}
