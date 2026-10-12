'use strict';
// Maps Node's platform/arch to the GitHub Release asset name built by CI.
const REPO = 'hucheng91/codex-peek';

function assetName(platform = process.platform, arch = process.arch) {
  const os = { darwin: 'macos', win32: 'windows', linux: 'linux' }[platform];
  const cpu = { arm64: 'arm64', x64: 'x64' }[arch];
  if (!os || !cpu) return null;
  if (os === 'windows' && cpu !== 'x64') return null;
  return `codex-peek-${os}-${cpu}${os === 'windows' ? '.exe' : ''}`;
}

function downloadUrl(version, asset) {
  return `https://github.com/${REPO}/releases/download/v${version}/${asset}`;
}

module.exports = { assetName, downloadUrl, REPO };
