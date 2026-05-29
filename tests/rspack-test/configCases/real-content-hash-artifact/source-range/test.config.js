module.exports = {
	async check(stats) {
		const info = stats.toJson({ all: false, assets: true });
		const jsAssets = info.assets
			.map(asset => asset.name)
			.filter(name => name.endsWith(".js"));
		expect(jsAssets.length).toBeGreaterThan(1);
		for (const asset of jsAssets) {
			expect(asset).not.toContain("[contenthash]");
			expect(asset).toMatch(/\.[a-f0-9]{8}\.js$/);
		}
		const emittedAssets = stats.compilation.getAssets();
		const runtimeSource = emittedAssets
			.filter(asset => asset.name.endsWith(".js"))
			.map(asset => asset.source.source().toString())
			.find(source => source.includes("mini-css") || source.includes("chunkId"));
		expect(runtimeSource).toBeTruthy();
		expect(runtimeSource).not.toContain("__RSPACK_REAL_CONTENT_HASH_START_");
		for (const asset of jsAssets.filter(name => !name.startsWith("main."))) {
			expect(runtimeSource).toContain(asset);
		}
	}
};
