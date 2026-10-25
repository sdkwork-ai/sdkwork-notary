import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';

const ROOT = process.cwd();

function read(relativePath) {
  return readFileSync(path.join(ROOT, relativePath), 'utf8');
}

function readJson(relativePath) {
  return JSON.parse(read(relativePath));
}

test('declares sdkwork-utils workspace dependency and release checkout', () => {
  const rootCargo = read('Cargo.toml');
  assert.match(rootCargo, /sdkwork-utils-rust = \{/);

  const workflow = readJson('agent.workflow.json');
  const dependencyIds = new Set((workflow.dependencies ?? []).map((dependency) => dependency.id));
  assert.equal(dependencyIds.has('sdkwork-utils'), true);
  assert.equal(dependencyIds.has('sdkwork-app-topology'), true);
});

test('service and contract validation use sdkwork-utils-rust is_blank', () => {
  const serviceSource = read('crates/sdkwork-notary-case-service/src/service.rs');
  assert.match(serviceSource, /sdkwork_utils_rust::\{[^}]*\bis_blank\b/);
  assert.match(serviceSource, /is_blank\(Some\(context\.tenant_id\.as_str\(\)\)\)/);
  assert.match(serviceSource, /add_minutes/);
  assert.doesNotMatch(serviceSource, /\.trim\(\)\.is_empty\(\)/);
  assert.doesNotMatch(serviceSource, /use chrono::/);

  const contractSource = read('crates/sdkwork-notary-case-contract/src/service_contract.rs');
  assert.match(contractSource, /use sdkwork_utils_rust::is_blank;/);
  assert.match(contractSource, /is_blank\(Some\(self\.domain\)\)/);

  const serviceCargo = read('crates/sdkwork-notary-case-service/Cargo.toml');
  assert.doesNotMatch(serviceCargo, /^chrono = /m);
});

test('repository PII vault uses sdkwork-utils-rust encoding and hashing', () => {
  const piiVault = read('crates/sdkwork-notary-case-repository-sqlx/src/pii_vault.rs');
  assert.match(piiVault, /base64_encode/);
  assert.match(piiVault, /base64_decode/);
  assert.match(piiVault, /sha256_hash/);
  assert.doesNotMatch(piiVault, /use base64::/);
  assert.doesNotMatch(piiVault, /use sha2::/);

  const repositoryCargo = read('crates/sdkwork-notary-case-repository-sqlx/Cargo.toml');
  assert.doesNotMatch(repositoryCargo, /^base64 = /m);
  assert.doesNotMatch(repositoryCargo, /^sha2 = /m);
});
