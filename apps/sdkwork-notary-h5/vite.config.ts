import { resolveViteEnvironment, resolveLucideReactEntry } from '../../../sdkwork-specs/tools/vite-runtime-profile.mjs';
import { resolveBrowserDistOutDir } from '../../../sdkwork-specs/tools/browser-dist-layout.mjs';

import path from 'node:path';
import { fileURLToPath } from 'node:url';
import react from '@vitejs/plugin-react';
import { defineConfig, loadEnv } from 'vite';

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

  return {
    build: {
      outDir: resolveBrowserDistOutDir(resolveViteEnvironment(mode, process.env)),
      emptyOutDir: true,
    },
    plugins: [react()],
    define: {
      'process.env.SDKWORK_ACCESS_TOKEN': JSON.stringify(env.SDKWORK_ACCESS_TOKEN ?? ''),
    },
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
