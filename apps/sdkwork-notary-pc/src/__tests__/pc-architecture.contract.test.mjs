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
  'sdkwork.app.config.json',
  '.sdkwork/README.md',
  '.sdkwork/skills/README.md',
  '.sdkwork/plugins/README.md',
  'bin/windows/README.md',
  'bin/linux/README.md',
  'bin/macos/README.md',
  'config/browser/runtime-env.development.example.json',
  'config/browser/runtime-env.test.example.json',
  'config/browser/runtime-env.staging.example.json',
  'config/browser/runtime-env.production.example.json',
  '.env.example',
  'config/desktop/notary.development.toml.example',
  'config/desktop/notary.test.toml.example',
  'config/desktop/notary.staging.toml.example',
  'config/desktop/notary.production.toml.example',
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
  'src/AdminSurface.tsx',
  'src/AuthGate.tsx',
  'src/providers/README.md',
  'src/shell/README.md',
  'src/routes/README.md',
  'src/bootstrap/environment.ts',
  'src/bootstrap/tokenManager.ts',
  'src/bootstrap/runtime.ts',
  'src/bootstrap/sdkClients.ts',
  'src/bootstrap/iamRuntime.ts',
  'src/bootstrap/routes.ts',
  'packages/sdkwork-notary-pc-core/src/appAuthRuntime.ts',
  'packages/sdkwork-notary-pc-core/src/appAuthService.ts',
  'packages/sdkwork-notary-pc-core/src/session.ts',
  'packages/sdkwork-notary-pc-core/src/sdkSessionLifecycle.ts',
  'packages/sdkwork-notary-pc-core/package.json',
  'packages/sdkwork-notary-pc-commons/package.json',
  'packages/sdkwork-notary-pc-shell/package.json',
  'packages/sdkwork-notary-pc-notary/package.json',
  'packages/sdkwork-notary-pc-admin-core/package.json',
  'packages/sdkwork-notary-pc-admin-shell/package.json',
  'packages/sdkwork-notary-pc-admin-merchandise/package.json',
  'packages/sdkwork-notary-pc-core/specs/component.spec.json',
  'packages/sdkwork-notary-pc-commons/specs/component.spec.json',
  'packages/sdkwork-notary-pc-shell/specs/component.spec.json',
  'packages/sdkwork-notary-pc-notary/specs/component.spec.json',
  'packages/sdkwork-notary-pc-admin-core/specs/component.spec.json',
  'packages/sdkwork-notary-pc-admin-shell/specs/component.spec.json',
  'packages/sdkwork-notary-pc-admin-merchandise/specs/component.spec.json',
];

test('notary pc root follows APP_PC_ARCHITECTURE_SPEC layout', () => {
  for (const relativePath of REQUIRED_LAYOUT_PATHS) {
    assert.equal(existsSync(path.join(appRoot, relativePath)), true, `missing ${relativePath}`);
  }

  const manifest = readJson('sdkwork.app.config.json');
  assert.equal(manifest.schemaVersion, 3);
  assert.equal(manifest.kind, 'sdkwork.app');
  assert.equal(manifest.app.key, 'sdkwork-notary-pc');
  assert.equal(manifest.runtime.family, 'pc');
  assert.equal(manifest.runtime.framework, 'react-pc');

  for (const required of [
    'sdkwork-notary-pc-core',
    'sdkwork-notary-pc-commons',
    'sdkwork-notary-pc-shell',
    'sdkwork-notary-pc-notary',
    'sdkwork-notary-pc-admin-core',
    'sdkwork-notary-pc-admin-shell',
    'sdkwork-notary-pc-admin-merchandise',
  ]) {
    assert(listPackageNames().includes(required), `packages must include ${required}`);
  }
});

