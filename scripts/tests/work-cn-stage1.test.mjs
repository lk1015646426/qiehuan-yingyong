import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const accountSource = readFileSync('src-tauri/src/modules/account.rs', 'utf8');
const tauriDevScript = readFileSync('scripts/tauri-dev.cjs', 'utf8');
const packageJson = JSON.parse(readFileSync('package.json', 'utf8'));
const packageLock = JSON.parse(readFileSync('package-lock.json', 'utf8'));
const tauriConfig = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8'));
const baseCss = readFileSync('src/styles/base.css', 'utf8');

test('Work CN switcher uses an isolated application data directory', () => {
  assert.match(accountSource, /const DATA_DIR: &str = "\.trae_work_cn_switcher";/);
  assert.match(accountSource, /const DEV_DATA_DIR: &str = "\.trae_work_cn_switcher_dev";/);
  assert.match(accountSource, /const DATA_DIR_ENV: &str = "TRAE_WORK_CN_SWITCHER_DATA_DIR";/);
  assert.match(accountSource, /const PROFILE_ENV: &str = "TRAE_WORK_CN_SWITCHER_PROFILE";/);
  assert.match(accountSource, /"TRAE_WORK_CN_SWITCHER_TEST_DATA_DIR"/);
  assert.match(accountSource, /cfg!\(debug_assertions\)/);
  assert.doesNotMatch(accountSource, /const DATA_DIR: &str = "\.antigravity_cockpit";/);
  assert.match(tauriDevScript, /TRAE_WORK_CN_SWITCHER_PROFILE/);
  assert.doesNotMatch(tauriDevScript, /process\.env\.COCKPIT_TOOLS_PROFILE/);
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

test('updater plugin keeps valid inert config without upstream endpoints', () => {
  assert.equal(typeof tauriConfig.plugins.updater.pubkey, 'string');
  assert.ok(tauriConfig.plugins.updater.pubkey.length > 0);
  assert.deepEqual(tauriConfig.plugins.updater.endpoints, []);
  assert.equal(JSON.stringify(tauriConfig).includes('jlcodes99/cockpit-tools/releases'), false);
  assert.equal(tauriConfig.bundle.createUpdaterArtifacts, false);
});

test('Work CN app shell does not depend on remote font resources', () => {
  assert.doesNotMatch(baseCss, /fonts\.(?:googleapis|gstatic)\.com/);
});
