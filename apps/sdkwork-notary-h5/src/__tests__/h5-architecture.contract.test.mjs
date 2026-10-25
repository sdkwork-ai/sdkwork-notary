#!/usr/bin/env node

import assert from 'node:assert/strict';
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const appRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');

function read(relativePath) {
  return readFileSync(path.join(appRoot, relativePath), 'utf8');
}

function readJson(relativePath) {
  return JSON.parse(read(relativePath));
}

function listPackageNames() {
  const packagesDir = path.join(appRoot, 'packages');
  return readdirSync(packagesDir).filter((entry) => statSync(path.join(packagesDir, entry)).isDirectory());
}

const REQUIRED_LAYOUT_PATHS = [
  'AGENTS.md',
  'agent.app.config.json',
  '.sdkwork/README.md',
  '.sdkwork/skills/README.md',
  '.sdkwork/plugins/README.md',
  'bin/ios/README.md',
  'bin/android/README.md',
  'config/browser/runtime-env.development.example.json',
  'config/browser/runtime-env.test.example.json',
  'config/browser/runtime-env.staging.example.json',
  'config/browser/runtime-env.production.example.json',
  'config/host/capacitor.development.example.json',
  'config/host/capacitor.test.example.json',
  'config/host/capacitor.staging.example.json',
  'config/host/capacitor.production.example.json',
  'config/server/notary.development.toml.example',
  'config/server/notary.test.toml.example',
  'config/server/notary.staging.toml.example',
  'config/server/notary.production.toml.example',
  'config/container/notary.development.toml.example',
  'config/container/notary.test.toml.example',
  'config/container/notary.staging.toml.example',
  'config/container/notary.production.toml.example',
  'docs/README.md',
  'public/README.md',
  'scripts/README.md',
  'sdks/README.md',
  'specs/README.md',
  'tests/README.md',
  'src/main.tsx',
  'src/App.tsx',
  'src/AuthGate.tsx',
  'src/providers/README.md',
  'src/shell/README.md',
  'src/routes/README.md',
  'src/bootstrap/environment.ts',
  'src/bootstrap/tokenManager.ts',
  'src/bootstrap/runtime.ts',
  'src/bootstrap/sdkClients.ts',
  'src/bootstrap/iamRuntime.ts',
  'src/bootstrap/hostAdapters.ts',
  'src/bootstrap/routes.ts',
  'packages/sdkwork-notary-h5-core/package.json',
  'packages/sdkwork-notary-h5-commons/package.json',
  'packages/sdkwork-notary-h5-shell/package.json',
  'packages/sdkwork-notary-h5-notary/package.json',
];

test('notary h5 root follows APP_H5_ARCHITECTURE_SPEC layout', () => {
  for (const relativePath of REQUIRED_LAYOUT_PATHS) {
    assert.equal(existsSync(path.join(appRoot, relativePath)), true, `missing ${relativePath}`);
  }

  const manifest = readJson('agent.app.config.json');
  assert.equal(manifest.schemaVersion, 3);
  assert.equal(manifest.kind, 'agent.app');
  assert.equal(manifest.app.key, 'sdkwork-notary-h5');
  assert.equal(manifest.runtime.family, 'mobile');
  assert.equal(manifest.runtime.framework, 'react-h5');

  const packageNames = listPackageNames();
  for (const required of [
    'sdkwork-notary-h5-core',
    'sdkwork-notary-h5-commons',
    'sdkwork-notary-h5-shell',
    'sdkwork-notary-h5-notary',
  ]) {
    assert(packageNames.includes(required), `packages must include ${required}`);
  }
});

test('notary h5 root keeps a thin bootstrap shell and SDK-backed notary package', () => {
  const app = read('src/App.tsx');
  const runtime = read('src/bootstrap/runtime.ts');
  const sdkClients = read('src/bootstrap/sdkClients.ts');
  const environment = read('src/bootstrap/environment.ts');
  const tokenManager = read('src/bootstrap/tokenManager.ts');
  const notaryHome = read('packages/sdkwork-notary-h5-notary/src/pages/NotaryHomePage.tsx');
  const notaryService = read('packages/sdkwork-notary-h5-notary/src/services/notaryH5Service.ts');
  const core = read('packages/sdkwork-notary-h5-core/src/index.ts');
  const rootPackage = readJson('package.json');

  assert(app.includes('BrowserRouter'));
  assert(app.includes('AuthGate'));
  assert(app.includes('NotaryApp'));
  assert(app.includes('bootstrap()'));
  assert(runtime.includes('createIamRuntime'));
  assert(runtime.includes('bootstrapSdkClients'));
  assert(sdkClients.includes('initNotaryH5AppSdkClient'));
  assert(sdkClients.includes("platform: 'h5'"));
  assert(environment.includes('@sdkwork/utils/string'));
  assert(environment.includes('isBlank'));
  assert(environment.includes('VITE_SDKWORK_NOTARY_APPLICATION_PUBLIC_HTTP_URL'));
  assert(tokenManager.includes('createTokenManager'));
  assert(tokenManager.includes('@sdkwork/sdk-common'));
  assert(core.includes('createNotaryH5ComposedApi'));
  assert(core.includes('getNotaryH5ComposedApi'));
  assert(notaryService.includes('getNotaryH5ComposedApi'));
  assert(notaryService.includes('@sdkwork/utils/string'));
  assert(notaryService.includes('getDashboardStatistics'));
  assert(notaryService.includes('listCases'));
  assert(notaryHome.includes('createNotaryH5Service'));
  assert(notaryHome.includes('Recent cases'));
  assert(rootPackage.dependencies['@sdkwork/utils']);

  for (const forbidden of ['fetch(', 'axios', 'Authorization', 'Access-Token', 'picsum.photos']) {
    assert(!notaryService.includes(forbidden), `notaryH5Service must not include ${forbidden}`);
    assert(!notaryHome.includes(forbidden), `NotaryHomePage must not include ${forbidden}`);
  }
});
