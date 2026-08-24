const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const {
  buildStagedAssetName,
  isAllowedReleaseAsset,
  normalizeReleaseAssetName,
  stageReleaseAssets,
} = require('./stage_release_assets.cjs');

function writeFile(filePath, content = 'asset') {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, content);
}

test('normalizes release asset names to the stable GitHub form', () => {
  assert.equal(
    normalizeReleaseAssetName('切换应用_1.2.3_x64-setup.exe'),
    '切换应用_1.2.3_x64-setup.exe',
  );
  assert.equal(
    normalizeReleaseAssetName('切换应用  1.2.3.dmg'),
    '切换应用.1.2.3.dmg',
  );
});

test('stages raw macOS updater archives with explicit architecture names', () => {
  assert.equal(
    buildStagedAssetName('macos', '切换应用.app.tar.gz', 'aarch64'),
    '切换应用_aarch64.app.tar.gz',
  );
  assert.equal(
    buildStagedAssetName('macos', '切换应用.app.tar.gz.sig', 'x64'),
    '切换应用_x64.app.tar.gz.sig',
  );
});

test('stages only whitelisted macOS release artifacts', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'qiehuan-yingyong-stage-assets-'));
  const assetsDir = path.join(root, 'bundle');
  const outputDir = path.join(root, 'staged');

  writeFile(path.join(assetsDir, 'dmg', '切换应用_1.2.3_aarch64.dmg'));
  writeFile(path.join(assetsDir, 'macos', '切换应用.app.tar.gz'));
  writeFile(path.join(assetsDir, 'macos', '切换应用.app.tar.gz.sig'), 'signature');
  writeFile(path.join(assetsDir, 'dmg', 'icon.icns'));
  writeFile(path.join(assetsDir, 'dmg', 'bundle_dmg.sh'));
  writeFile(path.join(assetsDir, 'share', 'create-dmg', 'support', 'template.applescript'));
  writeFile(path.join(assetsDir, 'macos', '切换应用.app', 'Contents', 'Info.plist'));

  const outputs = stageReleaseAssets({
    platform: 'macos',
    assetsDir,
    outputDir,
    macArch: 'aarch64',
  });

  assert.deepEqual(
    outputs.map((filePath) => path.basename(filePath)),
    [
      '切换应用_1.2.3_aarch64.dmg',
      '切换应用_aarch64.app.tar.gz',
      '切换应用_aarch64.app.tar.gz.sig',
    ],
  );
});

test('whitelist accepts release packages and rejects bundle helpers', () => {
  assert.equal(isAllowedReleaseAsset('windows', '切换应用_1.2.3_x64_en-US.msi'), true);
  assert.equal(isAllowedReleaseAsset('windows', 'bundle.wxs'), false);
  assert.equal(isAllowedReleaseAsset('linux', '切换应用_1.2.3_amd64.AppImage.sig'), true);
  assert.equal(isAllowedReleaseAsset('linux', 'AppRun'), false);
  assert.equal(isAllowedReleaseAsset('macos', '切换应用.app.tar.gz.sig'), true);
  assert.equal(isAllowedReleaseAsset('macos', 'template.applescript'), false);
});
