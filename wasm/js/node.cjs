// CommonJS entry. The package is an ES module, which `require` cannot load synchronously on
// every supported Node version, so each function loads it on first use; `load()` resolves to
// the whole module, including the error classes.
'use strict';

const load = () => import('./node.js');

module.exports = {
  load,
  render: async (html, options) => (await load()).render(html, options),
  renderParts: async (parts, options) => (await load()).renderParts(parts, options),
};
