import foo from './src/foo';

rs.mock('./src/foo', () => 42);

it('should support primitive mock factory values', () => {
  expect(foo).toBe(42);
});
