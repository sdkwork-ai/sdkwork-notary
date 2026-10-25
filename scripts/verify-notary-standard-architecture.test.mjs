import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';

const ROOT = process.cwd();

const STANDARD_ROOT_DIRECTORIES = [
  'apis',
  'apps',
  'crates',
  'database',
  'sdks',
  'jobs',
  'tools',
  'plugins',
  'examples',
  'configs',
  'deployments',
  'scripts',
  'docs',
  'tests',
];

const REQUIRED_WORKSPACE_FILES = [
  'AGENTS.md',
  'CLAUDE.md',
  'CODEX.md',
  'GEMINI.md',
  'README.md',
  'Cargo.toml',
  'agent.workflow.json',
  '.github/workflows/package.yml',
  '.sdkwork/README.md',
  '.sdkwork/.gitignore',
  '.sdkwork/skills/README.md',
  '.sdkwork/plugins/README.md',
  'docs/root-layout.md',
];

const API_INPUTS = [
  'apis/app-api/notary/notary-app-api.openapi.json',
  'apis/backend-api/notary/notary-backend-api.openapi.json',
];

const ROUTE_MANIFESTS = [
  {
    file: 'sdks/_route-manifests/app-api/sdkwork-routes-notary-app-api.route-manifest.json',
    apiSurface: 'app-api',
  },
  {
    file: 'sdks/_route-manifests/backend-api/sdkwork-routes-notary-backend-api.route-manifest.json',
    apiSurface: 'backend-api',
  },
];

const WEB_FRAMEWORK_CRATES = [
  'crates/sdkwork-routes-notary-app-api/Cargo.toml',
  'crates/sdkwork-routes-notary-backend-api/Cargo.toml',
  'crates/sdkwork-routes-notary-http-auth/Cargo.toml',
];

function read(relativePath) {
  return readFileSync(path.join(ROOT, relativePath), 'utf8');
}

function readJson(relativePath) {
  return JSON.parse(read(relativePath));
}

function exists(relativePath) {
  return existsSync(path.join(ROOT, relativePath));
}

test('declares sdkwork-notary-pc application root for host integrations', () => {
  assert.equal(exists('apps/sdkwork-notary-pc/package.json'), true);
  assert.equal(exists('apps/sdkwork-notary-pc/packages/sdkwork-notary-pc-notary/package.json'), true);
});

test('declares SDKWork standard root directory dictionary', () => {
  for (const directory of STANDARD_ROOT_DIRECTORIES) {
    assert.equal(exists(directory), true, `${directory}/ should exist`);
    assert.equal(exists(path.join(directory, 'README.md')), true, `${directory}/README.md should exist`);
  }
});

test('declares workspace agent entrypoints and packaging workflow', () => {
  for (const file of REQUIRED_WORKSPACE_FILES) {
    assert.equal(exists(file), true, `${file} should exist`);
  }

  const workflow = readJson('agent.workflow.json');
  assert.equal(workflow.app.id, 'sdkwork-notary');
  assert.equal(workflow.app.configPath, 'agent.app.config.json');
});

test('declares author-owned API contracts under apis/', () => {
  for (const file of API_INPUTS) {
    assert.equal(exists(file), true, `${file} should exist`);
    const openapi = readJson(file);
    assert.equal(openapi.openapi, '3.1.2');
  }
});

test('integrates sdkwork-web-framework in HTTP route crates', () => {
  const rootCargo = read('Cargo.toml');
  assert.match(rootCargo, /sdkwork-web-core/);
  assert.match(rootCargo, /sdkwork-web-axum/);
  assert.match(rootCargo, /sdkwork-iam-web-adapter/);

  for (const cargoPath of WEB_FRAMEWORK_CRATES) {
    const cargo = read(cargoPath);
    assert.match(cargo, /sdkwork-web-/);
  }

  const httpAuthLayer = read('crates/sdkwork-routes-notary-http-auth/src/layer.rs');
  assert.match(httpAuthLayer, /build_web_framework_layer/);
  assert.match(httpAuthLayer, /with_web_request_context/);
});

