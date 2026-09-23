//! The notary runtime's commerce adapter.
//!
//! Orders are `sdkwork-order`'s; matters are `sdkwork-merchandise`'s catalog. The two are separate
//! capabilities with separate owners, so this file is where the notary's vocabulary meets the
//! catalog's — and the whole of its job is to keep that translation honest in both directions.
//!
//! # How a notary matter is told apart from everything else in the tenant's catalog
//!
//! The notary claims a **dedicated category** and marks its products `product_type = 'service'`. The
//! pair is the claim predicate: every list and every lookup in this file is narrowed by it, so a
//! notary matter can never be confused with a physical good that happens to be a service, and the
//! tenant's own merchandising work in other categories is untouched.
//!
//! The category is the notary's to create, because nothing else can be: it is a
//! `commerce_product_category` row in the merchandise module, `commerce_product_spu.category_id` is
//! `NOT NULL` with a foreign key, and `DATABASE_SPEC` keeps one module's seed out of another module's
//! tables. So the first matter in a tenant creates it, `uk_commerce_product_category_tenant_no` makes
//! that creation single, and the id is memoised for the life of the process.
//!
//! # Which row carries which fact
//!
//! A matter is one product with one price, so it is one SPU with one SKU, and the two rows are not
//! interchangeable:
//!
//! * the **SPU** carries the product-level facts — title and description — and it is what the
//!   listing filters and searches, because `ProductSpuListQuery` is the query that can narrow by
//!   category, product type, status and free text all at once;
//! * the **SKU** carries the sellable facts — price, currency, and the capability metadata the
//!   catalog has no column for (the matter's `spec`), which is exactly what
//!   `commerce_product_sku.metadata` is for.
//!
//! Status is the one fact both rows hold, and it is written in one direction: the SKU carries the
//! precise value and is written first, and the SPU's copy is the projection the list filter binds to,
//! derived from the variant afterwards. Writing the caller's instruction straight to both rows
//! instead would make the two agree only for as long as every future writer remembered to move both,
//! which is a weaker guarantee than a list filtered on one row needs.
//!
//! # Money
//!
//! The notary's contract speaks **major-denomination** decimal strings (`"600.00"`);
//! `commerce_product_sku` stores the **smallest unit** (`60000`) plus the `price_scale` it was
//! written at (`API_SPEC` section 13.2.1). Placing that point is done by the money kernel against the
//! `commerce_currency` row itself — never against a table in this file — because a divisor that
//! disagrees with the registry is silent: the row records one scale, the amount was read at another,
//! and the price is out by a factor of ten with nothing to indicate it.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use sdkwork_commerce_money::{Money, MoneyError, MoneyUnit};
use sdkwork_contract_service::CommerceServiceError;
use sdkwork_database_id::IdGenerator;
use sdkwork_database_sqlx::DatabasePool;
use sdkwork_merchandise_repository_sqlx::PostgresCommerceCatalogStore;
use sdkwork_merchandise_service::{
    CategoryListQuery, CategoryRecord, CreateCategoryCommand, CreateProductSkuCommand,
    CreateProductSpuCommand, FulfillmentType, GuardedWrite, InventoryTrackingMode,
    ProductSkuListQuery, ProductSkuRetrieveQuery, ProductSpuListQuery, ProductSpuRetrieveQuery,
    ProductStatus, ProductType, SkuRecord, SpuRecord, StaleVersion, UpdateProductSkuCommand,
    UpdateProductSpuCommand,
};
use sdkwork_notary_case_contract::NotaryServiceError;
use sdkwork_notary_case_service::{
    CommerceCreateOrderCommand, CommerceMatterCommand, CommerceMatterListPage,
    CommerceMatterListQuery, CommerceMatterRecord, CommerceMatterUpdateCommand,
    CommerceOrderFulfillmentState, CommerceOrderReference, CommercePort,
};
use sdkwork_order_repository_sqlx::PostgresCommerceOrderStore;
use sdkwork_order_service::{
    CancelOwnerOrderCommand, CheckoutLineInput, CreateCheckoutQuoteCommand,
    CreateCheckoutSessionCommand, CreateOwnerOrderCommand, OrderOwnerDetailQuery,
};
use sdkwork_utils_rust::sha256_hash;

/// The `category_no` of the category every notary matter is filed under.
///
/// `uk_commerce_product_category_tenant_no` is `(tenant_id, category_no)` while live, so this is one
/// category per tenant and one vocabulary for the whole notary business.
const NOTARY_CATEGORY_NO: &str = "NOTARY";

/// The category's display name, as the catalog console will show it.
const NOTARY_CATEGORY_NAME: &str = "Notary services";

/// The `organization_id` the notary category is created under.
///
/// `commerce_product_category.organization_id` is `NOT NULL DEFAULT 0` and the unique index above is
/// tenant-scoped, so a category that belonged to one organization could still only exist once per
/// tenant — the first organization to create a matter would silently own the second one's category.
/// Filing it at the tenant level is what makes the claim a property of the tenant rather than of a
/// race between its organizations. Products are still filed under the matter's own organization, so
/// the listing still narrows by organization.
const NOTARY_CATEGORY_ORGANIZATION_ID: &str = "0";

/// `commerce_product_spu.product_type` (`ck_commerce_product_spu_product_type`).
///
/// `service` and not a notary-specific token: the column's CHECK is a closed six-value vocabulary
/// this repository does not own, and a notary matter genuinely is a service — nothing is shipped.
/// The notary is distinguished by its category, not by inventing a product type.
const NOTARY_PRODUCT_TYPE: ProductType = ProductType::Service;

/// `commerce_product_sku.fulfillment_type` (`ck_commerce_product_sku_fulfillment_type`).
///
/// Same reasoning as the product type: `service` says "nothing is delivered, but the SKU is still
/// sellable", which is what a notarization is.
const NOTARY_FULFILLMENT_TYPE: FulfillmentType = FulfillmentType::NoDelivery;

/// `commerce_product_sku.inventory_tracking`.
///
/// A notarization is not a thing in a warehouse, so there is no count to run out of. This also
/// settles `inventory_policy`, which `ck_commerce_product_sku_policy_requires_tracking` forces to
/// `deny` for an untracked SKU.
const NOTARY_INVENTORY_TRACKING: InventoryTrackingMode = InventoryTrackingMode::Untracked;

/// The number of rows requested when probing for a matter by its business key or by its variant.
///
/// One row is the expected answer in both cases; asking for two is what turns "there is more than
/// there should be" into something observable rather than something assumed away.
const MATTER_PROBE_PAGE_SIZE: i64 = 2;

/// How many times an unversioned read-modify-write is re-read before the caller is told to retry.
///
/// Not backoff-and-jitter: the contended resource is one row's `version` counter, not a remote
/// service, so a retry is a re-read and a re-write inside the same request, and a delay would only
/// add latency to a request that already has a user waiting on it. The bound is what matters — it is
/// what keeps a row under permanent contention from becoming an unbounded loop.
const MATTER_WRITE_ATTEMPTS: usize = 3;

const NOTARY_ORDER_CANCEL_REASON: &str = "notary case creation compensation";

pub struct CommerceOrderPort {
    store: PostgresCommerceOrderStore,
    catalog: PostgresCommerceCatalogStore,
    tenant_id: String,
    owner_user_id: String,
    /// The notary category's id, resolved on first use and then held for the process's life.
    ///
    /// A `std::sync::Mutex` and not a `tokio` one, deliberately: the guard is taken to read or
    /// replace one `Option<String>` and is dropped before anything is awaited, so there is no lock
    /// held across an await to worry about. It is also why this is the only shared mutable state in
    /// the adapter — everything else it needs is derived per call.
    notary_category_id: Mutex<Option<String>>,
}

impl CommerceOrderPort {
    pub fn new(
        pool: DatabasePool,
        merchandise_id_generator: Arc<dyn IdGenerator>,
        tenant_id: impl Into<String>,
        owner_user_id: impl Into<String>,
    ) -> Result<Self, NotaryServiceError> {
        // Initialization state: sdkwork-order and sdkwork-merchandise are authoritative-server
        // PostgreSQL modules (no sqlite store); the notary embedded runtime must be handed a
        // PostgreSQL pool.
        let (store, catalog) = match &pool {
            DatabasePool::Sqlite(_, _) => {
                return Err(NotaryServiceError::provider_unavailable(
                    "sdkwork-order and sdkwork-merchandise are authoritative-server PostgreSQL modules; the notary embedded runtime requires a PostgreSQL commerce pool",
                ));
            }
            DatabasePool::Postgres(postgres_pool, _) => (
                PostgresCommerceOrderStore::new(postgres_pool.clone()),
                PostgresCommerceCatalogStore::new(postgres_pool.clone(), merchandise_id_generator),
            ),
        };
        Ok(Self {
            store,
            catalog,
            tenant_id: tenant_id.into(),
            owner_user_id: owner_user_id.into(),
            notary_category_id: Mutex::new(None),
        })
    }

