module.exports = {
  findBundle() {
    return [];
  },
  async check(_, stats) {
    const info = stats.toJson({ all: false, assets: true, errors: true });
    expect(info.errors).toHaveLength(0);
    const assetNames = info.assets.map((asset) => asset.name);
    expect(assetNames).not.toContain('untracked.aaaa.js');
    expect(
      assetNames.some((name) => /^untracked\.[a-z0-9]{4}\.js$/.test(name)),
    ).toBe(true);
  },
};
