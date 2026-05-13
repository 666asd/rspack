module.exports = {
  snapshotContent(content) {
    expect(content).toMatch(/import \{ readFile( as \w+)? \} from "fs";/);
    expect(content).not.toContain('from "virtual-fs"');
    expect(content).not.toContain('__webpack_require__(/*! virtual-fs */');
    return content;
  },
};