    /// The id of this tenant's notary category, creating it if this is the first matter.
    ///
    /// # Why the id is memoised rather than re-read
    ///
    /// It is the same row for every matter in the tenant, and a category is not something a matter
    /// write can move. Memoising it costs one process's worth of staleness in a case that would take
    /// an operator deleting the category out from under a running runtime, and it saves a list query
    /// on every single matter write. Unlike the currency registry — where a stale unit misplaces a
    /// decimal point and is therefore never cached across requests — a stale category id fails
    /// loudly, because the row it names is gone and PostgreSQL says so.
    async fn notary_category_id(&self) -> Result<String, NotaryServiceError> {
        if let Some(cached) = self.cached_category_id() {
            return Ok(cached);
        }
        if let Some(found) = self.find_notary_category().await? {
            return Ok(self.remember_category_id(found.id));
        }

        let command = CreateCategoryCommand {
            tenant_id: self.tenant_id.clone(),
            organization_id: NOTARY_CATEGORY_ORGANIZATION_ID.to_owned(),
            category_no: NOTARY_CATEGORY_NO.to_owned(),
            parent_id: None,
            name: NOTARY_CATEGORY_NAME.to_owned(),
            sort_order: 0,
        };
        match self.catalog.create_category(&command).await {
            Ok(created) => Ok(self.remember_category_id(created.id)),
            // Another process claimed the category between the read and the write. That is the
            // outcome this call wanted, so it is re-read rather than reported: a conflict here would
            // be a lie about a state the caller is content with.
            Err(error) if error.code() == "conflict" => {
                let found = self.find_notary_category().await?.ok_or_else(|| {
                    NotaryServiceError::invalid_state(
                        "the notary category was reported as already taken but could not be read back",
                    )
                })?;
                Ok(self.remember_category_id(found.id))
            }
            Err(error) => Err(map_commerce_error(error)),
        }
    }