test('notary pc backend-admin uses the backend SDK surface and isolated packages', () => {
  const app = read('src/App.tsx');
  const adminSurface = read('src/AdminSurface.tsx');
  const environment = read('src/bootstrap/environment.ts');
  const adminCore = read('packages/sdkwork-notary-pc-admin-core/src/sdk/backendSdk.ts');
  const adminRoutes = read('packages/sdkwork-notary-pc-admin-shell/src/routes/AdminRoutes.tsx');
  const matterService = read(
    'packages/sdkwork-notary-pc-admin-merchandise/src/services/notaryMatterAdminService.ts',
  );
  const tsconfig = readJson('tsconfig.json');

  assert(app.includes('path="/admin/*"'));
  assert(app.includes('<AdminSurface />'));
  assert(adminSurface.includes('resolveEnvironment().backendApiBaseUrl'));
  assert(environment.includes('VITE_SDKWORK_NOTARY_APPLICATION_BACKEND_HTTP_URL'));
  assert(environment.includes('@sdkwork/sdk-common'));
  assert(environment.includes('resolveBaseUrl()'));
  assert(adminCore.includes("from '@sdkwork/notary-backend-sdk'"));
  assert(adminCore.includes('getNotaryPcGlobalTokenManager()'));
  assert(adminRoutes.includes("import('@sdkwork/notary-pc-admin-merchandise')"));
  assert(adminRoutes.includes('NotaryPcAdminPermissionGate'));
  assert(matterService.includes("from '@sdkwork/notary-pc-admin-core'"));
  assert(matterService.includes('management.list(input)'));
  assert(matterService.includes('idempotencyKey = uuid()'));
  assert.deepEqual(tsconfig.compilerOptions.paths['@sdkwork/notary-backend-sdk'], [
    '../../sdks/sdkwork-notary-backend-sdk/sdkwork-notary-backend-sdk-typescript/src/index.ts',
  ]);

  for (const source of [adminSurface, adminCore, adminRoutes, matterService]) {
    for (const forbidden of [
      'fetch(',
      'axios',
      'Authorization',
      'Access-Token',
      'generated/server-openapi',
      '@sdkwork/commerce-',
      '@sdkwork/order-',
      '@sdkwork/payment-',
    ]) {
      assert(!source.includes(forbidden), `backend-admin source must not include ${forbidden}`);
    }
  }
});

