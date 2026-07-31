// CommonJS: require() with a string literal is an import.

const math = require('./math');

function legacyAdd(a, b) {
  return math.add(a, b);
}

module.exports = { legacyAdd };