    fn cached_category_id(&self) -> Option<String> {
        self.notary_category_id
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn remember_category_id(&self, id: i64) -> String {
        let id = id.to_string();
        *self
            .notary_category_id
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(id.clone());
        id
    }

    /// Finds the tenant's notary category, if this tenant has one.
    ///
    /// `CategoryListQuery` has no `category_no` filter, so this walks the tenant's category pages.
    /// That is affordable because it is a cold path — the answer is memoised by the caller — and
    /// because a category tree is a merchandising artifact measured in tens of rows, not a table
    /// that grows with business volume.
    async fn find_notary_category(&self) -> Result<Option<CategoryRecord>, NotaryServiceError> {
        let mut page = 1_i64;
        loop {
            let records = self
                .catalog
                .list_categories(&CategoryListQuery {
                    tenant_id: self.tenant_id.clone(),
                    // Not filtered by organization: the category is tenant-level, and a filter here
                    // would look past the very row this method exists to find.
                    organization_id: None,
                    parent_id: None,
                    status: None,
                    page: Some(page),
                    page_size: Some(200),
                })
                .await
                .map_err(map_commerce_error)?;
            if let Some(found) = records
                .iter()
                .find(|record| record.category_no == NOTARY_CATEGORY_NO)
            {
                return Ok(Some(found.clone()));
            }
            if records.len() < 200 {
                return Ok(None);
            }
            page += 1;
        }
    }

    /// Finds a notary product by the business key its creation derived.
    ///
    /// # Why the lookup goes through the free-text filter
    ///
    /// `spu_no` is unique per tenant, but the catalog publishes no "read by `spu_no`" operation —
    /// `retrieve_spu` addresses a row by id, and the id is exactly what a retried create does not
    /// have. `ProductSpuListQuery.q` is the one filter that matches `spu_no`, so it is what is
    /// available; the derived key is hex, so it carries no `LIKE` wildcard and the `ILIKE` it builds
    /// is an exact match on the identifier. The row is then confirmed by comparing `spu_no` itself,
    /// which is what makes this a lookup of a known key rather than a search.
    ///
    /// The organization filter is applied here even though the key is tenant-scoped, because a
    /// retried create that names a different organization must not silently adopt the first
    /// organization's product.
    async fn find_notary_spu(
        &self,
        spu_no: &str,
        organization_id: &str,
    ) -> Result<Option<SpuRecord>, NotaryServiceError> {
        let records = self
            .catalog
            .list_spus(&ProductSpuListQuery {
                tenant_id: self.tenant_id.clone(),
                organization_id: Some(organization_id.to_owned()),
                q: Some(spu_no.to_owned()),
                category_id: Some(self.notary_category_id().await?),
                product_type: Some(NOTARY_PRODUCT_TYPE.as_storage_str().to_owned()),
                status: None,
                sort: None,
                page: Some(1),
                page_size: Some(MATTER_PROBE_PAGE_SIZE),
            })
            .await
            .map_err(map_commerce_error)?;
        Ok(records
            .into_iter()
            .find(|record| record.spu_no.as_str() == spu_no))
    }

    /// Finds a notary variant by the business key its creation derived.
    async fn find_notary_sku(
        &self,
        spu_id: i64,
        sku_no: &str,
    ) -> Result<Option<SkuRecord>, NotaryServiceError> {
        let records = self
            .catalog
            .list_skus(&ProductSkuListQuery {
                tenant_id: self.tenant_id.clone(),
                organization_id: None,
                spu_id: Some(spu_id.to_string()),
                attribute_value_id: None,
                status: None,
                page: Some(1),
                page_size: Some(MATTER_PROBE_PAGE_SIZE),
            })
            .await
            .map_err(map_commerce_error)?;
        Ok(records
            .into_iter()
            .find(|record| record.sku_no.as_str() == sku_no))
    }

    /// Reads the SPU that a variant belongs to.
    async fn require_spu(&self, spu_id: i64) -> Result<SpuRecord, NotaryServiceError> {
        self.catalog
            .retrieve_spu(&ProductSpuRetrieveQuery {
                tenant_id: self.tenant_id.clone(),
                spu_id: spu_id.to_string(),
            })
            .await
            .map_err(map_commerce_error)?
            .ok_or_else(|| {
                // `fk_commerce_product_sku_spu` cascades, so a live variant whose product is gone is
                // not a missing address — it is two rows that disagree about whether they exist.
                NotaryServiceError::invalid_state(format!(
                    "notary matter variant {spu_id} has no product row"
                ))
            })
    }

    /// Reads a variant by id, or reports that the matter is gone.
    async fn require_sku(&self, sku_id: &str) -> Result<SkuRecord, NotaryServiceError> {
        self.catalog
            .retrieve_sku(&ProductSkuRetrieveQuery {
                tenant_id: self.tenant_id.clone(),
                sku_id: sku_id.to_owned(),
            })
            .await
            .map_err(map_commerce_error)?
            .ok_or_else(|| NotaryServiceError::not_found("notary matter was not found"))
    }

    /// The single variant that prices a notary product.
    ///
    /// The notary write path mints exactly one variant per matter, and the notary contract has only
    /// one price field, so a second live variant under a notary product came from outside that path.
    /// When that happens the matter no longer has a price, it has a choice of prices, and picking one
    /// would publish a number the notary never chose. The error names the product so an operator can
    /// see which row to repair.
    async fn sole_variant_of(&self, spu: &SpuRecord) -> Result<SkuRecord, NotaryServiceError> {
        let mut variants = self
            .catalog
            .list_skus(&ProductSkuListQuery {
                tenant_id: self.tenant_id.clone(),
                organization_id: None,
                spu_id: Some(spu.id.to_string()),
                attribute_value_id: None,
                status: None,
                page: Some(1),
                page_size: Some(MATTER_PROBE_PAGE_SIZE),
            })
            .await
            .map_err(map_commerce_error)?;
        match variants.len() {
            1 => Ok(variants.swap_remove(0)),
            0 => Err(NotaryServiceError::invalid_state(format!(
                "notary product {} has no variant, so it has no price",
                spu.spu_no
            ))),
            _ => Err(NotaryServiceError::invalid_state(format!(
                "notary product {} carries more than one variant, so its price is ambiguous",
                spu.spu_no
            ))),
        }
    }

    /// Builds the list query that asks the catalog for this page of notary products.
    fn matter_spu_query(
        &self,
        category_id: &str,
        query: &CommerceMatterListQuery,
        page: i64,
        page_size: i64,
    ) -> ProductSpuListQuery {
        ProductSpuListQuery {
            tenant_id: self.tenant_id.clone(),
            organization_id: query.organization_id.clone(),
            q: query.search_term.clone(),
            category_id: Some(category_id.to_owned()),
            product_type: Some(NOTARY_PRODUCT_TYPE.as_storage_str().to_owned()),
            status: query.status.clone(),
            sort: None,
            page: Some(page),
            page_size: Some(page_size),
        }
    }

    /// Renders one matter from the two rows that carry it.
    async fn matter_record(
        &self,
        registry: &mut CurrencyRegistry<'_>,
        spu: &SpuRecord,
        sku: &SkuRecord,
    ) -> Result<CommerceMatterRecord, NotaryServiceError> {
        let unit = registry
            .unit_at(&sku.currency_code, sku.price_scale)
            .await?;
        Ok(matter_record_from_spu_and_sku(spu, sku, unit))
    }

    /// Moves a product to the status the matter was created with.
    ///
    /// `CreateProductSpuCommand` has no status and `INSERT_SPU_SQL` writes `'draft'`, so a matter
    /// that is not a draft takes a second statement. It is skipped when the status *is* `draft`:
    /// re-stating what the insert already recorded would advance `version` for a no-op.
    /// Writes the precise sales status to the row that carries it.
    ///
    /// The variant is the carrier: one product may have many variants, and only they record which of
    /// them is on sale. The product row's own `status` is a projection of this value (see
    /// [`Self::project_product_status`]), so the precise value is written here and read back from
    /// here — never restated from the caller's instruction straight onto the projection.
    ///
    /// A write that would store what the row already holds is skipped rather than issued. The
    /// baseline advances `version` on every update and every other caller's copy of the row is
    /// compared against that version, so a no-op write would invalidate those copies for no change.
    /// On the create path this is also what keeps a matter created `draft` at one statement instead
    /// of two, because `commerce_product_sku` already defaults to it.
    async fn apply_variant_status(
        &self,
        sku: &SkuRecord,
        status: ProductStatus,
    ) -> Result<SkuRecord, NotaryServiceError> {
        if sku.status == status.as_storage_str() {
            return Ok(sku.clone());
        }
        require_applied(
            self.catalog
                .update_sku(&UpdateProductSkuCommand {
                    tenant_id: self.tenant_id.clone(),
                    sku_id: sku.id.to_string(),
                    name: None,
                    title: None,
                    // The repository restates price, currency, and scale from the row it locks inside
                    // the same transaction, so a status-only edit cannot reinterpret a count it is not
                    // touching. `None` here means "keep", not "clear".
                    sale_price_minor: None,
                    list_price_minor: None,
                    currency_code: None,
                    fulfillment_type: None,
                    inventory_tracking: None,
                    status: Some(status),
                    attribute_value_ids: None,
                    metadata: None,
                    expected_version: sku.version,
                })
                .await
                .map_err(map_commerce_error)?,
            "notary matter variant",
        )
    }

    /// Projects the variant's precise status onto the row the listing filters on.
    ///
    /// `list_notary_matters` narrows on `commerce_product_spu.status`, because a page is a page of
    /// products; what it narrows by therefore has to be the variant's value, not a second copy of
    /// whatever the caller asked for. Reconciling the two in one place and in this direction is what
    /// makes "the product says `active` while its only variant says `draft`" unrepresentable through
    /// this port.
    ///
    /// Callers pass the value read off the variant row, which also means a replay of a create whose
    /// first attempt stopped between the two rows carries the projection forward instead of leaving
    /// the two disagreeing.
    async fn project_product_status(
        &self,
        spu: &SpuRecord,
        status: ProductStatus,
    ) -> Result<SpuRecord, NotaryServiceError> {
        if spu.status == status.as_storage_str() {
            return Ok(spu.clone());
        }
        require_applied(
            self.catalog
                .update_spu(&UpdateProductSpuCommand {
                    tenant_id: self.tenant_id.clone(),
                    spu_id: spu.id.to_string(),
                    title: None,
                    subtitle: None,
                    description: None,
                    category_id: None,
                    status: Some(status),
                    expected_version: spu.version,
                })
                .await
                .map_err(map_commerce_error)?,
            "notary matter product",
        )
    }
}

/// Reads a stored status column back into the vocabulary the catalog defines.
///
/// Total rather than defensive: `ck_commerce_product_spu_status` and
/// `ck_commerce_product_sku_status` each accept exactly the four tokens
/// [`ProductStatus::from_storage_str`] does, so a value that fails here is one PostgreSQL would not
/// have allowed to be stored in the first place.
fn stored_status(status: &str) -> Result<ProductStatus, NotaryServiceError> {
    ProductStatus::from_storage_str(status).map_err(map_commerce_error)
}

#[async_trait]
impl CommercePort for CommerceOrderPort {
    async fn create_notary_order(
        &self,
        command: CommerceCreateOrderCommand,
    ) -> Result<CommerceOrderReference, NotaryServiceError> {
        let line =
            CheckoutLineInput::new(command.sku_id.as_str(), 1).map_err(map_commerce_error)?;
        let request_digest = sha256_hash(command.idempotency_key.as_bytes());
        let request_no = format!("notary-{}", &request_digest[..24]);
        let session_idempotency = format!("{}-session", command.idempotency_key);
        let session_command = CreateCheckoutSessionCommand::new(
            self.tenant_id.as_str(),
            Some(command.organization_id.as_str()),
            self.owner_user_id.as_str(),
            "CNY",
            vec![line],
            request_no.as_str(),
            session_idempotency.as_str(),
        )
        .map_err(map_commerce_error)?;

        let session = self
            .store
            .create_checkout_session(session_command)
            .await
            .map_err(map_commerce_error)?;

        let quote_command = CreateCheckoutQuoteCommand::new(
            self.tenant_id.as_str(),
            Some(command.organization_id.as_str()),
            self.owner_user_id.as_str(),
            session.checkout_session_id.as_str(),
            request_no.as_str(),
            format!("{}-quote", command.idempotency_key).as_str(),
        )
        .map_err(map_commerce_error)?;

        self.store
            .create_checkout_quote(quote_command)
            .await
            .map_err(map_commerce_error)?;

        let order_command = CreateOwnerOrderCommand::new(
            self.tenant_id.as_str(),
            Some(command.organization_id.as_str()),
            self.owner_user_id.as_str(),
            session.checkout_session_id.as_str(),
            request_no.as_str(),
            command.idempotency_key.as_str(),
        )
        .map_err(map_commerce_error)?;

        let outcome = self
            .store
            .create_owner_order(order_command)
            .await
            .map_err(map_commerce_error)?;

        let detail_query = OrderOwnerDetailQuery::new(
            self.tenant_id.as_str(),
            Some(command.organization_id.as_str()),
            self.owner_user_id.as_str(),
            outcome.order_id.as_str(),
        )
        .map_err(map_commerce_error)?;
        let detail = self
            .store
            .retrieve_owner_order(detail_query)
            .await
            .map_err(map_commerce_error)?
            .ok_or_else(|| {
                NotaryServiceError::not_found("order detail was not created for notary case")
            })?;
        let order_item = match detail.items.as_slice() {
            [item] => item,
            [] => {
                return Err(NotaryServiceError::not_found(
                    "order item was not created for notary case",
                ));
            }
            _ => {
                return Err(NotaryServiceError::conflict(
                    "notary checkout must create exactly one order item",
                ));
            }
        };

        // The order module speaks smallest units too, so the same registry row places the point.
        let unit = self
            .catalog
            .currency_unit(&detail.summary.currency_code)
            .await
            .map_err(map_commerce_error)?;

        Ok(CommerceOrderReference {
            order_id: outcome.order_id,
            order_item_id: order_item.id.clone(),
            sku_id: command.sku_id,
            matter_title: command.title,
            fee_amount: render_stored_amount(
                outcome.total_amount.as_str(),
                unit,
                "order total amount",
            )?,
            currency_code: detail.summary.currency_code,
        })
    }

    async fn cancel_notary_order(
        &self,
        organization_id: &str,
        order_id: &str,
    ) -> Result<(), NotaryServiceError> {
        let command = CancelOwnerOrderCommand::new(
            self.tenant_id.as_str(),
            Some(organization_id),
            self.owner_user_id.as_str(),
            order_id,
            Some(NOTARY_ORDER_CANCEL_REASON),
        )
        .map_err(map_commerce_error)?;
        self.store
            .cancel_owner_order(command)
            .await
            .map_err(map_commerce_error)
    }

    async fn get_notary_order_fulfillment_state(
        &self,
        organization_id: &str,
        order_id: &str,
    ) -> Result<CommerceOrderFulfillmentState, NotaryServiceError> {
        let query = OrderOwnerDetailQuery::new(
            self.tenant_id.as_str(),
            Some(organization_id),
            self.owner_user_id.as_str(),
            order_id,
        )
        .map_err(map_commerce_error)?;
        let detail = self
            .store
            .retrieve_owner_order(query)
            .await
            .map_err(map_commerce_error)?
            .ok_or_else(|| {
                NotaryServiceError::not_found(
                    "commerce order was not found for the notary organization",
                )
            })?;
        // Smallest units, and deliberately not the major string `fee_amount` above is. The
        // acceptance path decides whether there is anything to pay by testing that this value is a
        // non-empty run of `0` digits, which is a test a bare integer survives and a decimal does
        // not: `"0"` reads as nothing to pay and `"0.00"` would read as something to pay. Changing
        // this unit without changing that test would refuse every free notarization.
        let payable_amount = detail.summary.total_amount.as_str().to_owned();
        Ok(CommerceOrderFulfillmentState {
            order_id: detail.summary.order_id,
            order_status: detail.summary.status,
            payment_status: detail.payment_status,
            payable_amount,
        })
    }

