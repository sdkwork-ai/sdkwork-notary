#!/usr/bin/env node

import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');

function readJson(relativePath) {
  return JSON.parse(readFileSync(path.join(repoRoot, relativePath), 'utf8'));
}

test('root manifest describes the domain library identity', () => {
  const manifest = readJson('agent.app.config.json');
  assert.equal(manifest.schemaVersion, 3);
  assert.equal(manifest.kind, 'agent.app');
  assert.equal(manifest.app.key, 'sdkwork-notary');
  assert.equal(manifest.runtime.family, 'library');
  assert.equal(manifest.devApp.sourceRoot, 'apps/sdkwork-notary-h5');
});

test('H5 manifest describes the mobile client identity', () => {
  const manifest = readJson('apps/sdkwork-notary-h5/agent.app.config.json');
  assert.equal(manifest.schemaVersion, 3);
  assert.equal(manifest.kind, 'agent.app');
  assert.equal(manifest.app.key, 'sdkwork-notary-h5');
  assert.equal(manifest.runtime.family, 'mobile');
  assert.equal(manifest.runtime.framework, 'react-h5');
  assert.ok(manifest.environments.staging, 'H5 manifest must declare staging environment');
  assert.ok(manifest.environments.test, 'H5 manifest must declare test environment');
});

test('workflow config points at the root manifest and declares topology dependency', () => {
  const workflow = readJson('agent.workflow.json');
  assert.equal(workflow.app.configPath, 'agent.app.config.json');
  const dependencyIds = new Set((workflow.dependencies ?? []).map((dependency) => dependency.id));
  assert.equal(dependencyIds.has('sdkwork-app-topology'), true);
});

test('GitHub package workflow passes topology dependency ref', () => {
  const workflow = readFileSync(path.join(repoRoot, '.github/workflows/package.yml'), 'utf8');
  assert.match(workflow, /sdkwork_app_topology_ref/u);
  assert.match(workflow, /SDKWORK_APP_TOPOLOGY_REF/u);
});
