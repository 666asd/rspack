const getter = __webpack_require__.d;
__webpack_require__.d({}, {});
__webpack_require__.d = function () {};
const boundGetter = __webpack_require__.d.bind(null);
const nestedGetter = __webpack_require__.d.foo;
__webpack_require__.d.foo = 1;
export { boundGetter, getter, nestedGetter };