    async fn list_notary_matters(
        &self,
        query: CommerceMatterListQuery,
    ) -> Result<CommerceMatterListPage, NotaryServiceError> {
        // The port is public, so it does not rely on the caller having validated anything: a zero
        // page size would be a division by zero below, and "whatever the caller passed" is not a
        // reason to panic in a library.
        if query.page_size < 1 {
            return Err(NotaryServiceError::validation(
                "page_size must be at least 1",
            ));
        }
        let offset = query.offset.max(0);
        let page_size = query.page_size;
        let category_id = self.notary_category_id().await?;

        // # Bridging an offset cursor to a page number
        //
        // The notary's list is cursored by row offset; the catalog pages by `page` and `page_size`,
        // and the statement behind it derives the offset as `(page - 1) * page_size`. The two agree
        // exactly only when the offset is a multiple of the page size, which it usually is — the
        // cursor advances by the number of rows returned — but not always, because
        // `decode_offset_cursor` accepts any non-negative integer and a caller may hand one back.
        //
        // So the page that contains the offset is asked for, and the rows before the offset inside
        // it are dropped. When the offset lands mid-page and that page was full, the window it needs
        // does not end inside it, so the next page is read too: with `page_size` capped at 100 by the
        // notary's own pagination policy and the catalog's `LIMIT` ceiling at 200, this is at most
        // two statements and never a scan.
        //
        // `has_more` comes from `count_spus` over the identical predicate rather than from having
        // fetched one row too many, because the ceiling above makes "one row too many" impossible to
        // ask for at a page size of 200. It is exact as of that count, which is the same guarantee
        // offset pagination already gives: a matter created after the count may land beyond it.
        let page = offset / page_size + 1;
        let skip = usize::try_from(offset % page_size).map_err(|_| {
            NotaryServiceError::validation("offset is beyond the range this runtime can page")
        })?;

        let mut window = self
            .catalog
            .list_spus(&self.matter_spu_query(&category_id, &query, page, page_size))
            .await
            .map_err(map_commerce_error)?;
        let total_items = self
            .catalog
            .count_spus(&self.matter_spu_query(&category_id, &query, page, page_size))
            .await
            .map_err(map_commerce_error)?;

        if skip > 0 && window.len() as i64 == page_size {
            window.extend(
                self.catalog
                    .list_spus(&self.matter_spu_query(&category_id, &query, page + 1, page_size))
                    .await
                    .map_err(map_commerce_error)?,
            );
        }

        let mut registry = CurrencyRegistry::new(&self.catalog);
        let mut items = Vec::new();
        for spu in window.into_iter().skip(skip).take(page_size as usize) {
            let sku = self.sole_variant_of(&spu).await?;
            items.push(self.matter_record(&mut registry, &spu, &sku).await?);
        }

        Ok(CommerceMatterListPage {
            has_more: offset.saturating_add(items.len() as i64) < total_items,
            items,
        })
    }

    async fn create_notary_matter(
        &self,
        command: CommerceMatterCommand,
    ) -> Result<CommerceMatterRecord, NotaryServiceError> {
        require_spec_object(&command.spec)?;
        // Parsed with the rest of the validation, before anything is written. The previous shape read
        // this only at the moment it was written, which was *after* `create_spu` had committed — so a
        // status this port does not speak left a product row behind for a command that was never
        // acceptable.
        let status = stored_status(&command.status)?;
        let organization_id = command.organization_id.as_deref().unwrap_or("0").to_owned();
        let category_id = self.notary_category_id().await?;
        let unit = self
            .catalog
            .currency_unit(&command.currency_code)
            .await
            .map_err(map_commerce_error)?;

        let sale_price_minor = parse_major_amount(&command.price_amount, unit, "priceAmount")?;
        let list_price_minor = command
            .original_price_amount
            .as_deref()
            .map(|amount| parse_major_amount(amount, unit, "originalPriceAmount"))
            .transpose()?;
        ensure_reference_price_not_lower(sale_price_minor, list_price_minor)?;

        // The derived business key is what makes this write idempotent; see `derived_business_key`.
        let spu_no = derived_business_key("spu", &self.tenant_id, &command.idempotency_key);
        let sku_no = derived_business_key("sku", &self.tenant_id, &command.idempotency_key);

        let create_spu = CreateProductSpuCommand {
            tenant_id: self.tenant_id.clone(),
            organization_id: organization_id.clone(),
            spu_no: spu_no.clone(),
            title: command.title.clone(),
            subtitle: None,
            description: command.description.clone(),
            product_type: NOTARY_PRODUCT_TYPE,
            category_id,
        };
        let spu = match self.catalog.create_spu(&create_spu).await {
            Ok(created) => created,
            // A conflict is only swallowed when the product the key names is actually there: a
            // retried create meets its own row and reports that matter, and anything else is the
            // conflict it was.
            Err(error) if error.code() == "conflict" => {
                match self.find_notary_spu(&spu_no, &organization_id).await? {
                    Some(existing) => existing,
                    None => return Err(map_commerce_error(error)),
                }
            }
            Err(error) => return Err(map_commerce_error(error)),
        };

        let create_sku = CreateProductSkuCommand {
            tenant_id: self.tenant_id.clone(),
            organization_id,
            spu_id: spu.id.to_string(),
            sku_no: sku_no.clone(),
            name: command.title.clone(),
            title: command.title.clone(),
            sale_price_minor,
            list_price_minor,
            currency_code: command.currency_code.clone(),
            fulfillment_type: NOTARY_FULFILLMENT_TYPE,
            inventory_tracking: NOTARY_INVENTORY_TRACKING,
            // The notary category declares no sales axis, so a matter has the empty combination and
            // `variant_signature` falls back to `sku_no` — which is what keeps
            // `uk_commerce_product_sku_variant` meaningful without inventing an axis.
            attribute_value_ids: Vec::new(),
            metadata: command.spec.clone(),
        };
        let sku = match self.catalog.create_sku(&create_sku).await {
            Ok(created) => self.apply_variant_status(&created, status).await?,
            Err(error) if error.code() == "conflict" => {
                match self.find_notary_sku(spu.id, &sku_no).await? {
                    Some(existing) => existing,
                    None => return Err(map_commerce_error(error)),
                }
            }
            Err(error) => return Err(map_commerce_error(error)),
        };

        // Last, and read off the variant rather than reused from `status`: the projection follows the
        // row that carries the precise value, so a replay that meets a half-written pair reconciles
        // it here instead of restating the caller's input over a row it may not match.
        let spu = self
            .project_product_status(&spu, stored_status(&sku.status)?)
            .await?;

        let mut registry = CurrencyRegistry::new(&self.catalog);
        self.matter_record(&mut registry, &spu, &sku).await
    }

