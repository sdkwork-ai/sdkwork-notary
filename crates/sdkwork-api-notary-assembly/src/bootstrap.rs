//! Host-neutral API assembly bootstrap for SDKWork Notary.

use axum::Router;
use sdkwork_database_sqlx::DatabasePool;
use sdkwork_notary_embedded_bootstrap::EmbeddedNotaryAssembly;
use sdkwork_web_bootstrap::{ApiAssemblyContribution, ReadinessCheck, ReadinessFuture, WebModule};
use sdkwork_web_core::HttpRouteManifest;
use std::sync::Arc;

pub type ApiAssembly = ApiAssemblyContribution;

#[derive(Clone)]
struct EmbeddedNotaryReadiness {
    assembly: Arc<EmbeddedNotaryAssembly>,
}

impl ReadinessCheck for EmbeddedNotaryReadiness {
    fn check(&self) -> ReadinessFuture<'_> {
        let assembly = self.assembly.clone();
        Box::pin(async move {
            for pool in assembly.database_pools() {
                match pool.test_connection().await {
                    Ok(true) => {}
                    Ok(false) => {
                        return Err("notary database readiness query returned no row".to_owned());
                    }
                    Err(error) => {
                        return Err(format!("notary database readiness check failed: {error}"));
                    }
                }
            }
            Ok(())
        })
    }
}

pub async fn assemble_api_router() -> Result<ApiAssembly, String> {
    let embedded = Arc::new(
        sdkwork_notary_embedded_bootstrap::assemble_embedded_notary_application_router_from_env()
            .await?,
    );
    let mut routes = Vec::new();
    routes.extend_from_slice(sdkwork_routes_notary_app_api::gateway_route_manifest().routes());
    routes.extend_from_slice(sdkwork_routes_notary_backend_api::gateway_route_manifest().routes());
    build_contribution(
        "SDKWork Notary API",
        embedded.router.clone(),
        HttpRouteManifest::from_owned_routes(routes),
        embedded,
    )
}

/// Assemble the Notary router against a caller-provided database pool so the
/// platform cloud gateway can share its process-wide PostgreSQL pool.
pub async fn assemble_api_router_with_pool(pool: DatabasePool) -> Result<ApiAssembly, String> {
    let embedded = Arc::new(
        sdkwork_notary_embedded_bootstrap::assemble_embedded_notary_application_router_with_pool(
            pool,
        )
        .await?,
    );
    let mut routes = Vec::new();
    routes.extend_from_slice(sdkwork_routes_notary_app_api::gateway_route_manifest().routes());
    routes.extend_from_slice(sdkwork_routes_notary_backend_api::gateway_route_manifest().routes());
    build_contribution(
        "SDKWork Notary API",
        embedded.router.clone(),
        HttpRouteManifest::from_owned_routes(routes),
        embedded,
    )
}

pub async fn assemble_app_api_contribution() -> Result<ApiAssembly, String> {
    let embedded = Arc::new(
        sdkwork_notary_embedded_bootstrap::assemble_embedded_notary_application_router_from_env()
            .await?,
    );
    build_contribution(
        "SDKWork Notary App API",
        embedded.app_router.clone(),
        sdkwork_routes_notary_app_api::gateway_route_manifest(),
        embedded,
    )
}

fn build_contribution(
    title: &str,
    router: Router,
    route_manifest: HttpRouteManifest,
    embedded: Arc<EmbeddedNotaryAssembly>,
) -> Result<ApiAssembly, String> {
    ApiAssemblyContribution::from_manifest(
        "sdkwork-notary",
        title,
        router,
        route_manifest,
        Vec::new(),
        Arc::new(EmbeddedNotaryReadiness { assembly: embedded }),
    )
}

/// Canonical Web Module definition for this application
/// (API_ASSEMBLY_SPEC §4.1.1): the complete HTTP surface — every route,
/// manifest, and OpenAPI document of this owner — as one installable module.
pub async fn web_module() -> Result<WebModule, String> {
    Ok(WebModule::from_contribution(assemble_api_router().await?))
}

/// Same as [`web_module`] but composed on a process-shared database pool
/// (platform gateways, API_ASSEMBLY_SPEC §4.1.1).
pub async fn web_module_with_pool(pool: DatabasePool) -> Result<WebModule, String> {
    Ok(WebModule::from_contribution(
        assemble_api_router_with_pool(pool).await?,
    ))
}
