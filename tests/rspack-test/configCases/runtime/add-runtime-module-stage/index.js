it("should inject trigger runtime module after normal runtime module", async function () {
  expect(__rspack_require.mockNormal).toBe("normal");
  expect(__rspack_require.mockTrigger).toBe("trigger");
  const fs = require("fs");
  const content = fs.readFileSync(__filename, 'utf-8');
  const triggerIndex = content.indexOf(`__rspack_require.mockTrigger = "trigger"`);
  const normalIndex = content.indexOf(`__rspack_require.mockNormal = "normal"`);
  expect(normalIndex).toBeLessThan(triggerIndex);
});