test('notary pc root keeps thin bootstrap and host-port based notary package', () => {
  const app = read('src/App.tsx');
  const runtime = read('src/bootstrap/runtime.ts');
  const sdkClients = read('src/bootstrap/sdkClients.ts');
  const environment = read('src/bootstrap/environment.ts');
  const iamRuntime = read('src/bootstrap/iamRuntime.ts');
  const authGate = read('src/AuthGate.tsx');
  const session = read('packages/sdkwork-notary-pc-core/src/session.ts');
  const sdkSessionLifecycle = read('packages/sdkwork-notary-pc-core/src/sdkSessionLifecycle.ts');
  const appAuthRuntime = read('packages/sdkwork-notary-pc-core/src/appAuthRuntime.ts');
  const appAuthService = read('packages/sdkwork-notary-pc-core/src/appAuthService.ts');
  const viteConfig = read('vite.config.ts');
  const notaryService = read('packages/sdkwork-notary-pc-notary/src/services/NotaryService.ts');
  const notaryRoutes = read('packages/sdkwork-notary-pc-shell/src/notaryRoutes.tsx');
  const notaryI18n = read('packages/sdkwork-notary-pc-notary/src/i18n/index.ts');
  const notaryView = read('packages/sdkwork-notary-pc-notary/src/NotaryView.tsx');
  const host = read('packages/sdkwork-notary-pc-commons/src/host/notaryPcHost.ts');
  const core = read('packages/sdkwork-notary-pc-core/src/index.ts');
  const tsconfig = readJson('tsconfig.json');

  assert(app.includes('AuthGate'));
  assert(app.includes('bootstrap()'));
  assert(runtime.includes('bootstrapSdkClients'));
  assert(runtime.includes('finalizeIamRuntime'));
  assert(sdkClients.includes('configureNotaryPcRuntime'));
  assert(sdkClients.includes('initNotaryPcDriveAppSdkClient'));
  assert(sdkClients.includes('initNotaryPcAppbaseAppSdkClient'));
  assert(sdkClients.includes('getNotaryPcDriveAppSdkClient'));
  assert(sdkClients.includes('getNotaryPcAppbaseAppSdkClient'));
  assert(sdkClients.includes('registerNotaryPcSdkClientRefresh'));
  assert(!sdkClients.includes('getDriveClient: () => ({})'));
  assert(iamRuntime.includes('createNotaryPcTokenManager'));
  assert(iamRuntime.includes('registerNotaryPcServiceReset'));
  assert(session.includes('SDKWORK_ACCESS_TOKEN'));
  assert(session.includes('createSdkworkAppbasePcAuthRuntime') === false);
  assert(sdkSessionLifecycle.includes('refreshAuthenticatedNotaryPcSdkClients'));
  assert(sdkSessionLifecycle.includes('enableNotaryPcSessionLifecycle'));
  assert(appAuthRuntime.includes('createSdkworkAppbasePcAuthRuntime'));
  assert(appAuthRuntime.includes('getNotaryPcIamRuntime'));
  assert(appAuthService.includes('notaryPcAuthService'));
  assert(appAuthService.includes('sessions.current.retrieve'));
  assert(authGate.includes('SdkworkIamAuthRoutes'));
  assert(authGate.includes('getNotaryPcIamRuntime'));
  assert(authGate.includes("AUTH_BASE_PATH = '/auth'"));
  assert(authGate.includes('/login?'));
  assert(authGate.includes('notaryPcAuthService.getCurrentSession'));
  assert(viteConfig.includes('@sdkwork/drive-app-sdk'));
  assert(viteConfig.includes('@sdkwork/iam-app-sdk'));
  assert(viteConfig.includes('@sdkwork/auth-runtime-pc-react'));
  assert.deepEqual(tsconfig.compilerOptions.paths['@sdkwork/notary-app-sdk'], [
    '../../sdks/sdkwork-notary-app-sdk/sdkwork-notary-app-sdk-typescript/src/index.ts',
  ]);
  assert.deepEqual(tsconfig.compilerOptions.paths['@sdkwork/drive-app-sdk'], [
    '../../../sdkwork-drive/sdks/sdkwork-drive-app-sdk/sdkwork-drive-app-sdk-typescript/src/index.ts',
  ]);
  assert.deepEqual(tsconfig.compilerOptions.paths['@sdkwork/iam-app-sdk'], [
    '../../../sdkwork-iam/sdks/sdkwork-iam-app-sdk/sdkwork-iam-app-sdk-typescript/src/index.ts',
  ]);
  assert(tsconfig.exclude.includes('packages/sdkwork-notary-pc-core/src/notaryAppSdk.d.ts'));
  assert(tsconfig.exclude.includes('packages/sdkwork-notary-pc-core/src/dependencyAppSdk.d.ts'));
  assert(notaryRoutes.includes('lazy('));
  assert(notaryRoutes.includes("import('@sdkwork/notary-pc-notary')"));
  assert(notaryRoutes.includes('Suspense'));
  assert(notaryRoutes.includes('LoadingState'));
  assert(environment.includes('resolveEnvironment'));
  assert(core.includes('configureNotaryPcSdkPorts'));
  assert(core.includes('getNotaryPcIamRuntime'));
  assert(host.includes('configureNotaryPcHost'));
  assert(notaryService.includes('createNotaryPcService'));
  assert(notaryService.includes('getConfiguredNotaryAppSdkClient'));
  assert(notaryI18n.includes('resolveNotaryHostLanguage'));
  assert(notaryI18n.includes('syncNotaryI18nFromHost'));
  assert(notaryView.includes('I18nextProvider'));
  assert(notaryView.includes('syncNotaryI18nFromHost'));

  for (const forbidden of ['fetch(', 'axios', 'Authorization', 'Access-Token', 'picsum.photos']) {
    assert(!notaryService.includes(forbidden), `NotaryService must not include ${forbidden}`);
  }
});

