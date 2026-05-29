module.exports = {
	async check(stats) {
		const info = stats.toJson({ all: false, assets: true });
		const jsAssets = info.assets
			.map(asset => asset.name)
			.filter(name => name.endsWith(".js"));
		expect(jsAssets.length).toBeGreaterThan(1);
		for (const asset of jsAssets) {
			expect(asset).not.toContain("[contenthash]");
		}
	}
};
