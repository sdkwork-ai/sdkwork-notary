# SDKWork Notary PC

Application: `sdkwork-notary-pc`
Runtime: PC browser renderer
Status: active
Manifest: `agent.app.config.json`

## Surfaces

| Surface | Route | Package family | SDK |
| --- | --- | --- | --- |
| App/user | `/notary` | `sdkwork-notary-pc-core`, `sdkwork-notary-pc-shell`, `sdkwork-notary-pc-notary` | `@sdkwork/notary-app-sdk` |
| Internal admin | `/admin` | `sdkwork-notary-pc-admin-core`, `sdkwork-notary-pc-admin-shell`, `sdkwork-notary-pc-admin-merchandise` | `@sdkwork/notary-backend-sdk` |

The shared AuthGate establishes the session. App SDK clients are constructed through PC core; backend SDK clients are constructed only through PC admin core. Feature packages use injected clients and never raw HTTP or manual authentication headers.

## Matter Management

The first backend-admin capability is Matter Management at `/admin/notary/matters`. It supports server-paginated search and lifecycle filtering, create, edit, activation, and deactivation. The UI calls Notary backend orchestration; Merchandise remains the SPU/SKU owner, and Order/Payment are not direct frontend dependencies.

## Runtime Configuration

The renderer resolves separate app and backend roots from `VITE_SDKWORK_NOTARY_APPLICATION_PUBLIC_HTTP_URL` and `VITE_SDKWORK_NOTARY_APPLICATION_BACKEND_HTTP_URL`. A shared gateway may satisfy both through `VITE_SDKWORK_NOTARY_PLATFORM_API_GATEWAY_HTTP_URL`. Production startup fails closed when either required surface is unresolved.

## Documentation And Contracts

- [PC product PRD](docs/product/prd/PRD.md)
- [PC technical architecture](docs/architecture/tech/TECH_ARCHITECTURE.md)
- [PC local specs index](specs/README.md)
- Package contracts: `packages/*/specs/component.spec.json`
- Global standards: `../../../sdkwork-specs/README.md`

## Commands

```text
pnpm dev
pnpm typecheck
pnpm test
pnpm build
```

Repository-level verification:

```text
pnpm test:desktop
pnpm build:desktop
pnpm verify
```
