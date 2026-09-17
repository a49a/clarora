const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const loader = require('./helpers/load-ts.cjs');

function setup(native) {
  return loader({
    './platform': { currentPlatform: 'macos', getNativeModules: () => ({ RNMacKeychain: native }) },
  })(path.join(__dirname, '../shared/services/secrets.ts'));
}

test('concurrent and repeated reads share one native read; writes update the cache', async () => {
  let reads = 0;
  const api = setup({
    getSecret: async () => { reads++; return 'old'; },
    setSecret: async () => {}, deleteSecret: async () => {},
  });
  const store = api.secretStore();
  assert.equal(store, api.secretStore());
  assert.deepEqual(await Promise.all([store.getSecret('key'), store.getSecret('key')]), ['old', 'old']);
  await store.setSecret('key', 'new');
  assert.equal(await store.getSecret('key'), 'new');
  await store.deleteSecret('key');
  assert.equal(await store.getSecret('key'), null);
  assert.equal(reads, 1);
});

test('denial stops queued reads across accounts and prevents migration writes', async () => {
  let reads = 0;
  let writes = 0;
  const denied = new Error('Access denied');
  const api = setup({
    getSecret: async () => { reads++; throw denied; },
    setSecret: async () => { writes++; }, deleteSecret: async () => {},
  });
  const store = api.secretStore();
  const results = await Promise.allSettled([
    api.migrateSecret(store, 'ai', 'keep-existing-key'),
    store.getSecret('storage'), store.getSecret('ai'),
  ]);
  assert.ok(results.every(result => result.status === 'rejected' && result.reason === denied));
  assert.equal(reads, 1);
  assert.equal(writes, 0);
});

test('failed migration writes propagate so callers retain the original configuration', async () => {
  const failed = new Error('Write failed');
  const api = setup({
    getSecret: async () => null,
    setSecret: async () => { throw failed; }, deleteSecret: async () => {},
  });
  await assert.rejects(api.migrateSecret(api.secretStore(), 'ai', 'keep-existing-key'), failed);
});