    async fn update_notary_matter(
        &self,
        command: CommerceMatterUpdateCommand,
    ) -> Result<CommerceMatterRecord, NotaryServiceError> {
        if let Some(spec) = command.spec.as_ref() {
            require_spec_object(spec)?;
        }
        let status = command
            .status
            .as_deref()
            .map(ProductStatus::from_storage_str)
            .transpose()
            .map_err(map_commerce_error)?;

        // The notary's port carries no `If-Match`, so this read-modify-write owns its own optimistic
        // retry: the catalog refuses a write whose row has moved since it was read, and the answer to
        // that here is to read the row again, not to hand a `409` to a user who never saw a version.
        let mut attempt = 0;
        loop {
            attempt += 1;
            let current = self.require_sku(command.sku_id.as_str()).await?;
            let mut spu = self.require_spu(current.spu_id).await?;

            let mut registry = CurrencyRegistry::new(&self.catalog);
            let currency = command
                .currency_code
                .clone()
                .unwrap_or_else(|| current.currency_code.clone());
            let unit = registry.registry_unit(&currency).await?;

            let sale_price_minor = match command.price_amount.as_deref() {
                Some(amount) => parse_major_amount(amount, unit, "priceAmount")?,
                None => current.sale_price_minor,
            };
            // The reference price keeps all three of its states, because flattening two of them would
            // make "this matter no longer has a reference price" indistinguishable from "this edit did
            // not mention one" — the catalog's `list_price_minor` has the same three states for the
            // same reason. `effective_reference` is what the comparison is made against; the
            // three-state value is what is submitted.
            let reference_price = match command.original_price_amount.as_ref() {
                None => None,
                Some(None) => Some(None),
                Some(Some(amount)) => Some(Some(parse_major_amount(
                    amount,
                    unit,
                    "originalPriceAmount",
                )?)),
            };
            let effective_reference = match reference_price {
                Some(reference) => reference,
                None => current.list_price_minor,
            };
            ensure_reference_price_not_lower(sale_price_minor, effective_reference)?;

            // Each row is written only when an instruction actually lands on it. An amount-only edit
            // has nothing to say to the SPU, and a title-only edit has nothing to say to the SKU
            // beyond keeping its copy of the name in step; writing either unconditionally would
            // advance a version every other caller's copy is compared against, for no change.
            //
            // A currency change that leaves an existing reference price unaddressed is refused by the
            // catalog rather than handled here: a minor count cannot be reinterpreted under a new
            // scale, so "restate it or clear it" is a rule this adapter has no better answer to than
            // the layer that owns the columns.
            let writes_sku = command.title.is_some()
                || command.price_amount.is_some()
                || command.original_price_amount.is_some()
                || command.currency_code.is_some()
                || status.is_some()
                || command.spec.is_some();

            let sku = if writes_sku {
                let written = self
                    .catalog
                    .update_sku(&UpdateProductSkuCommand {
                        tenant_id: self.tenant_id.clone(),
                        sku_id: current.id.to_string(),
                        // The variant's own name and title were written once at creation; they are
                        // restated here only so that they cannot drift from the product's, because a
                        // console showing two different names for one matter is worse than a copy.
                        name: command.title.clone(),
                        title: command.title.clone(),
                        sale_price_minor: Some(sale_price_minor),
                        list_price_minor: reference_price,
                        currency_code: command.currency_code.clone(),
                        fulfillment_type: None,
                        inventory_tracking: None,
                        status,
                        attribute_value_ids: None,
                        metadata: command.spec.clone(),
                        expected_version: current.version,
                    })
                    .await
                    .map_err(map_commerce_error)?;
                match written {
                    GuardedWrite::Applied(sku) => sku,
                    GuardedWrite::StaleVersion(_) if attempt < MATTER_WRITE_ATTEMPTS => continue,
                    GuardedWrite::StaleVersion(stale) => {
                        return Err(stale_conflict("notary matter", stale));
                    }
                }
            } else {
                current
            };

            // Read off the variant after the write above, not reused from `status`: the projection
            // follows the row that carries the precise value. When the caller said nothing about
            // status this reads back the unchanged value, which is what keeps the two rows agreeing
            // through an edit that only touched a title.
            let projected = stored_status(&sku.status)?;

            if command.title.is_some() || command.description.is_some() || status.is_some() {
                let written = self
                    .catalog
                    .update_spu(&UpdateProductSpuCommand {
                        tenant_id: self.tenant_id.clone(),
                        spu_id: spu.id.to_string(),
                        title: command.title.clone(),
                        subtitle: None,
                        // `description` is one of the catalog's three-state fields — `None` preserves,
                        // `Some(None)` clears — and the notary's port already speaks exactly that, so
                        // it is passed through rather than flattened.
                        description: command.description.clone(),
                        category_id: None,
                        status: Some(projected),
                        expected_version: spu.version,
                    })
                    .await
                    .map_err(map_commerce_error)?;
                spu = match written {
                    GuardedWrite::Applied(spu) => spu,
                    GuardedWrite::StaleVersion(_) if attempt < MATTER_WRITE_ATTEMPTS => continue,
                    GuardedWrite::StaleVersion(stale) => {
                        return Err(stale_conflict("notary matter product", stale));
                    }
                };
            }

            // Rendered from the rows as written: an edit that changed the currency changed the scale
            // too, and the count it just stored means the scale it just stored.
            return self.matter_record(&mut registry, &spu, &sku).await;
        }
    }
}

/// The `commerce_currency` rows this request has already read.
///
/// A page of matters is priced in one currency, sometimes two, and every row in it resolves to the
/// same registry row — so reading it once per distinct code keeps the listing from issuing a lookup
/// per matter. The scope is one call and not the process, and that matters: a `commerce_currency`
/// edit that changed an exponent would otherwise keep quoting prices at the old scale for as long as
/// the runtime lived, which is the silent-factor-of-ten defect the registry exists to prevent.
struct CurrencyRegistry<'a> {
    catalog: &'a PostgresCommerceCatalogStore,
    resolved: BTreeMap<String, MoneyUnit>,
}

impl<'a> CurrencyRegistry<'a> {
    fn new(catalog: &'a PostgresCommerceCatalogStore) -> Self {
        Self {
            catalog,
            resolved: BTreeMap::new(),
        }
    }

    /// The unit `commerce_currency` currently declares for `code`.
    async fn registry_unit(&mut self, code: &str) -> Result<MoneyUnit, NotaryServiceError> {
        if let Some(unit) = self.resolved.get(code) {
            return Ok(*unit);
        }
        let unit = self
            .catalog
            .currency_unit(code)
            .await
            .map_err(map_commerce_error)?;
        self.resolved.insert(code.to_owned(), unit);
        Ok(unit)
    }

    /// The unit a **stored** count of `scale` was written with.
    ///
    /// The scale is the row's own `price_scale` snapshot, not the registry's current exponent,
    /// because the snapshot is what the stored count means: re-placing the point from a registry row
    /// that has since been re-scaled would relabel every historical price by a factor of ten and
    /// report it as an ordinary number. Only the code and the rounding mode come from the registry,
    /// which are the two parts of the unit a stored count does not carry. Both are read from the row
    /// the kernel already validated — neither is supplied by this file.
    async fn unit_at(&mut self, code: &str, scale: i64) -> Result<MoneyUnit, NotaryServiceError> {
        let registry = self.registry_unit(code).await?;
        let scale = u8::try_from(scale).map_err(|_| {
            NotaryServiceError::storage(format!(
                "stored price scale {scale} for {code} is outside the range a unit can declare"
            ))
        })?;
        MoneyUnit::custom(registry.code(), scale, registry.rounding()).map_err(|error| {
            NotaryServiceError::storage(format!(
                "stored price scale {scale} for {code} is unusable: {error}"
            ))
        })
    }
}

/// The catalog business key a notary matter's product or variant is addressed by.
///
/// `commerce_product_spu.spu_no` and `commerce_product_sku.sku_no` are each unique per tenant while
/// live, and they are the only unique handles the catalog offers a capability: v2 writes carry no
/// idempotency key, and `metadata` is not indexed, so there is nowhere else one could be enforced.
/// Deriving these keys from the idempotency key therefore turns that key into the write's identity —
/// a retried create re-derives the same pair, meets its own rows, and returns the matter instead of
/// minting a second one.
///
/// A title-derived slug was the other candidate and is the wrong one: two matters may legitimately
/// share a title, and the unique index would then reject the second one as a duplicate of the first.
///
/// The digest is hex because `%` and `_` are `LIKE` wildcards and the recovery path looks these keys
/// up through the list query's free-text filter; uppercase because a business key is something an
/// operator reads out of a console and pastes into a ticket.
fn derived_business_key(kind: &str, tenant_id: &str, idempotency_key: &str) -> String {
    let seed = format!("notary:{kind}:{tenant_id}:{idempotency_key}");
    let digest = sha256_hash(seed.as_bytes());
    format!("NOTARY-{kind}-{}", digest[..32].to_ascii_uppercase())
}

/// Reads a major-denomination literal into the smallest-unit count the catalog stores.
fn parse_major_amount(
    amount: &str,
    unit: MoneyUnit,
    field_name: &str,
) -> Result<i64, NotaryServiceError> {
    let money =
        Money::parse(amount, unit).map_err(|error| amount_error(field_name, unit, &error))?;
    let money = money
        .require_non_negative()
        .map_err(|error| amount_error(field_name, unit, &error))?;
    i64::try_from(money.minor()).map_err(|_| {
        NotaryServiceError::validation(format!(
            "{field_name} is larger than {} can hold",
            unit.code()
        ))
    })
}

fn amount_error(field_name: &str, unit: MoneyUnit, error: &MoneyError) -> NotaryServiceError {
    NotaryServiceError::validation(format!(
        "{field_name} must be a non-negative major-denomination decimal with at most {} fractional digits for {}: {error}",
        unit.scale(),
        unit.code()
    ))
}

/// Renders a count the catalog already stored, which cannot fail once the unit is known.
fn render_stored_count(minor: i64, unit: MoneyUnit) -> String {
    Money::from_minor(i128::from(minor), unit).to_major_string()
}

/// Renders a smallest-unit count that arrived as text, as the order module publishes it.
fn render_stored_amount(
    minor: &str,
    unit: MoneyUnit,
    field_name: &str,
) -> Result<String, NotaryServiceError> {
    let money = Money::from_minor_str(minor, unit).map_err(|_| {
        NotaryServiceError::storage(format!(
            "{field_name} is not a smallest-unit integer: `{minor}`"
        ))
    })?;
    if money.is_negative() {
        return Err(NotaryServiceError::storage(format!(
            "{field_name} must not be negative"
        )));
    }
    Ok(money.to_major_string())
}

