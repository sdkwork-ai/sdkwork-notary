import { resolveViteEnvironment, resolveLucideReactEntry } from '../../../sdkwork-specs/tools/vite-runtime-profile.mjs';
import { resolveBrowserDistOutDir } from '../../../sdkwork-specs/tools/browser-dist-layout.mjs';

import path from 'node:path';
import { fileURLToPath } from 'node:url';
import react from '@vitejs/plugin-react';
import { defineConfig, loadEnv } from 'vite';
import { createSdkworkCredentialEntryBootstrapVitePlugin } from '@sdkwork/iam-credential-entry/vite';

const h5Root = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(h5Root, '../..');
const workspaceRoot = path.resolve(repoRoot, '..');
const sdkCommonRoot = path.resolve(
  workspaceRoot,
  'sdkwork-sdk-commons/sdkwork-sdk-common-typescript/src',
);
const generatedDriveAppSdkEntry = path.resolve(
  workspaceRoot,
  'sdkwork-drive/sdks/sdkwork-drive-app-sdk/sdkwork-drive-app-sdk-typescript/src/index.ts',
);
const generatedAppbaseAppSdkEntry = path.resolve(
  workspaceRoot,
  'sdkwork-iam/sdks/sdkwork-iam-app-sdk/sdkwork-iam-app-sdk-typescript/src/index.ts',
);

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, h5Root, '');

  const bootstrapAccessToken = env.SDKWORK_ACCESS_TOKEN ?? process.env.SDKWORK_ACCESS_TOKEN;
  return {
    build: {
      outDir: resolveBrowserDistOutDir(resolveViteEnvironment(mode, process.env)),
      emptyOutDir: true,
    },
    plugins: [
      // The bootstrap credential reaches the renderer only through the shared IAM
      // plugin (dev-server HTML injection as
      // `globalThis.__SDKWORK_CREDENTIAL_ENTRY_BOOTSTRAP_ACCESS_TOKEN__`).
      // `define['process.env.SDKWORK_ACCESS_TOKEN']` is NOT a valid handoff
      // (IAM_CREDENTIAL_ENTRY_SPEC.md section 4/5).
      createSdkworkCredentialEntryBootstrapVitePlugin({
        accessToken: bootstrapAccessToken,
        environment: resolveViteEnvironment(mode, process.env),
      }),
      react(),
    ],
    resolve: {
      alias: {
      },
    },
    optimizeDeps: {
      exclude: [
        '@sdkwork/notary-app-sdk',
        '@sdkwork/drive-app-sdk',
        '@sdkwork/iam-app-sdk',
        '@sdkwork/sdk-common',
        '@sdkwork/utils',
      ],
    },
    server: {
      port: 5185,
    },
  };
});
