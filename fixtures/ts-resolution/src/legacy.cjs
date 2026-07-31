// CommonJS: `const m = require('./x')` binds a namespace, so `m.foo()` resolves.

const math = require('./math');

function legacyAdd(a, b) {
  return math.add(a, b);
}

module.exports = { legacyAdd };