test('integrates sdkwork-database in notary SQLx repository crate', () => {
  const repositoryCargo = read('crates/sdkwork-notary-case-repository-sqlx/Cargo.toml');
  assert.match(repositoryCargo, /sdkwork-database-config/);
  assert.match(repositoryCargo, /sdkwork-database-sqlx/);
  assert.match(repositoryCargo, /sdkwork-utils-rust/);

  const databaseHostCargo = read('crates/sdkwork-notary-database-host/Cargo.toml');
  assert.match(databaseHostCargo, /sdkwork-database-lifecycle/);
  assert.match(databaseHostCargo, /sdkwork-database-spi/);

  assert.equal(exists('database/database.manifest.json'), true);
  assert.equal(exists('database/contract/schema.yaml'), true);
  assert.equal(exists('database/ddl/baseline/postgres/0001_notary_baseline.sql'), true);
  assert.equal(exists('tests/fixtures/database/sqlite/ddl/baseline/0001_notary_baseline.sql'), true);
  assert.equal(
    exists('crates/sdkwork-notary-case-repository-sqlx/migrations/0001_notary_foundation.sql'),
    false,
    'legacy crate migration must be removed',
  );
});

test('integrates sdkwork-utils-rust for shared datetime and crypto helpers', () => {
  const rootCargo = read('Cargo.toml');
  assert.match(rootCargo, /sdkwork-utils-rust/);

  const workflow = readJson('agent.workflow.json');
  const dependencyIds = new Set((workflow.dependencies ?? []).map((dependency) => dependency.id));
  assert.equal(dependencyIds.has('sdkwork-utils'), true);

  const contractCargo = read('crates/sdkwork-notary-case-contract/Cargo.toml');
  assert.match(contractCargo, /sdkwork-utils-rust/);

  const serviceCargo = read('crates/sdkwork-notary-case-service/Cargo.toml');
  assert.match(serviceCargo, /sdkwork-utils-rust/);

  const timeSource = read('crates/sdkwork-notary-case-contract/src/time.rs');
  assert.match(timeSource, /sdkwork_utils_rust/);

  const serviceSource = read('crates/sdkwork-notary-case-service/src/service.rs');
  assert.match(serviceSource, /sdkwork_utils_rust::\{[^}]*\bis_blank\b/);
  assert.match(serviceSource, /PageInfo/);
  assert.doesNotMatch(serviceSource, /\.trim\(\)\.is_empty\(\)/);

  const piiVaultSource = read('crates/sdkwork-notary-case-repository-sqlx/src/pii_vault.rs');
  assert.match(piiVaultSource, /sha256_hash/);
  assert.match(piiVaultSource, /base64_encode/);
});

test('declares PNPM_SCRIPT_SPEC required root commands', () => {
  const packageManifest = readJson('package.json');
  for (const script of ['dev', 'build', 'test', 'check', 'verify', 'clean']) {
    assert.ok(packageManifest.scripts?.[script], `package.json must expose pnpm ${script}`);
  }
});

test('does not declare sdkwork-discovery without RPC services', () => {
  const rootCargo = read('Cargo.toml');
  assert.doesNotMatch(rootCargo, /sdkwork-discovery/);
  assert.equal(exists('apis/rpc'), false, 'RPC contracts should not exist yet');
});

test('route manifests declare WebRequestContext and apiSurface on every route', () => {
  for (const { file, apiSurface } of ROUTE_MANIFESTS) {
    const manifest = readJson(file);
    assert.ok(Array.isArray(manifest.routes) && manifest.routes.length > 0, `${file} should declare routes`);
    for (const route of manifest.routes) {
      assert.equal(route.requestContext, 'WebRequestContext', `${route.method} ${route.path} missing requestContext`);
      assert.equal(route.apiSurface, apiSurface, `${route.method} ${route.path} missing apiSurface`);
      assert.ok(route.handler?.name, `${route.operationId} missing handler.name`);
      assert.ok(route.auth?.mode, `${route.operationId} missing auth.mode`);
    }
  }
});

