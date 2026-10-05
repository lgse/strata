// SPDX-License-Identifier: MIT
import { mathjax } from "@mathjax/src/mjs/mathjax.js";
import { TeX } from "@mathjax/src/mjs/input/tex.js";
import { SVG } from "@mathjax/src/mjs/output/svg.js";
import { MathJaxTexFont } from "@mathjax/mathjax-tex-font/mjs/svg.js";
import { liteAdaptor } from "@mathjax/src/mjs/adaptors/liteAdaptor.js";
import { RegisterHTMLHandler } from "@mathjax/src/mjs/handlers/html.js";
import "@mathjax/src/mjs/input/tex/base/BaseConfiguration.js";
import "@mathjax/src/mjs/input/tex/ams/AmsConfiguration.js";

const adaptor = liteAdaptor();
RegisterHTMLHandler(adaptor);
const document = mathjax.document("", {
  InputJax: new TeX({
    packages: ["base", "ams"],
    maxBuffer: 4096,
    maxMacros: 256,
    formatError: (_jax, error) => { throw error; },
  }),
  OutputJax: new SVG({ fontCache: "none", fontData: MathJaxTexFont, linebreaks: { inline: false } }),
});

globalThis.strataMath = (source, display) => {
  const node = document.convert(source, { display, em: 20, ex: 10, containerWidth: 800 });
  const svg = adaptor.firstChild(node);
  for (const dimension of ["width", "height"]) {
    const value = adaptor.getAttribute(svg, dimension);
    if (!value || !value.endsWith("ex")) throw new Error("Unsupported equation dimensions");
    adaptor.setAttribute(svg, dimension, String(parseFloat(value) * 10));
  }
  adaptor.setAttribute(svg, "color", "black");
  return adaptor.outerHTML(svg);
};
