const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const loader = require('./helpers/load-ts.cjs');
const file = path.join(__dirname, '../shared/services/pdf.ts');
let renderCalls = 0;

function setup(natives = {}) {
  const files = new Set();
  const api = loader({
    './platform': {
      currentPlatform: 'macos',
      nativePath: value => value,
      getNativeModules: () => ({ RNMacPdf: natives.mac, RNWindowsPdf: natives.windows }),
      FileSystem: {
        getDocumentDirectoryAsync: async () => '/docs/',
        makeDirectoryAsync: async () => {},
        listFilesAsync: async () => [...files],
        copyAsync: async ({ to }) => files.add(to.split('/').pop()),
      },
    },
  })(file);
  return { api, files, count: () => renderCalls };
}

const macModule = () => ({
  // macOS 原生模块直接 resolve 字典（非 JSON 字符串）
  open: async () => ({
    pages: [{ width: 595, height: 842 }],
    outline: [{ title: '第一章', page: 0, children: [{ title: '小节', page: -1, children: [] }] }],
  }),
  renderPage: async () => { renderCalls += 1; return { png: 'cGFnZQ==' }; },
});

test('open maps page sizes and the outline tree', async () => {
  const s = setup({ mac: macModule() });
  const meta = await s.api.openPdf('/docs/pdf/a.pdf');
  assert.equal(meta.pages.length, 1);
  assert.equal(meta.outline[0].title, '第一章');
  assert.equal(meta.outline[0].page, 0);
  assert.equal(meta.outline[0].children[0].title, '小节');
});

test('render cache reuses one native call per page+width', async () => {
  const s = setup({ mac: macModule() });
  const first = await s.api.renderPdfPage('/docs/pdf/a.pdf', 0, 720);
  const second = await s.api.renderPdfPage('/docs/pdf/a.pdf', 0, 720);
  assert.equal(first, 'data:image/png;base64,cGFnZQ==');
  assert.equal(second, first);
  assert.equal(s.count(), 1);
});

test('library lists pdf files and import validates extension', async () => {
  const s = setup({ mac: macModule() });
  await assert.rejects(s.api.importPdf('/tmp/a.txt', 'a.txt'), /请选择 PDF 文件/);
  await s.api.importPdf('/tmp/book.pdf', 'Book.PDF');
  const library = await s.api.listPdfLibrary();
  assert.equal(library.length, 1);
  assert.match(library[0].name, /book$/i);
});

test('unsupported platform reports setup guidance', async () => {
  const s = setup({});
  await assert.rejects(s.api.openPdf('/docs/pdf/a.pdf'), /macOS 和 Windows/);
});