test('OpenAPI authorities declare x-sdkwork-request-context and x-sdkwork-api-surface', () => {
  const expectations = [
    { file: 'apis/app-api/notary/notary-app-api.openapi.json', apiSurface: 'app-api' },
    { file: 'apis/backend-api/notary/notary-backend-api.openapi.json', apiSurface: 'backend-api' },
    { file: 'generated/openapi/notary-app-api.openapi.json', apiSurface: 'app-api' },
    { file: 'generated/openapi/notary-backend-api.openapi.json', apiSurface: 'backend-api' },
  ];

  const methods = new Set(['get', 'post', 'put', 'patch', 'delete']);

  for (const { file, apiSurface } of expectations) {
    const openapi = readJson(file);
    let operationCount = 0;
    for (const pathItem of Object.values(openapi.paths ?? {})) {
      for (const [method, operation] of Object.entries(pathItem ?? {})) {
        if (!methods.has(method)) {
          continue;
        }
        operationCount += 1;
        assert.equal(
          operation['x-sdkwork-request-context'],
          'WebRequestContext',
          `${file} ${method} missing x-sdkwork-request-context`,
        );
        assert.equal(
          operation['x-sdkwork-api-surface'],
          apiSurface,
          `${file} ${method} missing x-sdkwork-api-surface`,
        );
        assert.equal(
          operation['x-sdkwork-auth-mode'],
          'dual-token',
          `${file} ${operation.operationId ?? method} missing x-sdkwork-auth-mode`,
        );
        assert.ok(
          operation['x-sdkwork-source-route-crate'],
          `${file} ${operation.operationId ?? method} missing x-sdkwork-source-route-crate`,
        );
        assert.ok(
          operation['x-sdkwork-source'],
          `${file} ${operation.operationId ?? method} missing x-sdkwork-source`,
        );
      }
    }
    assert.ok(operationCount > 0, `${file} should contain HTTP operations`);
  }
});

test('declares topology profiles for host application wiring', () => {
  assert.equal(exists('specs/topology.spec.json'), true);

  const topology = readJson('specs/topology.spec.json');
  for (const profileFile of Object.values(topology.profileFiles ?? {})) {
    assert.equal(exists(profileFile), true, `${profileFile} should exist`);
  }

  assert.equal(topology.database?.appPrefix, 'SDKWORK_NOTARY');
  assert.equal(topology.schemaVersion, 4);
  assert.equal(topology.defaults?.developmentProfileId, 'standalone.development');
  assert.deepEqual(topology.surfaces?.['application.public-ingress']?.protocols, ['http']);
});

test('declares SDKWORK workspace config and test skeleton directories', () => {
  for (const directory of [
    'configs/schemas',
    'configs/profiles',
    'configs/examples',
    'configs/defaults',
    'tests/contract',
    'tests/integration',
    'tests/e2e',
    'tests/fixtures',
    'tests/static',
  ]) {
    assert.equal(exists(directory), true, `${directory}/ should exist`);
    assert.equal(exists(path.join(directory, 'README.md')), true, `${directory}/README.md should exist`);
  }
});

test('domain library root declares agent.app.config.json', () => {
  assert.equal(exists('agent.app.config.json'), true);

  const manifest = readJson('agent.app.config.json');
  assert.equal(manifest.schemaVersion, 3);
  assert.equal(manifest.kind, 'agent.app');
  assert.equal(manifest.app.key, 'sdkwork-notary');
  assert.equal(manifest.runtime.family, 'library');
  assert.equal(manifest.devApp.sourceRoot, 'apps/sdkwork-notary-h5');
});

