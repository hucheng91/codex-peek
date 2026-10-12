'use strict';
const test = require('node:test');
const assert = require('node:assert');
const { assetName, downloadUrl } = require('./platform');

test('asset names', () => {
  assert.strictEqual(assetName('darwin', 'arm64'), 'codex-peek-macos-arm64');
  assert.strictEqual(assetName('darwin', 'x64'), 'codex-peek-macos-x64');
  assert.strictEqual(assetName('win32', 'x64'), 'codex-peek-windows-x64.exe');
  assert.strictEqual(assetName('linux', 'x64'), 'codex-peek-linux-x64');
  assert.strictEqual(assetName('win32', 'arm64'), null);
  assert.strictEqual(assetName('aix', 'x64'), null);
});

test('download url', () => {
  assert.match(downloadUrl('0.1.0', 'a'), /releases\/download\/v0\.1\.0\/a$/);
});
