const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { createRequire } = require('node:module');
const ts = require('typescript');
module.exports = function createLoader(mocks = {}, globals = {}) {
  const cache = new Map();
  function load(file) {
    file = path.resolve(file);
    if (cache.has(file)) return cache.get(file);
    const exports = {}; cache.set(file, exports);
    const nativeRequire = createRequire(file);
    const requireFile = name => {
      if (Object.hasOwn(mocks, name)) return mocks[name];
      if (name.startsWith('.')) {
        const target = path.resolve(path.dirname(file), name + '.ts');
        if (fs.existsSync(target)) return load(target);
      }
      return nativeRequire(name);
    };
    vm.runInNewContext(ts.transpileModule(fs.readFileSync(file, 'utf8'), {
      compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
    }).outputText, { exports, require: requireFile, Uint8Array, TextEncoder, TextDecoder, Date, Map, Set, URL, AbortController, setTimeout, clearTimeout, console, ...globals }, { filename: file });
    return exports;
  }
  return load;
};