/// Refuses a reference price below the price being charged.
///
/// The catalog enforces the same rule as `ck_commerce_product_sku_sale_not_above_list`, and it is
/// restated here because the notary's contract has its own name for each side of the comparison and
/// a caller that gets a `422` naming `originalPriceAmount` can act on it, where
/// "violates ck_commerce_product_sku_sale_not_above_list" makes them go and read the baseline.
fn ensure_reference_price_not_lower(
    price_minor: i64,
    reference_minor: Option<i64>,
) -> Result<(), NotaryServiceError> {
    if reference_minor.is_some_and(|reference| reference < price_minor) {
        return Err(NotaryServiceError::validation(
            "originalPriceAmount must not be lower than priceAmount",
        ));
    }
    Ok(())
}

/// Rejects a capability metadata payload that is not a JSON object.
///
/// `commerce_product_sku.metadata` is `NOT NULL DEFAULT '{}'` and every reader treats it as an
/// object, so a string or a number would either be refused deeper in the catalog with a message
/// naming the catalog's field, or stored as a document the notary could never read back.
fn require_spec_object(spec: &serde_json::Value) -> Result<(), NotaryServiceError> {
    if spec.is_object() {
        Ok(())
    } else {
        Err(NotaryServiceError::validation("spec must be a JSON object"))
    }
}

/// Unwraps a write this code has no way to recover from, naming what was being written.
fn require_applied<T>(write: GuardedWrite<T>, what: &str) -> Result<T, NotaryServiceError> {
    match write {
        GuardedWrite::Applied(value) => Ok(value),
        GuardedWrite::StaleVersion(stale) => Err(stale_conflict(what, stale)),
    }
}

fn stale_conflict(what: &str, stale: StaleVersion) -> NotaryServiceError {
    NotaryServiceError::conflict(format!(
        "{what} was written by another caller while this request held version {} (now {})",
        stale.expected, stale.actual
    ))
}

/// Maps one matter from the product row that carries it and the variant row that prices it.
///
/// * `title` and `description` come from the **SPU**: in catalog v2 `description` is a
///   `commerce_product_spu` column, and the title is the same product-level fact. The SKU keeps a
///   copy of the name and title it was created with, which is why an update restates both.
/// * `status` comes from the **SKU**, the row that records whether the thing being sold is on; the
///   SPU's identical value is the projection the listing filters on. Nothing here clamps it to the
///   notary's three-value vocabulary: a fourth value would only appear from a write outside the
///   notary path, and reporting a state that exists is not a lie, where hiding it would be.
/// * `spec` is the SKU's `metadata`, returned verbatim. It is capability-owned storage, not a
///   projection of anything the catalog understands.
fn matter_record_from_spu_and_sku(
    spu: &SpuRecord,
    sku: &SkuRecord,
    unit: MoneyUnit,
) -> CommerceMatterRecord {
    CommerceMatterRecord {
        sku_id: sku.id.to_string(),
        spu_id: spu.id.to_string(),
        sku_no: sku.sku_no.clone(),
        title: spu.title.clone().unwrap_or_else(|| spu.name.clone()),
        description: spu.description.clone(),
        price_amount: render_stored_count(sku.sale_price_minor, unit),
        original_price_amount: sku
            .list_price_minor
            .map(|minor| render_stored_count(minor, unit)),
        currency_code: sku.currency_code.clone(),
        status: sku.status.clone(),
        spec: sku.metadata.clone(),
    }
}

