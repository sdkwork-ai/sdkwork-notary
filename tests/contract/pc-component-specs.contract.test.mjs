#!/usr/bin/env node

import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');

const packages = [
  'sdkwork-notary-pc-core',
  'sdkwork-notary-pc-commons',
  'sdkwork-notary-pc-shell',
  'sdkwork-notary-pc-notary',
];

function readJson(relativePath) {
  return JSON.parse(readFileSync(path.join(repoRoot, relativePath), 'utf8'));
}

for (const packageName of packages) {
  test(`${packageName} declares component.spec.json`, () => {
    const specPath = path.join(
      repoRoot,
      'apps/sdkwork-notary-pc/packages',
      packageName,
      'specs/component.spec.json',
    );
    assert.equal(existsSync(specPath), true, `${packageName} must declare specs/component.spec.json`);
    const spec = JSON.parse(readFileSync(specPath, 'utf8'));
    assert.equal(spec.kind, 'sdkwork.component.spec');
    assert.equal(spec.component.name, packageName);
    assert.equal(spec.component.surface, 'pc');
    assert.ok(Array.isArray(spec.canonicalSpecs) && spec.canonicalSpecs.length > 0);
  });
}

test('PC packages use workspace protocol for internal dependencies', () => {
  for (const packageName of packages) {
    const manifest = readJson(`apps/sdkwork-notary-pc/packages/${packageName}/package.json`);
    for (const [dependencyName, version] of Object.entries(manifest.dependencies ?? {})) {
      if (!dependencyName.startsWith('@sdkwork/notary-pc-')) {
        continue;
      }
      assert.equal(version, 'workspace:*', `${packageName} must use workspace:* for ${dependencyName}`);
    }
  }
});

test('PC manifest describes the desktop client identity', () => {
  const manifest = readJson('apps/sdkwork-notary-pc/agent.app.config.json');
  assert.equal(manifest.app.key, 'sdkwork-notary-pc');
  assert.equal(manifest.runtime.family, 'pc');
  assert.equal(manifest.runtime.framework, 'react-pc');
  assert.equal(manifest.devApp.sourceRoot, 'apps/sdkwork-notary-pc');
});
