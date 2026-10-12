#!/usr/bin/env node
'use strict';
// npx wrapper: downloads the native binary for this OS from GitHub Releases
// (once, cached) and runs it with the given arguments.
const fs = require('fs');
const os = require('os');
const path = require('path');
const https = require('https');
const { spawn } = require('child_process');
const { assetName, downloadUrl } = require('./platform');
const { version } = require('../package.json');

function cacheDir() {
  const base = process.env.LOCALAPPDATA || process.env.XDG_CACHE_HOME || path.join(os.homedir(), '.cache');
  return path.join(base, 'codex-peek', version);
}

function get(url, dest, redirects = 5) {
  return new Promise((resolve, reject) => {
    https.get(url, { headers: { 'User-Agent': 'codex-peek' } }, (res) => {
      if ([301, 302, 303, 307, 308].includes(res.statusCode) && redirects > 0) {
        res.resume();
        return resolve(get(res.headers.location, dest, redirects - 1));
      }
      if (res.statusCode !== 200) return reject(new Error(`HTTP ${res.statusCode} for ${url}`));
      const tmp = `${dest}.part`;
      const out = fs.createWriteStream(tmp);
      res.pipe(out);
      out.on('finish', () => out.close(() => { fs.renameSync(tmp, dest); resolve(); }));
      out.on('error', reject);
    }).on('error', reject);
  });
}

async function main() {
  let bin = process.env.CODEX_PEEK_BIN;
  if (!bin) {
    const asset = assetName();
    if (!asset) throw new Error(`unsupported platform ${process.platform}/${process.arch}`);
    bin = path.join(cacheDir(), asset);
    if (!fs.existsSync(bin)) {
      fs.mkdirSync(path.dirname(bin), { recursive: true });
      process.stderr.write(`codex-peek: downloading ${asset} v${version}...\n`);
      await get(downloadUrl(version, asset), bin);
      if (process.platform !== 'win32') fs.chmodSync(bin, 0o755);
    }
  }
  const child = spawn(bin, process.argv.slice(2), { stdio: 'inherit' });
  child.on('exit', (code) => process.exit(code ?? 0));
}

main().catch((e) => { console.error(`codex-peek: ${e.message}`); process.exit(1); });
