import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const accountSource = readFileSync('src-tauri/src/modules/account.rs', 'utf8');
const tauriDevScript = readFileSync('scripts/tauri-dev.cjs', 'utf8');
const packageJson = JSON.parse(readFileSync('package.json', 'utf8'));
const packageLock = JSON.parse(readFileSync('package-lock.json', 'utf8'));
const tauriConfig = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8'));
const baseCss = readFileSync('src/styles/base.css', 'utf8');

test('切换应用使用隔离的数据目录并兼容旧环境变量', () => {
  assert.match(accountSource, /const DATA_DIR: &str = "\.qiehuan_yingyong";/);
  assert.match(accountSource, /const DEV_DATA_DIR: &str = "\.qiehuan_yingyong_dev";/);
  assert.match(accountSource, /const DATA_DIR_ENV: &str = "QIEHUAN_YINGYONG_DATA_DIR";/);
  assert.match(accountSource, /const PROFILE_ENV: &str = "QIEHUAN_YINGYONG_PROFILE";/);
  assert.match(accountSource, /"QIEHUAN_YINGYONG_TEST_DATA_DIR"/);
  assert.match(accountSource, /"TRAE_WORK_CN_SWITCHER_TEST_DATA_DIR"/);
  assert.match(accountSource, /"COCKPIT_TOOLS_TEST_DATA_DIR"/);
  assert.match(accountSource, /cfg!\(debug_assertions\)/);
  assert.doesNotMatch(accountSource, /const DATA_DIR: &str = "\.antigravity_cockpit";/);
  assert.match(tauriDevScript, /QIEHUAN_YINGYONG_PROFILE/);
  assert.match(tauriDevScript, /QIEHUAN_YINGYONG_API_PORT/);
  assert.match(tauriDevScript, /VITE_QIEHUAN_YINGYONG_PROFILE/);
  assert.doesNotMatch(tauriDevScript, /TRAE_WORK_CN_SWITCHER_PROFILE|COCKPIT_TOOLS_PROFILE/);
  assert.match(tauriDevScript, /process\.platform === 'win32' \? 'npm\.cmd' : 'npm'/);
  assert.match(tauriDevScript, /npx(?:\.cmd)?/);
  assert.match(tauriDevScript, /src-tauri\/tauri\.dev\.conf\.json/);
});

test('package lock root metadata matches the specialized package', () => {
  assert.equal(packageLock.name, packageJson.name);
  assert.equal(packageLock.version, packageJson.version);
  assert.equal(packageLock.packages[''].name, packageJson.name);
  assert.equal(packageLock.packages[''].version, packageJson.version);
});

test('Tauri bundle uses the renamed identity and keeps the legacy deep link', () => {
  assert.equal(tauriConfig.productName, '切换应用');
  assert.equal(tauriConfig.identifier, 'com.qiehuanyingyong.desktop');
  assert.deepEqual(tauriConfig.plugins?.updater, undefined);
  assert.ok(tauriConfig.plugins['deep-link'].desktop.schemes.includes('qiehuanyingyong'));
  assert.ok(tauriConfig.plugins['deep-link'].desktop.schemes.includes('cockpit-tools'));
  assert.equal(tauriConfig.bundle.createUpdaterArtifacts, false);
});

test('Work CN app shell does not depend on remote font resources', () => {
  assert.doesNotMatch(baseCss, /fonts\.(?:googleapis|gstatic)\.com/);
});