test('Rust HTTP crates follow sdkwork-routes-notary-* naming', () => {
  const expectedMembers = [
    'crates/sdkwork-routes-notary-app-api',
    'crates/sdkwork-routes-notary-backend-api',
    'crates/sdkwork-routes-notary-http-auth',
    'crates/sdkwork-notary-case-contract',
    'crates/sdkwork-notary-case-service',
    'crates/sdkwork-notary-case-repository-sqlx',
    'crates/sdkwork-notary-database-host',
  ];

  const cargo = read('Cargo.toml');
  for (const member of expectedMembers) {
    assert.match(cargo, new RegExp(`"${member.replaceAll('/', '\\/')}"`));
  }
});

test('route crate component specs reference canonical route manifest files', () => {
  assert.equal(
    readJson('crates/sdkwork-routes-notary-app-api/specs/component.spec.json').contracts.routeManifest,
    '../../../sdks/_route-manifests/app-api/sdkwork-routes-notary-app-api.route-manifest.json',
  );
  assert.equal(
    readJson('crates/sdkwork-routes-notary-backend-api/specs/component.spec.json').contracts.routeManifest,
    '../../../sdks/_route-manifests/backend-api/sdkwork-routes-notary-backend-api.route-manifest.json',
  );
});

test('declares IMF module manifest for notary permissions', () => {
  assert.equal(exists('specs/iam.module.manifest.json'), true);
  const manifest = readJson('specs/iam.module.manifest.json');
  assert.equal(manifest.moduleId, 'notary');
  assert.equal(manifest.owner, 'sdkwork-notary');
  assert.ok(Array.isArray(manifest.permissions?.catalog));
  assert.ok(manifest.permissions.catalog.length > 0);
});

test('declares IMF registry config with notary module enabled', () => {
  assert.equal(exists('iam/registry/iam-registry.config.json'), true);
  const registry = readJson('iam/registry/iam-registry.config.json');
  assert.equal(registry.kind, 'sdkwork.iam.registry.config');
  assert.ok(registry.enabledModules.includes('notary'));
});

test('production topology profiles document PII vault secret injection', () => {
  for (const profile of [
    'etc/topology/standalone.production.env',
    'etc/topology/cloud.production.env',
  ]) {
    const env = read(profile);
    assert.match(env, /NOTARY_PII_VAULT_KEY/);
  }
});

test('declares domain library root component spec', () => {
  assert.equal(exists('specs/component.spec.json'), true);
  const spec = readJson('specs/component.spec.json');
  assert.equal(spec.component.name, 'sdkwork-notary');
  assert.equal(spec.component.type, 'domain-library');
  const files = spec.canonicalSpecs.map((entry) => entry.file);
  assert.ok(files.includes('SECURITY_SPEC.md'));
  assert.ok(files.includes('IAM_MODULE_MANIFEST_SPEC.md'));
});

test('database seed manifest wires bootstrap common seed', () => {
  const seeds = readJson('database/seeds/seed.manifest.json');
  assert.ok(seeds.profiles.standard.common.includes('001_bootstrap.sql'));
});

test('route and service component specs reference security and observability standards', () => {
  const observabilityRequired = new Set([
    'sdkwork-routes-notary-app-api',
    'sdkwork-routes-notary-backend-api',
    'sdkwork-notary-case-service',
  ]);

  for (const specPath of [
    'crates/sdkwork-routes-notary-app-api/specs/component.spec.json',
    'crates/sdkwork-routes-notary-backend-api/specs/component.spec.json',
    'crates/sdkwork-routes-notary-http-auth/specs/component.spec.json',
    'crates/sdkwork-notary-case-service/specs/component.spec.json',
    'crates/sdkwork-notary-case-contract/specs/component.spec.json',
    'crates/sdkwork-notary-case-repository-sqlx/specs/component.spec.json',
    'crates/sdkwork-notary-database-host/specs/component.spec.json',
    'sdks/sdkwork-notary-app-sdk/specs/component.spec.json',
    'sdks/sdkwork-notary-backend-sdk/specs/component.spec.json',
  ]) {
    const spec = readJson(specPath);
    const files = spec.canonicalSpecs.map((entry) => entry.file);
    assert.ok(files.includes('SECURITY_SPEC.md'), `${specPath} must cite SECURITY_SPEC.md`);
    if (observabilityRequired.has(spec.component.name)) {
      assert.ok(
        files.includes('OBSERVABILITY_SPEC.md'),
        `${specPath} must cite OBSERVABILITY_SPEC.md`,
      );
    }
  }
});