fn map_commerce_error(error: CommerceServiceError) -> NotaryServiceError {
    let message = error.message().to_owned();
    match error.code() {
        "unauthenticated" => NotaryServiceError::unauthenticated(message),
        "unauthorized" => NotaryServiceError::unauthorized(message),
        "not-found" => NotaryServiceError::not_found(message),
        "conflict" | "locked" => NotaryServiceError::conflict(message),
        "invalid-state" => NotaryServiceError::invalid_state(message),
        "validation" => NotaryServiceError::validation(message),
        "transport" => NotaryServiceError::transport(message),
        "unsupported-capability" | "provider-unavailable" => {
            NotaryServiceError::provider_unavailable(message)
        }
        "storage" => NotaryServiceError::storage(message),
        _ => NotaryServiceError::unknown(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cny() -> MoneyUnit {
        MoneyUnit::of(sdkwork_commerce_money::UnitCode::CNY).expect("CNY is a built-in unit")
    }

    fn sample_spu() -> SpuRecord {
        SpuRecord {
            id: 1001,
            tenant_id: 1,
            organization_id: 200001,
            spu_no: "NOTARY-SPU-1".to_owned(),
            category_id: 9001,
            name: "Remote notarization".to_owned(),
            title: Some("Remote notarization".to_owned()),
            subtitle: None,
            description: Some("Three-day service".to_owned()),
            product_type: "service".to_owned(),
            status: "active".to_owned(),
            sales_status: "active".to_owned(),
            published_at: Some("2026-07-11T00:00:00Z".to_owned()),
            version: 1,
            created_at: "2026-07-11T00:00:00Z".to_owned(),
            updated_at: "2026-07-11T00:00:00Z".to_owned(),
        }
    }

    fn sample_sku() -> SkuRecord {
        SkuRecord {
            id: 2002,
            tenant_id: 1,
            organization_id: 200001,
            spu_id: 1001,
            sku_no: "NOTARY-SKU-1".to_owned(),
            variant_signature: "NOTARY-SKU-1".to_owned(),
            name: Some("Remote notarization".to_owned()),
            title: Some("Remote notarization".to_owned()),
            currency_code: "CNY".to_owned(),
            price_scale: 2,
            sale_price_minor: 60000,
            list_price_minor: Some(80000),
            fulfillment_type: "service".to_owned(),
            inventory_tracking: "none".to_owned(),
            status: "inactive".to_owned(),
            sales_status: "inactive".to_owned(),
            published_at: None,
            version: 4,
            created_at: "2026-07-11T00:00:00Z".to_owned(),
            updated_at: "2026-07-11T00:00:00Z".to_owned(),
            attribute_values: Vec::new(),
            metadata: serde_json::json!({"materialCodes": ["identity", "evidence"]}),
        }
    }

    #[test]
    fn a_matter_reads_its_copy_from_the_product_and_its_price_from_the_variant() {
        let record = matter_record_from_spu_and_sku(&sample_spu(), &sample_sku(), cny());
        assert_eq!(record.sku_id, "2002");
        assert_eq!(record.spu_id, "1001");
        // Product-level facts, from the SPU.
        assert_eq!(record.title, "Remote notarization");
        assert_eq!(record.description.as_deref(), Some("Three-day service"));
        // Sellable facts, from the SKU.
        assert_eq!(record.price_amount, "600.00");
        assert_eq!(record.original_price_amount.as_deref(), Some("800.00"));
        assert_eq!(record.currency_code, "CNY");
        assert_eq!(record.status, "inactive");
        // Capability metadata, verbatim and unwrapped.
        assert_eq!(
            record.spec,
            serde_json::json!({"materialCodes": ["identity", "evidence"]})
        );
    }

    #[test]
    fn a_product_without_a_title_falls_back_to_the_required_name_column() {
        let mut spu = sample_spu();
        spu.title = None;
        let record = matter_record_from_spu_and_sku(&spu, &sample_sku(), cny());
        assert_eq!(record.title, "Remote notarization");
    }

    #[test]
    fn converts_major_and_minor_money_without_floating_point() {
        assert_eq!(
            parse_major_amount("600.00", cny(), "priceAmount").expect("minor units"),
            60000
        );
        assert_eq!(
            parse_major_amount("600", cny(), "priceAmount").expect("minor units"),
            60000
        );
        assert_eq!(render_stored_count(60000, cny()), "600.00");
        assert_eq!(render_stored_count(0, cny()), "0.00");

        let jpy = MoneyUnit::of(sdkwork_commerce_money::UnitCode::JPY).expect("JPY");
        assert_eq!(
            parse_major_amount("100", jpy, "priceAmount").expect("JPY minor units"),
            100
        );
        assert_eq!(render_stored_count(100, jpy), "100");
    }

    #[test]
    fn refuses_an_amount_the_declared_scale_cannot_hold() {
        // The registry says CNY carries two fractional digits, so a third is refused rather than
        // rounded: a write path that silently changes an amount is the defect, not the fix.
        assert!(parse_major_amount("1.001", cny(), "priceAmount").is_err());
        let jpy = MoneyUnit::of(sdkwork_commerce_money::UnitCode::JPY).expect("JPY");
        assert!(parse_major_amount("100.00", jpy, "priceAmount").is_err());
        assert!(parse_major_amount("", cny(), "priceAmount").is_err());
        assert!(parse_major_amount("twelve", cny(), "priceAmount").is_err());
    }

    #[test]
    fn refuses_a_negative_amount_before_the_database_has_to() {
        // `ck_commerce_product_sku_sale_price` is `sale_price_minor >= 0`; a sign that reaches it
        // comes back as a `23514` naming a constraint instead of the field the caller sent.
        let error = parse_major_amount("-1.00", cny(), "priceAmount").expect_err("negative");
        assert_eq!(error.code(), "validation");
        assert!(error.message().contains("priceAmount"));
    }

    #[test]
    fn refuses_a_reference_price_below_the_price() {
        assert!(ensure_reference_price_not_lower(60000, Some(80000)).is_ok());
        assert!(ensure_reference_price_not_lower(60000, Some(60000)).is_ok());
        assert!(ensure_reference_price_not_lower(60000, None).is_ok());
        let error =
            ensure_reference_price_not_lower(60000, Some(59999)).expect_err("cheaper reference");
        assert!(error.message().contains("originalPriceAmount"));
    }

    #[test]
    fn business_keys_are_stable_unique_and_free_of_like_wildcards() {
        let first = derived_business_key("spu", "1", "notary-matter:7:abc");
        assert_eq!(
            first,
            derived_business_key("spu", "1", "notary-matter:7:abc"),
            "the same idempotency key must derive the same row"
        );
        assert_ne!(
            first,
            derived_business_key("sku", "1", "notary-matter:7:abc"),
            "a product and its variant must not share a key"
        );
        assert_ne!(
            first,
            derived_business_key("spu", "2", "notary-matter:7:abc"),
            "the key is unique per tenant, so the tenant has to be in the seed"
        );
        // The recovery path looks these up through an `ILIKE`; a wildcard of either kind would make
        // one matter's key match another's row.
        assert!(!first.contains('%') && !first.contains('_'), "{first}");
        assert!(first.starts_with("NOTARY-spu-"), "{first}");
    }

    #[test]
    fn a_stored_count_is_rendered_at_the_scale_the_row_snapshotted() {
        // The registry says CNY is two digits; this row was written at three. The row's count means
        // three, so re-placing the point at the registry's exponent would report 60.000 as 600.00.
        let registry = cny();
        let row = MoneyUnit::custom(registry.code(), 3, registry.rounding()).expect("three digits");
        assert_eq!(render_stored_count(60000, row), "60.000");
        assert_eq!(render_stored_count(60000, registry), "600.00");
    }

    #[test]
    fn preserves_owner_error_classification() {
        assert_eq!(
            map_commerce_error(CommerceServiceError::conflict("duplicate")).code(),
            "conflict"
        );
        assert_eq!(
            map_commerce_error(CommerceServiceError::validation("invalid")).code(),
            "validation"
        );
        assert_eq!(
            map_commerce_error(CommerceServiceError::not_found("gone")).code(),
            "not-found"
        );
    }

    #[test]
    fn refuses_a_spec_that_is_not_an_object() {
        assert!(require_spec_object(&serde_json::json!({})).is_ok());
        let error = require_spec_object(&serde_json::json!("materialCodes")).expect_err("string");
        assert!(error.message().contains("spec"));
    }
}

/// A live-PostgreSQL gate for this adapter.
///
/// # Why this exists when the tests above pass
///
/// Everything above is arithmetic and mapping, and all of it passes without a database. The parts of
/// this adapter that cannot be tested that way are exactly the parts that decide whether it works:
///
/// * the **claim** — that a category created at the tenant level is found again by `category_no`, and
///   that a product filed under it with `product_type = 'service'` is what a narrowed list returns;
/// * the **foreign keys and CHECKs** — that `fulfillment_type = 'service'`,
///   `inventory_tracking = 'none'` and an empty axis set are values PostgreSQL accepts, and that the
///   `variant_signature` fallback survives `uk_commerce_product_sku_variant`;
/// * the **status projection** — that the SPU and the SKU really do end up agreeing, which is the
///   assumption the listing's filter rests on;
/// * the **offset-to-page bridge** — that a `page_size`-sized window taken at an offset which is not
///   a multiple of the page size is the window asked for, and neither a row short nor a row repeated;
/// * **money at the registry's scale** — that `"600.00"` becomes `60000` under the row
///   `commerce_currency` actually holds, and that a zero-exponent currency stays a bare integer.
///
/// Run it against any database that has the merchandise catalog baseline applied:
///
/// ```text
/// export SDKWORK_DATABASE_TEST_POSTGRES_URL='postgresql://sdkwork_ai_dev:sdkworkdev123@127.0.0.1:5432/sdkwork_ai_dev?sslmode=disable'
/// cargo test -p sdkwork-notary-embedded-bootstrap --lib -- --ignored --nocapture
/// ```
///
/// It is `#[ignore]`d because it needs that database, not because it is optional.
///
/// # It writes, and where it writes is chosen so nothing else reads it
///
/// The tenant is a fixed id no other tenant shares, so a notary runtime or a console pointed at the
/// same shared development database cannot see these rows. The titles carry a per-run nonce, so every
/// list this gate makes is narrowed to its own run and re-running it neither inherits earlier rows
/// nor disturbs them. The rows it leaves behind are named `NOTARY-*` and are inert; the tenant they
/// belong to is [`GATE_TENANT_ID`] if they ever need clearing.
#[cfg(test)]
mod postgres_gate {
    use super::*;
    use sdkwork_database_config::{DatabaseConfig, DatabaseEngine};
    use sdkwork_database_id::{current_time_millis, SnowflakeIdGenerator};
    use sdkwork_database_sqlx::PoolContext;

    const ENV: &str = "SDKWORK_DATABASE_TEST_POSTGRES_URL";

    /// A tenant id no seeded data uses, so this gate's rows are invisible to every other reader of a
    /// shared development database.
    const GATE_TENANT_ID: &str = "999000001";
    const GATE_ORGANIZATION_ID: &str = "999000002";

    /// The workspace convention is that a database's schema carries the database's name
    /// (`ENVIRONMENT_SPEC` section 7.1), so it is read from the URL rather than configured a second
    /// time — two places to state one fact is how they come to disagree.
    fn search_path_from_url(url: &str) -> String {
        url.split('?')
            .next()
            .unwrap_or(url)
            .rsplit('/')
            .next()
            .unwrap_or("public")
            .to_owned()
    }

    /// Builds the port over the gate database, or reports what is wrong with it.
    async fn gate_port() -> Option<CommerceOrderPort> {
        let Ok(url) = std::env::var(ENV) else {
            eprintln!("skipped: {ENV} is not set, so there is no database to gate against");
            return None;
        };
        let schema = search_path_from_url(&url);
        let options: sqlx::postgres::PgConnectOptions = url
            .parse()
            .unwrap_or_else(|error| panic!("{ENV} is not a PostgreSQL URL: {error}"));
        // `sqlx::PgPoolOptions` is not re-exported at the crate root once more than one driver is
        // enabled, so the driver-qualified path is the one that always resolves.
        let raw = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .unwrap_or_else(|error| panic!("cannot connect to the database {ENV} names: {error}"));

        // Preflight rather than a confusing `relation does not exist` three assertions later.
        let catalog: Option<String> =
            sqlx::query_scalar("SELECT to_regclass('commerce_product_spu')::text")
                .fetch_one(&raw)
                .await
                .expect("probe for the merchandise catalog");
        assert!(
            catalog.is_some(),
            "{ENV} points at a database whose `{schema}` schema has no merchandise catalog baseline"
        );
        let cny_scale: Option<i16> = sqlx::query_scalar(
            "SELECT minor_unit_exponent FROM commerce_currency WHERE code = 'CNY'",
        )
        .fetch_optional(&raw)
        .await
        .expect("read the CNY registry row");
        assert_eq!(
            cny_scale,
            Some(2),
            "the currency registry this gate measures against is not the one seeded"
        );

        let ids: Arc<dyn IdGenerator> =
            Arc::new(SnowflakeIdGenerator::new(1).expect("node id 1 is within range"));
        let pool = DatabasePool::Postgres(
            raw,
            PoolContext {
                config: DatabaseConfig {
                    engine: DatabaseEngine::Postgres,
                    url,
                    ..DatabaseConfig::default()
                },
            },
        );
        Some(
            CommerceOrderPort::new(pool, ids, GATE_TENANT_ID, GATE_ORGANIZATION_ID)
                .expect("a PostgreSQL pool is what this port requires"),
        )
    }

    fn matter_command(
        nonce: &str,
        slug: &str,
        currency: &str,
        price: &str,
        reference: &str,
    ) -> CommerceMatterCommand {
        CommerceMatterCommand {
            organization_id: Some(GATE_ORGANIZATION_ID.to_owned()),
            title: format!("{nonce} {slug}"),
            description: Some(format!("{nonce} three-day service")),
            price_amount: price.to_owned(),
            original_price_amount: Some(reference.to_owned()),
            currency_code: currency.to_owned(),
            status: "active".to_owned(),
            spec: serde_json::json!({"materialCodes": ["identity", "evidence"]}),
            idempotency_key: format!("{nonce}-{slug}"),
        }
    }

    async fn list_matters(
        port: &CommerceOrderPort,
        nonce: &str,
        page_size: i64,
        offset: i64,
    ) -> CommerceMatterListPage {
        port.list_notary_matters(CommerceMatterListQuery {
            organization_id: Some(GATE_ORGANIZATION_ID.to_owned()),
            search_term: Some(nonce.to_owned()),
            status: None,
            page_size,
            offset,
        })
        .await
        .expect("list notary matters")
    }

    #[tokio::test]
    #[ignore = "requires SDKWORK_DATABASE_TEST_POSTGRES_URL (a database with the merchandise catalog baseline)"]
    async fn a_notary_matter_round_trips_through_the_merchandise_catalog() {
        let Some(port) = gate_port().await else {
            return;
        };
        // Hex, so no byte of the nonce can be read as a `LIKE` wildcard by the recovery path, and
        // drawn from the clock so two runs against the same shared database do not collide.
        let nonce = format!(
            "{:x}",
            current_time_millis().expect("the system clock is at or after the Unix epoch")
        );
        let tenant: i64 = GATE_TENANT_ID
            .parse()
            .expect("the gate tenant is a decimal id");

        // ---------------------------------------------------------------- create
        let created = port
            .create_notary_matter(matter_command(&nonce, "remote", "CNY", "600.00", "800.00"))
            .await
            .expect("create a notary matter");
        assert_eq!(created.price_amount, "600.00");
        assert_eq!(created.original_price_amount.as_deref(), Some("800.00"));
        assert_eq!(created.currency_code, "CNY");
        assert_eq!(created.status, "active");
        assert_eq!(created.title, format!("{nonce} remote"));
        assert_eq!(
            created.description.as_deref(),
            Some(format!("{nonce} three-day service").as_str())
        );
        assert_eq!(
            created.spec,
            serde_json::json!({"materialCodes": ["identity", "evidence"]})
        );
        assert_ne!(created.sku_id, created.spu_id);
        assert!(
            created.sku_no.starts_with("NOTARY-sku-"),
            "{}",
            created.sku_no
        );
        let sku_id: i64 = created.sku_id.parse().expect("a decimal variant id");
        let spu_id: i64 = created.spu_id.parse().expect("a decimal product id");

        // The rows themselves, because a record can be right while the model behind it is not.
        let product_type: String = sqlx::query_scalar(
            "SELECT product_type FROM commerce_product_spu WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant)
        .bind(spu_id)
        .fetch_one(port.catalog.pool())
        .await
        .expect("read the product row");
        assert_eq!(product_type, "service");

        let category_no: String = sqlx::query_scalar(
            "SELECT cat.category_no
               FROM commerce_product_spu spu
               JOIN commerce_product_category cat ON cat.id = spu.category_id
              WHERE spu.tenant_id = $1 AND spu.id = $2",
        )
        .bind(tenant)
        .bind(spu_id)
        .fetch_one(port.catalog.pool())
        .await
        .expect("read the claim category");
        assert_eq!(
            category_no, "NOTARY",
            "the matter must be filed under the notary claim"
        );

        let row: (bool, String, String, i16, i64, i64, bool) = sqlx::query_as(
            "SELECT spu.status = sku.status,
                    sku.fulfillment_type,
                    sku.inventory_tracking,
                    sku.price_scale,
                    sku.sale_price_minor,
                    sku.list_price_minor,
                    jsonb_exists(sku.metadata, 'materialCodes')
               FROM commerce_product_spu spu
               JOIN commerce_product_sku sku
                 ON sku.spu_id = spu.id AND sku.tenant_id = spu.tenant_id
              WHERE spu.tenant_id = $1 AND spu.id = $2",
        )
        .bind(tenant)
        .bind(spu_id)
        .fetch_one(port.catalog.pool())
        .await
        .expect("read the variant row");
        assert!(
            row.0,
            "the status the listing filters on and the status the record reports must be one value"
        );
        assert_eq!(row.1, "service");
        assert_eq!(row.2, "none");
        assert_eq!(
            row.3, 2,
            "the registry exponent is what the price row carries"
        );
        assert_eq!(row.4, 60000);
        assert_eq!(row.5, 80000);
        assert!(
            row.6,
            "the capability metadata must reach the column, not stay in the request"
        );

        let signature_is_the_fallback: bool = sqlx::query_scalar(
            "SELECT variant_signature = sku_no FROM commerce_product_sku WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant)
        .bind(sku_id)
        .fetch_one(port.catalog.pool())
        .await
        .expect("read the variant signature");
        assert!(
            signature_is_the_fallback,
            "a category with no sales axis must fall back to the sku number, not invent an axis"
        );

        // ------------------------------------------------- idempotent replay
        let replayed = port
            .create_notary_matter(matter_command(&nonce, "remote", "CNY", "600.00", "800.00"))
            .await
            .expect("replay the same create");
        assert_eq!(
            (replayed.sku_id, replayed.spu_id),
            (created.sku_id.clone(), created.spu_id.clone()),
            "the same idempotency key must resolve to the same matter, not mint a second one"
        );
        let products_with_the_title: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM commerce_product_spu WHERE tenant_id = $1 AND title = $2",
        )
        .bind(tenant)
        .bind(format!("{nonce} remote"))
        .fetch_one(port.catalog.pool())
        .await
        .expect("count the products behind the replay");
        assert_eq!(
            products_with_the_title, 1,
            "a replayed create must not leave a second product behind"
        );

        // ---------------------------------------------------------------- list
        let first_page = list_matters(&port, &nonce, 10, 0).await;
        assert_eq!(first_page.items.len(), 1);
        assert_eq!(first_page.items[0].sku_id, created.sku_id);
        assert!(!first_page.has_more);

        // -------------------------------------------------------------- update
        let updated = port
            .update_notary_matter(CommerceMatterUpdateCommand {
                organization_id: GATE_ORGANIZATION_ID.to_owned(),
                sku_id: created.sku_id.clone(),
                title: None,
                description: Some(None),
                price_amount: None,
                original_price_amount: Some(None),
                currency_code: None,
                status: Some("inactive".to_owned()),
                spec: None,
            })
            .await
            .expect("clear the description and the reference price, and retire the matter");
        assert_eq!(
            updated.description, None,
            "an explicit null clears the text"
        );
        assert_eq!(
            updated.original_price_amount, None,
            "an explicit null clears the reference price"
        );
        assert_eq!(
            updated.price_amount, "600.00",
            "an unmentioned price is kept"
        );
        assert_eq!(updated.status, "inactive");
        assert_eq!(
            updated.spec,
            serde_json::json!({"materialCodes": ["identity", "evidence"]}),
            "an unmentioned spec is kept"
        );
        let projection_after: bool = sqlx::query_scalar(
            "SELECT spu.status = sku.status
               FROM commerce_product_spu spu
               JOIN commerce_product_sku sku
                 ON sku.spu_id = spu.id AND sku.tenant_id = spu.tenant_id
              WHERE spu.tenant_id = $1 AND spu.id = $2",
        )
        .bind(tenant)
        .bind(spu_id)
        .fetch_one(port.catalog.pool())
        .await
        .expect("re-read the status projection");
        assert!(
            projection_after,
            "an update must move the projection with the value, or the filter lies"
        );

        // ------------------------------------------- the offset-to-page bridge
        for slug in ["alpha", "beta"] {
            port.create_notary_matter(matter_command(&nonce, slug, "CNY", "600.00", "800.00"))
                .await
                .expect("create a matter to page through");
        }
        // Newest first (`created_at DESC, id DESC`), so this run reads beta, alpha, remote — and
        // offset 1 against a page size of 2 is deliberately not a page boundary, which is the case
        // the bridge exists for.
        let head = list_matters(&port, &nonce, 2, 0).await;
        assert_eq!(head.items.len(), 2);
        assert!(
            head.items[0].title.ends_with(" beta"),
            "{}",
            head.items[0].title
        );
        assert!(
            head.items[1].title.ends_with(" alpha"),
            "{}",
            head.items[1].title
        );
        assert!(head.has_more);

        let mid = list_matters(&port, &nonce, 2, 1).await;
        assert_eq!(
            mid.items.len(),
            2,
            "offset 1 of a 2-row page is not a page number"
        );
        assert_eq!(mid.items[0].sku_id, head.items[1].sku_id);
        assert_eq!(mid.items[1].sku_id, created.sku_id);
        assert!(
            !mid.has_more,
            "the window reaches the last matter, so there is nothing after it"
        );

        // ------------------------------------------- a zero-exponent currency
        // The reference price is the higher of the two, as the catalog requires; the point of this
        // pair is the *exponent*, so both amounts are whole units that would gain a spurious `.00` if
        // the adapter ever fell back to a two-decimal assumption instead of reading the registry row.
        let jpy = port
            .create_notary_matter(matter_command(&nonce, "yen", "JPY", "100", "200"))
            .await
            .expect("create a matter priced in a currency with no minor unit");
        assert_eq!(
            jpy.price_amount, "100",
            "a currency whose registry exponent is zero must read back with no point at all"
        );
        assert_eq!(
            jpy.original_price_amount.as_deref(),
            Some("200"),
            "and the reference price carries the same scale, from the same row"
        );

        // The listing narrows by the claim as well as by the search term, so a matter this gate never
        // created cannot appear in it.
        let narrowed = list_matters(&port, &nonce, 20, 0).await;
        assert_eq!(
            narrowed.items.len(),
            4,
            "every matter of this run, and only them"
        );
    }
}
