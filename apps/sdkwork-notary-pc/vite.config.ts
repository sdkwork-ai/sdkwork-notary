import { resolveViteEnvironment, resolveLucideReactEntry } from '../../../sdkwork-specs/tools/vite-runtime-profile.mjs';
import { resolveBrowserDistOutDir } from '../../../sdkwork-specs/tools/browser-dist-layout.mjs';

import path from 'node:path';
import { fileURLToPath } from 'node:url';
import react from '@vitejs/plugin-react';
import { defineConfig, loadEnv } from 'vite';

const pcRoot = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(pcRoot, '../..');
const workspaceRoot = path.resolve(repoRoot, '..');
const appbaseRoot = path.resolve(workspaceRoot, 'sdkwork-appbase');
const iamRoot = path.resolve(workspaceRoot, 'sdkwork-iam');
const uiRoot = path.resolve(workspaceRoot, 'sdkwork-ui');
const coreRoot = path.resolve(workspaceRoot, 'sdkwork-core');
const sdkCommonRoot = path.resolve(workspaceRoot, 'sdkwork-sdk-commons/sdkwork-sdk-common-typescript/src');

const generatedDriveAppSdkEntry = path.resolve(
  workspaceRoot,
  'sdkwork-drive/sdks/sdkwork-drive-app-sdk/sdkwork-drive-app-sdk-typescript/src/index.ts',
);
const generatedAppbaseAppSdkEntry = path.resolve(
  iamRoot,
  'sdks/sdkwork-iam-app-sdk/sdkwork-iam-app-sdk-typescript/src/index.ts',
);

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, pcRoot, '');

  return {
    plugins: [react()],
    define: {
      'process.env.SDKWORK_ACCESS_TOKEN': JSON.stringify(env.SDKWORK_ACCESS_TOKEN ?? ''),
    },
    resolve: {
      alias: [
        { find: '@sdkwork/notary-pc-core', replacement: path.resolve(pcRoot, 'packages/sdkwork-notary-pc-core/src/index.ts') },
        { find: '@sdkwork/notary-pc-commons', replacement: path.resolve(pcRoot, 'packages/sdkwork-notary-pc-commons/src/index.ts') },
        { find: '@sdkwork/notary-pc-admin-core', replacement: path.resolve(pcRoot, 'packages/sdkwork-notary-pc-admin-core/src/index.ts') },
        { find: '@sdkwork/notary-pc-admin-shell', replacement: path.resolve(pcRoot, 'packages/sdkwork-notary-pc-admin-shell/src/index.ts') },
        { find: '@sdkwork/notary-pc-admin-merchandise', replacement: path.resolve(pcRoot, 'packages/sdkwork-notary-pc-admin-merchandise/src/index.ts') },
        { find: '@sdkwork/notary-pc-shell', replacement: path.resolve(pcRoot, 'packages/sdkwork-notary-pc-shell/src/index.ts') },
        { find: '@sdkwork/notary-pc-notary', replacement: path.resolve(pcRoot, 'packages/sdkwork-notary-pc-notary/src/index.ts') },
        {
          find: '@sdkwork/notary-app-sdk',
          replacement: path.resolve(
            repoRoot,
            'sdks/sdkwork-notary-app-sdk/sdkwork-notary-app-sdk-typescript/src/index.ts',
          ),
        },
        {
          find: '@sdkwork/notary-backend-sdk',
          replacement: path.resolve(
            repoRoot,
            'sdks/sdkwork-notary-backend-sdk/sdkwork-notary-backend-sdk-typescript/src/index.ts',
          ),
        },
        { find: '@sdkwork/drive-app-sdk', replacement: generatedDriveAppSdkEntry },
        { find: '@sdkwork/iam-app-sdk', replacement: generatedAppbaseAppSdkEntry },
        { find: '@sdkwork/auth-pc-react', replacement: path.resolve(iamRoot, 'apps/sdkwork-iam-pc/packages/sdkwork-auth-pc-react/src/index.ts') },
        { find: '@sdkwork/auth-runtime-pc-react', replacement: path.resolve(iamRoot, 'apps/sdkwork-iam-pc/packages/sdkwork-auth-runtime-pc-react/src/index.ts') },
        { find: '@sdkwork/iam-runtime', replacement: path.resolve(iamRoot, 'apps/sdkwork-iam-common/packages/sdkwork-iam-runtime/src/index.ts') },
        { find: '@sdkwork/iam-contracts', replacement: path.resolve(iamRoot, 'apps/sdkwork-iam-common/packages/sdkwork-iam-contracts/src/index.ts') },
        { find: '@sdkwork/iam-sdk-ports', replacement: path.resolve(iamRoot, 'apps/sdkwork-iam-common/packages/sdkwork-iam-sdk-ports/src/index.ts') },
        { find: '@sdkwork/iam-sdk-adapter', replacement: path.resolve(iamRoot, 'apps/sdkwork-iam-common/packages/sdkwork-iam-sdk-adapter/src/index.ts') },
        { find: '@sdkwork/appbase-pc-react', replacement: path.resolve(appbaseRoot, 'packages/pc-react/foundation/sdkwork-appbase-pc-react/src/index.ts') },
        { find: '@sdkwork/i18n-pc-react', replacement: path.resolve(appbaseRoot, 'packages/pc-react/foundation/sdkwork-i18n-pc-react/src/index.ts') },
        { find: '@sdkwork/core-pc-react', replacement: path.resolve(coreRoot, 'sdkwork-core-pc-react/src') },
        { find: '@sdkwork/ui-pc-react', replacement: path.resolve(uiRoot, 'sdkwork-ui-pc-react/src/index.ts') },
        { find: '@sdkwork/sdk-common', replacement: path.resolve(sdkCommonRoot, 'index.ts') },
      ],
    },
    optimizeDeps: {
      exclude: [
        '@sdkwork/notary-app-sdk',
        '@sdkwork/notary-backend-sdk',
        '@sdkwork/drive-app-sdk',
        '@sdkwork/iam-app-sdk',
        '@sdkwork/auth-pc-react',
        '@sdkwork/auth-runtime-pc-react',
        '@sdkwork/iam-runtime',
        '@sdkwork/iam-contracts',
        '@sdkwork/iam-sdk-ports',
        '@sdkwork/iam-sdk-adapter',
        '@sdkwork/appbase-pc-react',
        '@sdkwork/i18n-pc-react',
        '@sdkwork/core-pc-react',
        '@sdkwork/ui-pc-react',
        '@sdkwork/sdk-common',
      ],
    },
    build: {
      outDir: resolveBrowserDistOutDir(resolveViteEnvironment(mode, process.env)),
      rollupOptions: {
        output: {
          manualChunks(id) {
            if (
              id.includes('node_modules/react/')
              || id.includes('node_modules/react-dom/')
              || id.includes('node_modules/react-router')
            ) {
              return 'react-vendor';
            }

            if (
              id.includes('node_modules/motion/')
              || id.includes('node_modules/i18next/')
              || id.includes('node_modules/react-i18next/')
            ) {
              return 'i18n-motion-vendor';
            }

            if (id.includes('/sdkwork-auth-pc-react/') || id.includes('/sdkwork-iam-runtime/')) {
              return 'auth-vendor';
            }
          },
        },
      },
    },
    server: {
      port: 5285,
    },
  };
});
