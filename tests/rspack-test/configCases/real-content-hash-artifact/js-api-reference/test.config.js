module.exports = {
	async check(stats) {
		const info = stats.toJson({ all: false, assets: true });
		const referencedAsset = info.assets.find(asset =>
			/^referenced\.[a-f0-9]{8}\.js$/.test(asset.name)
		);
		expect(referencedAsset).toBeTruthy();

		const [, hash] = /^referenced\.([a-f0-9]{8})\.js$/.exec(
			referencedAsset.name
		);
		const manifestSource = stats.compilation
			.getAsset("manifest.json")
			.source.source()
			.toString();
		const manifest = JSON.parse(manifestSource);

		expect(manifest.prefix).toBe("π:");
		expect(manifest.file).toBe(referencedAsset.name);
		expect(manifest.hash).toBe(hash);
		expect(manifestSource).toBe(JSON.stringify(manifest));
	}
};
