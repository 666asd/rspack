module.exports = {
  mode: 'none',
  entry: { main: './index.js', test: './test.js' },
  output: {
    module: true,
    library: {
      type: 'modern-module',
    },
    filename: '[name].js',
    chunkFormat: 'module',
  },
  resolve: {
    extensions: ['.js'],
  },
  externalsType: 'module',
  externals: [
    'externals0',
    'externals1',
    'externals2',
    'externals3',
    'externals4',
  ],
  optimization: {
    concatenateModules: true,
    usedExports: true,
  },
  plugins: [
    function () {
      const handler = (compilation) => {
        compilation.hooks.processAssets.tap('testcase', (assets) => {
          const source = assets['test.js'].source();
          expect(source).toMatchInlineSnapshot(`
            import { HomeLayout, a, HomeLayout as aaa } from "externals0";
            import { a, a as a_2 } from "externals1";
            import externals2 from "externals2";
            import * as _rspack_external_externals3 from "externals3";
            import * as namespace from "externals3";
            import "externals4";

            const defaultValue = externals2;








            (function Layout(props) {
              const { HomeLayout = aaa } = props;
              call({ HomeLayout });
            })()

            // re export


            // named import
            ;

            // default import


            // namespace import


            // side effect only import




            a_2;
            defaultValue;
            namespace;

            export { a };
          `);
        });
      };
      this.hooks.compilation.tap('testcase', handler);
    },
  ],
};
