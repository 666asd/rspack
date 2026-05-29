module.exports = {
	findBundle() {
		return [];
	},
	async check(_, stats) {
		const info = stats.toJson({ all: false, errors: true });
		expect(
			info.errors.some(error =>
				String(error.message || error).includes(
					"MissingRealContentHashRecord"
				)
			)
		).toBe(true);
	}
};