test('notary case service declares owner ports instead of direct dependency SDK clients', () => {
  const spec = readJson('crates/sdkwork-notary-case-service/specs/component.spec.json');
  const requiredPorts = new Set(spec.contracts.requiredPorts.map((port) => port.name));

  assert.deepEqual(spec.contracts.sdkClients, []);
  assert.equal(requiredPorts.has('commerce.merchandise'), true);
  assert.equal(requiredPorts.has('commerce.order'), true);
  assert.equal(requiredPorts.has('commerce.payment'), false);

  const serviceCargo = read('crates/sdkwork-notary-case-service/Cargo.toml');
  assert.doesNotMatch(serviceCargo, /sdkwork-(?:catalog|merchandise|order|payment).*sdk/u);
});

test('H5 AuthGate redirects unauthenticated production sessions to platform IAM login', () => {
  const authGate = read('apps/sdkwork-notary-h5/src/AuthGate.tsx');
  assert.match(authGate, /import\.meta\.env\.PROD/);
  assert.match(authGate, /\/auth\/login\?redirect=/);
});

test('PC AuthGate mounts sdkwork-iam login routes for unauthenticated sessions', () => {
  const authGate = read('apps/sdkwork-notary-pc/src/AuthGate.tsx');
  assert.match(authGate, /SdkworkIamAuthRoutes/u);
  assert.match(authGate, /getNotaryPcIamRuntime/u);
  assert.match(authGate, /AUTH_BASE_PATH = '\/auth'/u);
  assert.match(authGate, /\/login\?/u);
  assert.match(authGate, /LoadingState/u);
  assert.match(authGate, /notaryPcAuthService\.getCurrentSession/u);
});

test('notary runtime enforces operation permission map and production membership guard', () => {
  const permissions = read('crates/sdkwork-notary-case-service/src/permissions.rs');
  assert.match(permissions, /require_operation_permission/);
  assert.match(permissions, /OPERATION_PERMISSIONS/);

  const context = read('crates/sdkwork-routes-notary-http-auth/src/context.rs');
  assert.match(context, /allows_dev_synthetic_membership/);
  assert.match(context, /permission_scopes/);
  assert.match(context, /extract_permission_scopes/);
});

test('declares contract maintenance scripts and schema registry', () => {
  const packageManifest = readJson('package.json');
  assert.equal(
    packageManifest.scripts['api:materialize'],
    'node scripts/sync-notary-openapi-authorities.mjs && node scripts/generate-notary-route-manifests.mjs && node scripts/sync-notary-api-framework-metadata.mjs',
  );
  assert.ok(packageManifest.scripts['dev:browser'], 'package.json must expose pnpm dev:browser');
  assert.ok(packageManifest.scripts['api:check'], 'package.json must expose pnpm api:check');
  assert.ok(packageManifest.scripts['sdk:check'], 'package.json must expose pnpm sdk:check');
  assert.ok(packageManifest.scripts['topology:plan'], 'package.json must expose pnpm topology:plan');
  assert.ok(packageManifest.scripts.typecheck, 'package.json must expose pnpm typecheck');

  assert.equal(exists('specs/README.md'), true);
  assert.equal(exists('generated/README.md'), true);
  assert.equal(exists('generated/openapi/README.md'), true);
  assert.equal(exists('docs/schema-registry/README.md'), true);
  assert.equal(exists('docs/schema-registry/tables/001-notary-core.yaml'), true);
});
