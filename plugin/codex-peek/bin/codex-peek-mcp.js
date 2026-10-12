#!/usr/bin/env node
'use strict';
// Codex plugin MCP entry. Codex starts this when a session boots and closes
// stdin when it exits; `codex-peek --mcp` opens the preview window for exactly
// that lifetime. Finds or downloads the native binary, then hands over stdio.
const fs = require('fs');
const os = require('os');
const path = require('path');
const https = require('https');
const { spawn } = require('child_process');

const VERSION = '0.1.1';
const REPO = 'hucheng91/codex-peek';

function asset() {
  const o = { darwin: 'macos', win32: 'windows', linux: 'linux' }[process.platform];
  const a = { arm64: 'arm64', x64: 'x64' }[process.arch];
  if (!o || !a || (o === 'windows' && a !== 'x64')) return null;
  return `codex-peek-${o}-${a}${o === 'windows' ? '.exe' : ''}`;
}

function get(url, dest, n = 5) {
  return new Promise((resolve, reject) => {
    https.get(url, { headers: { 'User-Agent': 'codex-peek' } }, (res) => {
      if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location && n > 0) {
        res.resume();
        return resolve(get(res.headers.location, dest, n - 1));
      }
      if (res.statusCode !== 200) return reject(new Error(`HTTP ${res.statusCode}`));
      const out = fs.createWriteStream(`${dest}.part`);
      res.pipe(out);
      out.on('finish', () => out.close(() => { fs.renameSync(`${dest}.part`, dest); resolve(); }));
      out.on('error', reject);
    }).on('error', reject);
  });
}

async function binary() {
  if (process.env.CODEX_PEEK_BIN) return process.env.CODEX_PEEK_BIN;
  const name = asset();
  if (!name) return null;
  const base = process.env.LOCALAPPDATA || process.env.XDG_CACHE_HOME || path.join(os.homedir(), '.cache');
  const bin = path.join(base, 'codex-peek', VERSION, name);
  if (!fs.existsSync(bin)) {
    fs.mkdirSync(path.dirname(bin), { recursive: true });
    await get(`https://github.com/${REPO}/releases/download/v${VERSION}/${name}`, bin);
    if (process.platform !== 'win32') fs.chmodSync(bin, 0o755);
  }
  return bin;
}

// Fallback when no binary is available: still be a valid (empty) MCP server
// so Codex doesn't report a startup failure.
function emptyServer() {
  let buf = '';
  process.stdin.on('data', (d) => {
    buf += d;
    let i;
    while ((i = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, i); buf = buf.slice(i + 1);
      let m; try { m = JSON.parse(line); } catch { continue; }
      if (m.id === undefined) continue;
      const result = m.method === 'initialize'
        ? { protocolVersion: m.params?.protocolVersion || '2025-06-18', capabilities: { tools: {} }, serverInfo: { name: 'codex-peek', version: VERSION } }
        : m.method === 'tools/list' ? { tools: [] } : {};
      process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: m.id, result }) + '\n');
    }
  });
}

binary().then((bin) => {
  if (!bin || !fs.existsSync(bin)) return emptyServer();
  const extra = (process.env.CODEX_PEEK_ARGS || '').split(/\s+/).filter(Boolean);
  const child = spawn(bin, ['--mcp', ...extra], { stdio: 'inherit' });
  child.on('exit', (code) => process.exit(code ?? 0));
  child.on('error', () => emptyServer());
}).catch((e) => {
  process.stderr.write(`codex-peek: ${e.message}\n`);
  emptyServer();
});
