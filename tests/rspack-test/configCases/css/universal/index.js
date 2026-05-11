import * as pureStyle from "./style.css";
import * as styles from "./style.modules.css";

it("should work", async () => {
  expect(pureStyle).toEqual(nsObj({}));

  if (typeof document !== "undefined") {
    const style = getComputedStyle(document.body);
		expect(style.getPropertyValue("background")).toBe("red");
  }

  expect(styles.foo).toBe("style_modules_css-foo");

  const x = await import(/* webpackPrefetch: true */ "./style2.css");
  expect(x).toEqual(nsObj({}));

  if (typeof document !== "undefined") {
    const style = getComputedStyle(document.body);
		expect(style.getPropertyValue("color")).toBe("rgb(0, 0, 255)");
  }

  const y = await import(/* webpackPrefetch: true */ "./style2.modules.css");
  expect(y.bar).toBe("style2_modules_css-bar");
});

it("should work in web worker", async () => {
  if (typeof window !== "undefined") {
    const worker = new Worker(new URL("./worker.js", import.meta.url), {
      type: "module"
    });
    worker.postMessage("ok");
    const result = await new Promise((resolve) => {
      worker.onmessage = (event) => {
        resolve(event.data);
      };
    });
    expect(result).toBe(
      "data: style_modules_css-foo style2_modules_css-bar style3_modules_css-baz, thanks"
    );
    await worker.terminate();
  } else {
    expect(true).toBe(true);
  }
});