test('notary pc task interactions use cursor pagination and preserve party attachments', () => {
  const notaryService = read('packages/sdkwork-notary-pc-notary/src/services/NotaryService.ts');
  const notaryView = read('packages/sdkwork-notary-pc-notary/src/NotaryView.tsx');
  const taskTable = read('packages/sdkwork-notary-pc-notary/src/components/list/NotaryTaskTable.tsx');
  const partyDrawer = read('packages/sdkwork-notary-pc-notary/src/PartyDrawer.tsx');
  const commonTypes = read('packages/sdkwork-notary-pc-commons/src/types/notary.ts');

  assert(notaryService.includes('NotaryTaskPageInfo'));
  assert(notaryService.includes("mode: 'cursor'"));
  assert(notaryService.includes('pageInfo: mapTaskPageInfo'));
  assert(notaryService.includes('syncPartyAuxiliaryAttachments'));
  assert(!notaryService.includes('syncCaseAssignments'));
  assert(notaryService.includes("'电子合同公证': 'sku-notary-electronic-contract'"));
  assert(notaryService.includes("'知识产权确权公证': 'sku-notary-ipr'"));
  assert(notaryService.includes("'电子证据固化': 'sku-notary-evidence'"));
  assert(notaryService.includes("'商业秘密确权': 'sku-notary-trade-secret'"));
  assert(notaryService.includes("'抽奖摇号公证': 'sku-notary-lottery'"));
  assert(notaryService.includes("'遗嘱公证': 'sku-notary-will'"));

  assert(notaryView.includes('taskPageCursorByPageRef'));
  assert(notaryView.includes('cursor: pageCursor'));
  assert(!notaryView.includes('.slice('));
  assert(!taskTable.includes('paginatedTasks'));
  assert(partyDrawer.includes('auxiliaryAttachments: attachments.map'));
  assert(commonTypes.includes('auxiliaryAttachments?: File[]'));
  assert(commonTypes.includes("'CANCELLED'"));
});

test('notary pc create intent owns one stable secure idempotency key', () => {
  const createView = read('packages/sdkwork-notary-pc-notary/src/CreateNotaryTaskView.tsx');
  const notaryService = read('packages/sdkwork-notary-pc-notary/src/services/NotaryService.ts');
  const idempotencyKeys = read('packages/sdkwork-notary-pc-notary/src/utils/createCaseIdempotencyKey.ts');

  assert(createView.includes('const [idempotencyKey] = useState(createNotaryCaseIntentIdempotencyKey);'));
  assert(createView.includes('idempotencyKey,'));
  assert(notaryService.includes('idempotencyKey?: string;'));
  assert(notaryService.includes('resolveNotaryCaseIdempotencyKey(data.idempotencyKey)'));
  assert(!notaryService.includes('buildIdempotencyKey'));
  assert(idempotencyKeys.includes("from '@sdkwork/utils/id'"));
  assert(idempotencyKeys.includes('uuid()'));
  assert(idempotencyKeys.includes('callerKey || createNotaryCaseIntentIdempotencyKey()'));
  assert(!idempotencyKeys.includes('Date.now'));
  assert(!idempotencyKeys.includes('Math.random'));
  assert(!idempotencyKeys.includes('traceId'));
  assert(!idempotencyKeys.includes('requestId'));
});

test('notary pc interactive pages keep remote workflows behind NotaryService', () => {
  const notaryView = read('packages/sdkwork-notary-pc-notary/src/NotaryView.tsx');
  const createView = read('packages/sdkwork-notary-pc-notary/src/CreateNotaryTaskView.tsx');
  const partyDrawer = read('packages/sdkwork-notary-pc-notary/src/PartyDrawer.tsx');
  const notaryService = read('packages/sdkwork-notary-pc-notary/src/services/NotaryService.ts');

  for (const source of [notaryView, createView, partyDrawer]) {
    assert(!source.includes('createNotaryApi('));
    assert(!source.includes('getConfiguredNotaryAppSdkClient'));
    assert(!source.includes('fetch('));
    assert(!source.includes('axios'));
    assert(!source.includes('Authorization'));
    assert(!source.includes('Access-Token'));
  }

  assert(notaryView.includes('notaryService.getTasks'));
  assert(notaryView.includes('notaryService.getDocumentUrl(selectedTask.id, doc,'));
  assert(createView.includes('notaryService.createTask'));
  assert(notaryService.includes('notaryApi.createCase'));
  assert(notaryService.includes('notaryApi.createCaseFileDownloadUrl'));
});
