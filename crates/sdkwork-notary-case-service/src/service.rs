use std::collections::BTreeMap;

use sdkwork_notary_case_contract::{
    now_compact_date, now_iso8601, NotaryCaseCommand, NotaryCaseRecord, NotaryCaseStatus,
    NotaryRuntimeContext, NotaryServiceContract, NotaryServiceError,
};
use sdkwork_utils_rust::{is_blank, PageInfo, PageMode, DEFAULT_LIST_PAGE_SIZE};
use serde_json::{json, Value};

use crate::pagination::{
    decode_offset_cursor, default_list_page_size, encode_offset_cursor, validated_list_page_size,
    NOTARY_MAX_LIST_PAGE_SIZE,
};
use crate::{
    expand_notary_access_permissions, granted_notary_permissions, require_operation_permission,
    AppbaseOrganizationMember, CommerceCreateOrderCommand, CommerceMatterCommand,
    CommerceMatterListQuery, CommerceMatterRecord, CommerceMatterUpdateCommand,
    DriveCreateDownloadPackageCommand, DriveCreateFolderCommand, DriveCreateMonthlyReportCommand,
    DriveCreatePartySignatureInviteCommand, DriveCreatePartyVideoInviteCommand,
    DriveCreateSpaceCommand, DriveDownloadPackageReference, DriveListNodesPage,
    DriveListNodesQuery, DriveNodeReference, DriveRegisterCaseFileCommand,
    NotaryCaseAssignmentCommand, NotaryCaseAssignmentRecord, NotaryCaseEventListQuery,
    NotaryCaseEventRecord, NotaryCaseListPage, NotaryCaseListQuery, NotaryCaseUpdateCommand,
    NotaryDashboardStatisticsAggregate, NotaryDashboardStatisticsQuery,
    NotaryMonthlyCaseCountQuery, NotaryOperationMetadata, NotaryOrganizationProfile,
    NotaryOrganizationProfileUpdateCommand, NotaryPartyListQuery, NotaryPartyRecord,
    NotaryPartyUpdateCommand, NotaryRuntimePorts, NotaryStaffListQuery,
    NOTARY_CASE_REPOSITORY_PORT, NOTARY_COMMERCE_PORT, NOTARY_DRIVE_PORT, NOTARY_IAM_PORT,
};

const ESTIMATED_PROCESS_HOURS_PER_PENDING_CASE: f64 = 2.0;

pub fn notary_runtime_contract() -> NotaryServiceContract {
    NotaryServiceContract::new(
        "notary",
        "notary.case",
        vec![
            "notary.business.open",
            "notary.organizationProfiles.create",
            "notary.organizationProfiles.update",
            "notary.matters.create",
            "notary.matters.update",
            "notary.cases.create",
            "notary.cases.update",
            "notary.cases.acceptances.create",
            "notary.cases.rejections.create",
            "notary.cases.completions.create",
            "notary.cases.assignments.create",
            "notary.cases.assignments.delete",
            "notary.cases.files.create",
            "notary.cases.downloadPackages.create",
            "notary.cases.parties.create",
            "notary.cases.parties.update",
            "notary.cases.parties.delete",
            "notary.cases.parties.signatures.create",
            "notary.cases.parties.videoInvites.create",
            "notary.cases.parties.signatureInvites.create",
        ],
        vec![
            "notary.access.retrieve",
            "notary.organizationProfiles.list",
            "notary.organizationProfiles.retrieve",
            "notary.matters.list",
            "notary.matters.management.list",
            "notary.staff.list",
            "notary.cases.list",
            "notary.cases.retrieve",
            "notary.cases.management.list",
            "notary.cases.management.retrieve",
            "notary.cases.parties.list",
            "notary.cases.files.list",
            "notary.cases.events.list",
            "notary.reports.caseSummary.retrieve",
            "notary.dashboard.statistics.retrieve",
            "notary.reports.monthly.retrieve",
        ],
        vec![
            NOTARY_IAM_PORT,
            NOTARY_COMMERCE_PORT,
            NOTARY_DRIVE_PORT,
            NOTARY_CASE_REPOSITORY_PORT,
        ],
        true,
    )
}

pub async fn ensure_notary_business_open(
    context: &NotaryRuntimeContext,
    organization_id: &str,
    opened_by_membership_id: &str,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<NotaryOrganizationProfile, NotaryServiceError> {
    validate_context(context)?;
    let member = require_notary_member(
        ports.appbase,
        organization_id,
        opened_by_membership_id,
        true,
    )
    .await?;
    if member.user_id != context.user_id {
        return Err(NotaryServiceError::unauthorized(
            "organization member does not match current user",
        ));
    }

    let drive_space_id = ports
        .drive
        .create_notary_space(DriveCreateSpaceCommand {
            owner_subject_type: "organization".to_string(),
            owner_subject_id: organization_id.to_string(),
            space_type: "notary".to_string(),
            display_name: "Notary".to_string(),
        })
        .await?;

    ports
        .repository
        .upsert_organization_profile(organization_id, &drive_space_id, "notary")
        .await
}

pub async fn create_notary_case(
    context: &NotaryRuntimeContext,
    command: NotaryCaseCommand,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<NotaryCaseRecord, NotaryServiceError> {
    validate_context(context)?;
    validate_create_case_command(&command)?;
    ensure_organization_scope(context, &command.organization_id)?;

    let creator_membership_id = context
        .membership_id
        .as_deref()
        .ok_or_else(|| NotaryServiceError::unauthorized("membership_id is required"))?;
    let creator = require_notary_member(
        ports.appbase,
        &command.organization_id,
        creator_membership_id,
        false,
    )
    .await?;
    if creator.user_id != context.user_id {
        return Err(NotaryServiceError::unauthorized(
            "organization member does not match current user",
        ));
    }

    if let Some(existing) = ports
        .repository
        .get_case_by_idempotency_key(&command.idempotency_key)
        .await?
    {
        ensure_case_access(context, &existing)?;
        return Ok(existing);
    }

    let primary_notary_member =
        if let Some(membership_id) = command.primary_notary_membership_id.as_deref() {
            if membership_id == creator_membership_id {
                creator.clone()
            } else {
                require_notary_member(
                    ports.appbase,
                    &command.organization_id,
                    membership_id,
                    false,
                )
                .await?
            }
        } else {
            creator.clone()
        };

    let primary_notary_membership_id = Some(primary_notary_member.membership_id.clone());
    let primary_notary_user_id = Some(primary_notary_member.user_id.clone());
    let primary_notary_name = Some(member_display_name(&primary_notary_member));

    let profile = ports
        .repository
        .get_organization_profile(&command.organization_id)
        .await?
        .ok_or_else(|| NotaryServiceError::not_found("notary business is not open"))?;

    if profile.drive_space_type != "notary" {
        return Err(NotaryServiceError::invalid_state(
            "notary organization profile must use drive space_type notary",
        ));
    }

    let order = ports
        .commerce
        .create_notary_order(CommerceCreateOrderCommand {
            organization_id: command.organization_id.clone(),
            sku_id: command.sku_id.clone(),
            title: command.title.clone(),
            applicant_name: command.applicant_name.clone(),
            product_type: "notary".to_string(),
            idempotency_key: command.idempotency_key.clone(),
        })
        .await?;

    let case_id = format!("case-{}", order.order_item_id);
    let folder = match ports
        .drive
        .create_case_folder(DriveCreateFolderCommand {
            space_id: profile.drive_space_id.clone(),
            space_type: "notary".to_string(),
            parent_node_id: None,
            folder_name: command
                .drive_folder_name
                .clone()
                .unwrap_or_else(|| command.title.clone()),
            order_id: order.order_id.clone(),
            case_id: case_id.clone(),
        })
        .await
    {
        Ok(folder) => folder,
        Err(error) => {
            let _ = ports
                .commerce
                .cancel_notary_order(&command.organization_id, &order.order_id)
                .await;
            return Err(error);
        }
    };

    let folder_node_id = folder.folder_node_id.clone();
    let folder_space_id = folder.space_id.clone();
    let folder_space_type = folder.space_type.clone();

    let now = now_iso8601();
    let record = NotaryCaseRecord {
        case_id: case_id.clone(),
        case_no: build_case_no(&order.order_item_id),
        organization_id: command.organization_id.clone(),
        title: command.title,
        applicant_name: command.applicant_name,
        primary_notary_membership_id,
        primary_notary_user_id,
        primary_notary_name,
        status: NotaryCaseStatus::PendingReview,
        order_id: order.order_id.clone(),
        order_item_id: order.order_item_id.clone(),
        sku_id: order.sku_id.clone(),
        matter_title: order.matter_title,
        fee_amount: order.fee_amount,
        currency_code: order.currency_code,
        drive_space_id: folder.space_id,
        drive_space_type: folder.space_type,
        drive_folder_node_id: folder.folder_node_id,
        chain_hash: None,
        remarks: command.remarks,
        request_no: build_case_no(&order.order_item_id),
        idempotency_key: command.idempotency_key,
        version: 1,
        created_at: now.clone(),
        updated_at: now,
    };

    let inserted = match ports.repository.insert_case(record).await {
        Ok(inserted) => inserted,
        Err(error) => {
            let _ = ports
                .drive
                .delete_case_folder(&folder_node_id, &folder_space_id, &folder_space_type)
                .await;
            let _ = ports
                .commerce
                .cancel_notary_order(&command.organization_id, &order.order_id)
                .await;
            return Err(error);
        }
    };
    for party in &command.parties {
        if let Err(error) = ports
            .repository
            .insert_party(
                &inserted.case_id,
                party,
                &inserted.order_id,
                &inserted.order_item_id,
                &inserted.sku_id,
            )
            .await
        {
            let _ = ports.repository.delete_case(&inserted.case_id).await;
            let _ = ports
                .drive
                .delete_case_folder(&folder_node_id, &folder_space_id, &folder_space_type)
                .await;
            let _ = ports
                .commerce
                .cancel_notary_order(&inserted.organization_id, &order.order_id)
                .await;
            return Err(error);
        }
    }
    ports
        .repository
        .append_event(&inserted.case_id, "notary.case.submitted")
        .await?;

    Ok(inserted)
}

pub async fn list_case_files(
    context: &NotaryRuntimeContext,
    case_id: &str,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<Vec<DriveNodeReference>, NotaryServiceError> {
    validate_context(context)?;
    let record = load_case_for_context(context, ports, case_id).await?;

    ports
        .drive
        .list_nodes(DriveListNodesQuery {
            space_id: record.drive_space_id,
            space_type: record.drive_space_type,
            parent_node_id: record.drive_folder_node_id,
            category: None,
            page_size: 50,
            cursor: None,
        })
        .await
        .map(|page| page.items)
}

pub async fn handle_notary_app_operation(
    context: &NotaryRuntimeContext,
    operation_id: &str,
    path_params: BTreeMap<String, String>,
    body: Value,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<Value, NotaryServiceError> {
    handle_notary_app_operation_with_metadata(
        context,
        operation_id,
        path_params,
        body,
        &NotaryOperationMetadata::default(),
        ports,
    )
    .await
}

pub async fn handle_notary_app_operation_with_metadata(
    context: &NotaryRuntimeContext,
    operation_id: &str,
    path_params: BTreeMap<String, String>,
    body: Value,
    metadata: &NotaryOperationMetadata,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<Value, NotaryServiceError> {
    require_operation_permission(context, operation_id)?;
    tracing::info!(
        operation_id,
        tenant_id = %context.tenant_id,
        organization_id = ?context.organization_id,
        "notary app operation"
    );
    match operation_id {
        "notary.access.retrieve" => retrieve_notary_access(context, ports).await,
        "notary.matters.list" => list_notary_matters(context, &body, ports).await,
        "notary.dashboard.statistics.retrieve" => {
            ensure_report_staff_access(context, &body, ports).await?;
            let organization_id = report_organization_id(context, &body)?;
            let statistics = ports
                .repository
                .get_dashboard_statistics(NotaryDashboardStatisticsQuery { organization_id })
                .await?;
            Ok(notary_statistics_to_value(&statistics))
        }
        "notary.reports.monthly.retrieve" => {
            ensure_report_staff_access(context, &body, ports).await?;
            let organization_id = report_organization_id(context, &body)?;
            let month =
                string_field(&body, &["month"]).unwrap_or_else(|| now_iso8601()[..7].to_string());
            let query = NotaryMonthlyCaseCountQuery::new(organization_id, month)?;
            let month = query.month().to_string();
            let count = ports.repository.count_cases_for_month(query).await?;
            monthly_report_to_value(&month, count.count, &body, ports).await
        }
        "notary.cases.create" => {
            let command =
                create_case_command_from_body(context, &body, metadata.idempotency_key.as_deref())?;
            let record = create_notary_case(context, command, ports).await?;
            Ok(case_record_to_value(&record))
        }
        "notary.staff.list" => list_notary_staff(context, &body, ports).await,
        "notary.cases.list" => {
            let organization_id = string_field(&body, &["organizationId", "organization_id"])
                .or_else(|| context.organization_id.clone())
                .unwrap_or_else(|| "0".to_string());
            ensure_organization_scope(context, &organization_id)?;
            let page_size =
                page_size_field(&body, &["pageSize", "page_size"], default_list_page_size())?;
            let page = ports
                .repository
                .list_cases(NotaryCaseListQuery {
                    organization_id,
                    status: string_field(&body, &["status"]).map(|value| status_to_storage(&value)),
                    sku_id: string_field(&body, &["skuId", "sku_id"]),
                    search_term: string_field(&body, &["q", "searchTerm", "search_term"]),
                    page_size,
                    cursor: string_field(&body, &["cursor"]),
                })
                .await?;
            Ok(json!({
                "items": page
                    .items
                    .iter()
                    .map(case_record_to_value)
                    .collect::<Vec<_>>(),
                "pageInfo": cursor_page_info_with_total(
                    page_size,
                    page.has_more,
                    page.next_cursor.as_deref(),
                    page.total_items,
                ),
            }))
        }
        "notary.cases.retrieve" => {
            let case_id = path_param(&path_params, "caseId")?;
            let record = load_case_for_context(context, ports, case_id).await?;
            case_detail_to_value(&record, ports).await
        }
        "notary.cases.update" => {
            let case_id = path_param(&path_params, "caseId")?;
            let record = update_case_from_body(context, case_id, &body, ports).await?;
            case_detail_to_value(&record, ports).await
        }
        "notary.cases.acceptances.create" => {
            let case_id = path_param(&path_params, "caseId")?;
            let current = load_case_for_staff_mutation(context, ports, case_id).await?;
            if !current.status.allows_acceptance() {
                return Err(NotaryServiceError::invalid_state(
                    "only pending_review cases can be accepted",
                ));
            }
            let expected_version = version_field(&body)?.unwrap_or(current.version);
            let order_state = ports
                .commerce
                .get_notary_order_fulfillment_state(
                    current.organization_id.as_str(),
                    current.order_id.as_str(),
                )
                .await?;
            if order_status_is_terminal(order_state.order_status.as_str()) {
                return Err(NotaryServiceError::conflict(
                    "notary case cannot be accepted for a terminal commerce order",
                ));
            }
            let payable_amount = order_state.payable_amount.trim();
            let payment_succeeded = order_state
                .payment_status
                .as_deref()
                .is_some_and(|status| status.trim().eq_ignore_ascii_case("success"));
            let zero_total =
                !payable_amount.is_empty() && payable_amount.bytes().all(|byte| byte == b'0');
            if !payment_succeeded && !zero_total {
                return Err(NotaryServiceError::conflict(
                    "notary case cannot be accepted before order payment succeeds",
                ));
            }
            let record = ports
                .repository
                .update_case(NotaryCaseUpdateCommand {
                    case_id: case_id.to_string(),
                    expected_version,
                    title: None,
                    remarks: string_field(&body, &["remarks"]),
                    status: Some(NotaryCaseStatus::Processing),
                    chain_hash: None,
                    reject_reason: None,
                    event_type: "notary.case.accepted".to_string(),
                })
                .await?;
            case_detail_to_value(&record, ports).await
        }
        "notary.cases.rejections.create" => {
            let case_id = path_param(&path_params, "caseId")?;
            let current = load_case_for_staff_mutation(context, ports, case_id).await?;
            if !current.status.allows_rejection() {
                return Err(NotaryServiceError::invalid_state(
                    "only pending_review or processing cases can be rejected",
                ));
            }
            let reason = string_field(&body, &["reason"])
                .ok_or_else(|| NotaryServiceError::validation("reason is required"))?;
            let record = ports
                .repository
                .update_case(NotaryCaseUpdateCommand {
                    case_id: case_id.to_string(),
                    expected_version: version_field(&body)?.unwrap_or(current.version),
                    title: None,
                    remarks: None,
                    status: Some(NotaryCaseStatus::Rejected),
                    chain_hash: None,
                    reject_reason: Some(reason),
                    event_type: "notary.case.rejected".to_string(),
                })
                .await?;
            case_detail_to_value(&record, ports).await
        }
        "notary.cases.completions.create" => {
            let case_id = path_param(&path_params, "caseId")?;
            let current = load_case_for_staff_mutation(context, ports, case_id).await?;
            if !current.status.allows_completion() {
                return Err(NotaryServiceError::invalid_state(
                    "only processing cases can be completed",
                ));
            }
            let record = ports
                .repository
                .update_case(NotaryCaseUpdateCommand {
                    case_id: case_id.to_string(),
                    expected_version: version_field(&body)?.unwrap_or(current.version),
                    title: None,
                    remarks: string_field(&body, &["remarks"]),
                    status: Some(NotaryCaseStatus::Completed),
                    chain_hash: string_field(&body, &["chainHash", "chain_hash"]),
                    reject_reason: None,
                    event_type: "notary.case.completed".to_string(),
                })
                .await?;
            case_detail_to_value(&record, ports).await
        }
        "notary.cases.assignments.create" => {
            create_case_assignment(context, &path_params, &body, ports).await
        }
        "notary.cases.parties.list" => {
            let case_id = path_param(&path_params, "caseId")?;
            load_case_for_context(context, ports, case_id).await?;
            let page_size =
                page_size_field(&body, &["pageSize", "page_size"], default_list_page_size())?;
            let page = ports
                .repository
                .list_parties(NotaryPartyListQuery {
                    case_id: case_id.to_string(),
                    page_size,
                    cursor: string_field(&body, &["cursor"]),
                })
                .await?;
            Ok(json!({
                "items": page
                    .items
                    .iter()
                    .map(party_record_to_value)
                    .collect::<Vec<_>>(),
                "pageInfo": cursor_page_info(page_size, page.has_more, page.next_cursor.as_deref()),
            }))
        }
        "notary.cases.parties.create" => {
            let case_id = path_param(&path_params, "caseId")?.to_string();
            let record = load_mutable_case_for_staff(context, ports, &case_id).await?;
            let party = party_command_from_value(&body)?;
            ports
                .repository
                .insert_party(
                    &record.case_id,
                    &party,
                    &record.order_id,
                    &record.order_item_id,
                    &record.sku_id,
                )
                .await?;
            ports
                .repository
                .append_event(&record.case_id, "notary.party.created")
                .await?;
            let parties = list_all_parties(&record.case_id, ports).await?;
            let created = parties
                .last()
                .ok_or_else(|| NotaryServiceError::storage("party missing after create"))?;
            Ok(party_record_to_value(created))
        }
        "notary.cases.parties.update" => {
            let case_id = path_param(&path_params, "caseId")?.to_string();
            let party_id = path_param(&path_params, "partyId")?.to_string();
            load_mutable_case_for_staff(context, ports, &case_id).await?;
            let party = ports
                .repository
                .update_party(party_update_command_from_body(&case_id, &party_id, &body))
                .await?;
            ports
                .repository
                .append_event(&case_id, "notary.party.updated")
                .await?;
            Ok(party_record_to_value(&party))
        }
        "notary.cases.parties.delete" => {
            let case_id = path_param(&path_params, "caseId")?;
            let party_id = path_param(&path_params, "partyId")?;
            load_mutable_case_for_staff(context, ports, case_id).await?;
            ports.repository.remove_party(case_id, party_id).await?;
            ports
                .repository
                .append_event(case_id, "notary.party.removed")
                .await?;
            Ok(Value::Null)
        }
        "notary.cases.parties.signatures.create" => {
            let case_id = path_param(&path_params, "caseId")?.to_string();
            let party_id = path_param(&path_params, "partyId")?.to_string();
            load_mutable_case_for_staff(context, ports, &case_id).await?;
            let drive_node_id = string_field(
                &body,
                &[
                    "signatureNodeId",
                    "signature_node_id",
                    "driveNodeId",
                    "drive_node_id",
                    "nodeId",
                ],
            )
            .ok_or_else(|| NotaryServiceError::validation("signatureNodeId is required"))?;
            let party = ports
                .repository
                .update_party(NotaryPartyUpdateCommand {
                    case_id: case_id.clone(),
                    party_id,
                    name: None,
                    party_role: None,
                    identity_no: None,
                    phone: None,
                    signature_node_id: Some(drive_node_id),
                })
                .await?;
            ports
                .repository
                .append_event(&case_id, "notary.party.signature_attached")
                .await?;
            Ok(party_record_to_value(&party))
        }
        "notary.cases.parties.videoInvites.create" => {
            let case_id = path_param(&path_params, "caseId")?;
            let party_id = path_param(&path_params, "partyId")?;
            create_party_video_invite(context, case_id, party_id, &body, ports).await
        }
        "notary.cases.parties.signatureInvites.create" => {
            let case_id = path_param(&path_params, "caseId")?;
            let party_id = path_param(&path_params, "partyId")?;
            create_party_signature_invite(context, case_id, party_id, &body, ports).await
        }
        "notary.cases.files.list" => {
            let case_id = path_param(&path_params, "caseId")?;
            let record = load_case_for_context(context, ports, case_id).await?;
            let page_size = page_size_field(&body, &["pageSize", "page_size"], 50)?;
            let file_page = ports
                .drive
                .list_nodes(DriveListNodesQuery {
                    space_id: record.drive_space_id.clone(),
                    space_type: record.drive_space_type.clone(),
                    parent_node_id: record.drive_folder_node_id.clone(),
                    category: string_field(&body, &["category"]),
                    page_size,
                    cursor: string_field(&body, &["cursor"]),
                })
                .await?;
            Ok(json!({
                "items": file_page
                    .items
                    .iter()
                    .map(|file| drive_node_to_document_value(file, &record))
                    .collect::<Vec<_>>(),
                "pageInfo": drive_list_page_info(&file_page, page_size)
            }))
        }
        "notary.cases.files.create" => {
            let case_id = path_param(&path_params, "caseId")?;
            let record = load_mutable_case_for_staff(context, ports, case_id).await?;
            let node_id = string_field(&body, &["driveNodeId", "drive_node_id", "nodeId"])
                .ok_or_else(|| NotaryServiceError::validation("driveNodeId is required"))?;
            let category =
                string_field(&body, &["category"]).unwrap_or_else(|| "evidence".to_string());
            let review_status = string_field(&body, &["reviewStatus", "review_status", "status"])
                .unwrap_or_else(|| "pending".to_string());
            ports
                .drive
                .register_case_file(DriveRegisterCaseFileCommand {
                    space_id: record.drive_space_id.clone(),
                    node_id: node_id.clone(),
                    category,
                    review_status,
                    material_code: string_field(&body, &["materialCode", "material_code"]),
                    party_id: string_field(&body, &["partyId", "party_id"]),
                })
                .await?;
            let document = file_create_to_document_value(&body, &record)?;
            ports
                .repository
                .append_event(&record.case_id, "notary.case.file_attached")
                .await?;
            Ok(document)
        }
        "notary.cases.downloadPackages.create" => {
            let case_id = path_param(&path_params, "caseId")?;
            let record = load_case_for_staff_mutation(context, ports, case_id).await?;
            let package_name = string_field(&body, &["packageName", "package_name"])
                .unwrap_or_else(|| format!("{}.zip", record.case_no));
            let node_ids = body
                .get("nodeIds")
                .or_else(|| body.get("node_ids"))
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let package = ports
                .drive
                .create_download_package(DriveCreateDownloadPackageCommand {
                    space_id: record.drive_space_id.clone(),
                    space_type: record.drive_space_type.clone(),
                    case_id: record.case_id.clone(),
                    node_ids,
                    package_name,
                })
                .await?;
            ports
                .repository
                .append_event(&record.case_id, "notary.case.download_package_requested")
                .await?;
            Ok(download_package_to_value(&package))
        }
        "notary.cases.events.list" => {
            let case_id = path_param(&path_params, "caseId")?;
            load_case_for_context(context, ports, case_id).await?;
            let page_size =
                page_size_field(&body, &["pageSize", "page_size"], default_list_page_size())?;
            let page = ports
                .repository
                .list_events(NotaryCaseEventListQuery {
                    case_id: case_id.to_string(),
                    page_size,
                    cursor: string_field(&body, &["cursor"]),
                })
                .await?;
            Ok(json!({
                "items": page
                    .items
                    .iter()
                    .map(event_record_to_value)
                    .collect::<Vec<_>>(),
                "pageInfo": cursor_page_info(page_size, page.has_more, page.next_cursor.as_deref()),
            }))
        }
        _ => Err(NotaryServiceError::provider_unavailable(format!(
            "unsupported notary app operation: {operation_id}"
        ))),
    }
}

pub async fn handle_notary_backend_operation(
    context: &NotaryRuntimeContext,
    operation_id: &str,
    path_params: BTreeMap<String, String>,
    body: Value,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<Value, NotaryServiceError> {
    require_operation_permission(context, operation_id)?;
    tracing::info!(
        operation_id,
        tenant_id = %context.tenant_id,
        organization_id = ?context.organization_id,
        "notary backend operation"
    );
    match operation_id {
        "notary.organizationProfiles.create" => {
            let organization_id = string_field(&body, &["organizationId", "organization_id"])
                .or_else(|| context.organization_id.clone())
                .unwrap_or_else(|| "0".to_string());
            let opened_by_membership_id = string_field(
                &body,
                &[
                    "openedByMembershipId",
                    "opened_by_membership_id",
                    "membershipId",
                ],
            )
            .or_else(|| context.membership_id.clone())
            .ok_or_else(|| NotaryServiceError::validation("openedByMembershipId is required"))?;
            let profile = ensure_notary_business_open(
                context,
                &organization_id,
                &opened_by_membership_id,
                ports,
            )
            .await?;
            Ok(organization_profile_to_value(&profile, context))
        }
        "notary.organizationProfiles.list" => {
            // `API_SPEC.md` §10.4: the organization scope of a list is the
            // verified session scope, never a request field. A body
            // `organizationId` is only a target identifier and must match that
            // scope; the SQL predicate always binds the resolved scope.
            if let Some(requested) = string_field(&body, &["organizationId", "organization_id"]) {
                ensure_organization_scope(context, &requested)?;
            }
            let organization_id = context
                .organization_id
                .clone()
                .unwrap_or_else(|| "0".to_owned());
            let page_size = page_size_field(
                &body,
                &["pageSize", "page_size"],
                i64::from(DEFAULT_LIST_PAGE_SIZE),
            )?;
            let page = ports
                .repository
                .list_organization_profiles(
                    Some(organization_id.as_str()),
                    page_size,
                    string_field(&body, &["cursor"]).as_deref(),
                )
                .await?;
            Ok(json!({
                "items": page
                    .items
                    .iter()
                    .map(|profile| organization_profile_to_value(profile, context))
                    .collect::<Vec<_>>(),
                "pageInfo": cursor_page_info(page_size, page.has_more, page.next_cursor.as_deref()),
            }))
        }
        "notary.organizationProfiles.retrieve" => {
            let organization_id = path_param(&path_params, "organizationProfileId")?;
            ensure_organization_scope(context, organization_id)?;
            let profile = ports
                .repository
                .get_organization_profile(organization_id)
                .await?
                .ok_or_else(|| {
                    NotaryServiceError::not_found("notary organization profile not found")
                })?;
            Ok(organization_profile_to_value(&profile, context))
        }
        "notary.organizationProfiles.update" => {
            let organization_id = path_param(&path_params, "organizationProfileId")?.to_string();
            require_notary_admin_for_org(context, &organization_id, ports).await?;
            let status = string_field(&body, &["status"]);
            if let Some(status) = status.as_deref() {
                validate_profile_status(status)?;
            }
            let profile = ports
                .repository
                .update_organization_profile(NotaryOrganizationProfileUpdateCommand {
                    organization_id,
                    status,
                    settings: body.get("settings").cloned(),
                })
                .await?;
            Ok(organization_profile_to_value(&profile, context))
        }
        "notary.matters.management.list" => list_notary_matters(context, &body, ports).await,
        "notary.matters.create" => {
            let organization_id = string_field(&body, &["organizationId", "organization_id"])
                .or_else(|| context.organization_id.clone())
                .unwrap_or_else(|| "0".to_string());
            require_notary_admin_for_org(context, &organization_id, ports).await?;
            let matter = ports
                .commerce
                .create_notary_matter(matter_command_from_body(context, &body)?)
                .await?;
            Ok(matter_record_to_value(&matter))
        }
        "notary.matters.update" => {
            let sku_id = path_param(&path_params, "skuId")?.to_string();
            let organization_id = context
                .organization_id
                .clone()
                .unwrap_or_else(|| "0".to_string());
            require_notary_admin_for_org(context, &organization_id, ports).await?;
            let matter = ports
                .commerce
                .update_notary_matter(matter_update_command_from_body(
                    &organization_id,
                    &sku_id,
                    &body,
                )?)
                .await?;
            Ok(matter_record_to_value(&matter))
        }
        "notary.cases.management.list" => {
            let (page, page_size) = list_backend_cases(context, &body, ports).await?;
            Ok(json!({
                "items": page.items.iter().map(case_record_to_value).collect::<Vec<_>>(),
                "pageInfo": cursor_page_info_with_total(
                    page_size,
                    page.has_more,
                    page.next_cursor.as_deref(),
                    page.total_items,
                ),
            }))
        }
        "notary.cases.management.retrieve" => {
            let case_id = path_param(&path_params, "caseId")?;
            let record = load_case_for_context(context, ports, case_id).await?;
            case_detail_to_value(&record, ports).await
        }
        "notary.cases.assignments.create" => {
            create_case_assignment(context, &path_params, &body, ports).await
        }
        "notary.cases.assignments.delete" => {
            let case_id = path_param(&path_params, "caseId")?.to_string();
            let assignment_id = path_param(&path_params, "assignmentId")?.to_string();
            load_mutable_case_for_staff(context, ports, &case_id).await?;
            ports
                .repository
                .release_assignment(&case_id, &assignment_id)
                .await?;
            ports
                .repository
                .append_event(&case_id, "notary.case.assignment_released")
                .await?;
            Ok(Value::Null)
        }
        "notary.staff.list" => list_notary_staff(context, &body, ports).await,
        "notary.reports.caseSummary.retrieve" => {
            let (page, _) = list_backend_cases(context, &body, ports).await?;
            Ok(case_summary_to_value(&page.items))
        }
        _ => Err(NotaryServiceError::provider_unavailable(format!(
            "unsupported notary backend operation: {operation_id}"
        ))),
    }
}

async fn list_backend_cases(
    context: &NotaryRuntimeContext,
    body: &Value,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<(NotaryCaseListPage, i64), NotaryServiceError> {
    let organization_id = string_field(body, &["organizationId", "organization_id"])
        .or_else(|| context.organization_id.clone())
        .unwrap_or_else(|| "0".to_string());
    ensure_organization_scope(context, &organization_id)?;
    let page_size = page_size_field(body, &["pageSize", "page_size"], 20)?;
    let page = ports
        .repository
        .list_cases(NotaryCaseListQuery {
            organization_id,
            status: string_field(body, &["status"]).map(|value| status_to_storage(&value)),
            sku_id: None,
            search_term: string_field(body, &["q", "searchTerm", "search_term"]),
            page_size,
            cursor: string_field(body, &["cursor"]),
        })
        .await?;
    Ok((page, page_size))
}

async fn create_case_assignment(
    context: &NotaryRuntimeContext,
    path_params: &BTreeMap<String, String>,
    body: &Value,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<Value, NotaryServiceError> {
    let case_id = path_param(path_params, "caseId")?.to_string();
    let record = load_mutable_case_for_staff(context, ports, &case_id).await?;
    let organization_membership_id = string_field(
        body,
        &[
            "organizationMembershipId",
            "organization_membership_id",
            "membershipId",
        ],
    )
    .ok_or_else(|| NotaryServiceError::validation("organizationMembershipId is required"))?;
    let assignment_role = string_field(body, &["assignmentRole", "assignment_role"])
        .ok_or_else(|| NotaryServiceError::validation("assignmentRole is required"))?;
    validate_assignment_role(&assignment_role)?;
    let member = require_notary_member(
        ports.appbase,
        &record.organization_id,
        &organization_membership_id,
        false,
    )
    .await?;
    let display_name = member_display_name(&member);
    let assignment = ports
        .repository
        .insert_assignment(NotaryCaseAssignmentCommand {
            case_id: case_id.clone(),
            organization_id: record.organization_id,
            organization_membership_id,
            user_id: member.user_id,
            assignment_role,
            assigned_by_membership_id: context.membership_id.clone(),
        })
        .await?;
    ports
        .repository
        .append_event(&case_id, "notary.case.assignment_created")
        .await?;
    Ok(assignment_record_to_value(&assignment, Some(display_name)))
}

async fn list_notary_staff(
    context: &NotaryRuntimeContext,
    body: &Value,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<Value, NotaryServiceError> {
    let organization_id = string_field(body, &["organizationId", "organization_id"])
        .or_else(|| context.organization_id.clone())
        .unwrap_or_else(|| "0".to_string());
    ensure_organization_scope(context, &organization_id)?;
    let staff_role = string_field(body, &["staffRole", "staff_role"]);
    let page_size = page_size_field(body, &["pageSize", "page_size"], default_list_page_size())?;
    let offset = decode_offset_cursor(string_field(body, &["cursor"]).as_deref())?;
    let page = ports
        .appbase
        .list_notary_staff_page(NotaryStaffListQuery {
            organization_id,
            staff_role,
            page_size,
            offset,
        })
        .await?;
    Ok(json!({
        "items": page
            .items
            .iter()
            .map(staff_member_to_value)
            .collect::<Vec<_>>(),
        "pageInfo": cursor_page_info(
            page_size,
            page.has_more,
            page.has_more
                .then(|| encode_offset_cursor(page.next_offset))
                .as_deref(),
        ),
    }))
}

async fn retrieve_notary_access(
    context: &NotaryRuntimeContext,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<Value, NotaryServiceError> {
    validate_context(context)?;
    let organization_id = context.organization_id.clone().unwrap_or_default();
    let membership_id = context.membership_id.clone().unwrap_or_default();
    if is_blank(Some(organization_id.as_str())) || is_blank(Some(membership_id.as_str())) {
        return Ok(notary_access_value(
            context,
            None,
            false,
            "organization membership is required",
        ));
    }

    let member = ports
        .appbase
        .get_organization_member(&organization_id, &membership_id)
        .await?;
    let Some(member) = member else {
        return Ok(notary_access_value(
            context,
            None,
            false,
            "organization member is required",
        ));
    };

    let profile = ports
        .repository
        .get_organization_profile(&organization_id)
        .await?;
    let profile_active = profile
        .as_ref()
        .is_some_and(|profile| profile.status == "active" && profile.drive_space_type == "notary");
    let visible = member.enterprise_verified && member.notary_enabled && profile_active;
    let reason = if visible {
        ""
    } else if !member.enterprise_verified {
        "organization member is not enterprise verified"
    } else if !member.notary_enabled {
        "organization member is not enabled for notary business"
    } else {
        "notary business is not open"
    };

    Ok(notary_access_value(context, Some(&member), visible, reason))
}

async fn list_notary_matters(
    context: &NotaryRuntimeContext,
    body: &Value,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<Value, NotaryServiceError> {
    validate_context(context)?;
    let organization_id = string_field(body, &["organizationId", "organization_id"])
        .or_else(|| context.organization_id.clone())
        .unwrap_or_else(|| "0".to_string());
    ensure_organization_scope(context, &organization_id)?;
    let page_size = page_size_field(body, &["pageSize", "page_size"], default_list_page_size())?;
    let offset = decode_offset_cursor(string_field(body, &["cursor"]).as_deref())?;
    let page = ports
        .commerce
        .list_notary_matters(CommerceMatterListQuery {
            organization_id: Some(organization_id),
            search_term: string_field(body, &["q", "searchTerm", "search_term"]),
            status: string_field(body, &["status"]),
            page_size,
            offset,
        })
        .await?;
    let item_count = page.items.len() as i64;
    let items = page
        .items
        .into_iter()
        .map(|matter| matter_record_to_value(&matter))
        .collect::<Vec<_>>();
    let next_cursor = page
        .has_more
        .then(|| encode_offset_cursor(offset.saturating_add(item_count)));
    Ok(json!({
        "items": items,
        "pageInfo": cursor_page_info(page_size, page.has_more, next_cursor.as_deref()),
    }))
}

fn notary_access_value(
    context: &NotaryRuntimeContext,
    member: Option<&AppbaseOrganizationMember>,
    visible: bool,
    reason: &str,
) -> Value {
    let permissions = if visible {
        expand_notary_access_permissions(&granted_notary_permissions(context))
    } else {
        Vec::new()
    };
    json!({
        "visible": visible,
        "organizationVerified": member.is_some_and(|member| member.enterprise_verified),
        "notaryBusinessEnabled": visible,
        "tenantId": context.tenant_id,
        "organizationId": context.organization_id,
        "memberId": member
            .map(|member| member.membership_id.clone())
            .or_else(|| context.membership_id.clone())
            .unwrap_or_default(),
        "roles": member.map(|member| member.roles.clone()).unwrap_or_default(),
        "positions": member.map(|member| member.positions.clone()).unwrap_or_default(),
        "departments": member.map(|member| member.departments.clone()).unwrap_or_default(),
        "permissions": permissions,
        "reason": reason
    })
}

fn matter_record_to_value(record: &CommerceMatterRecord) -> Value {
    json!({
        "skuId": record.sku_id,
        "spuId": record.spu_id,
        "skuNo": record.sku_no,
        "title": record.title,
        "description": record.description,
        "priceAmount": record.price_amount,
        "originalPriceAmount": record.original_price_amount,
        "currencyCode": record.currency_code,
        "status": record.status,
        "spec": record.spec
    })
}

fn assignment_record_to_value(
    record: &NotaryCaseAssignmentRecord,
    display_name: Option<String>,
) -> Value {
    json!({
        "id": record.assignment_id,
        "caseId": record.case_id,
        "organizationMembershipId": record.organization_membership_id,
        "userId": record.user_id,
        "displayName": display_name.unwrap_or_else(|| record.organization_membership_id.clone()),
        "assignmentRole": record.assignment_role,
        "status": record.status,
        "assignedAt": record.assigned_at
    })
}

fn organization_profile_to_value(
    profile: &NotaryOrganizationProfile,
    context: &NotaryRuntimeContext,
) -> Value {
    json!({
        "id": format!("notary-profile-{}", profile.organization_id),
        "tenantId": context.tenant_id,
        "organizationId": profile.organization_id,
        "status": profile.status,
        "driveSpaceId": profile.drive_space_id,
        "driveSpaceType": profile.drive_space_type,
        "openedAt": now_iso8601(),
        "settings": {
            "driveSpaceType": profile.drive_space_type
        },
        "version": "1"
    })
}

fn staff_member_to_value(member: &AppbaseOrganizationMember) -> Value {
    json!({
        "membershipId": member.membership_id,
        "userId": member.user_id,
        "displayName": member_display_name(member),
        "status": "active",
        "roles": member.roles,
        "positions": member.positions,
        "departments": member.departments,
        "notaryStaffRole": member_notary_staff_role(member).unwrap_or_else(|| "assistant".to_string())
    })
}

fn member_notary_staff_role(member: &AppbaseOrganizationMember) -> Option<String> {
    for role in ["notary", "assistant", "reviewer", "approver"] {
        if member.roles.iter().any(|candidate| candidate == role) {
            return Some(role.to_string());
        }
    }
    if member
        .positions
        .iter()
        .any(|position| position.eq_ignore_ascii_case("notary"))
    {
        return Some("notary".to_string());
    }
    None
}

fn case_summary_to_value(cases: &[NotaryCaseRecord]) -> Value {
    let total_count = cases.len();
    let pending_review_count = cases
        .iter()
        .filter(|record| record.status == NotaryCaseStatus::PendingReview)
        .count();
    let processing_count = cases
        .iter()
        .filter(|record| record.status == NotaryCaseStatus::Processing)
        .count();
    let completed_count = cases
        .iter()
        .filter(|record| record.status == NotaryCaseStatus::Completed)
        .count();
    let rejected_count = cases
        .iter()
        .filter(|record| {
            matches!(
                record.status,
                NotaryCaseStatus::Rejected
                    | NotaryCaseStatus::Cancelled
                    | NotaryCaseStatus::CreateFailed
            )
        })
        .count();
    let fee_amount_total = cases
        .iter()
        .filter_map(|record| record.fee_amount.parse::<f64>().ok())
        .sum::<f64>();

    json!({
        "totalCount": total_count,
        "pendingReviewCount": pending_review_count,
        "processingCount": processing_count,
        "completedCount": completed_count,
        "rejectedCount": rejected_count,
        "feeAmountTotal": format!("{fee_amount_total:.2}")
    })
}

fn notary_statistics_to_value(statistics: &NotaryDashboardStatisticsAggregate) -> Value {
    json!({
        "pendingReviewQueue": {
            "count": statistics.pending_review_count,
            "estimatedProcessHours": statistics.pending_review_count as f64
                * ESTIMATED_PROCESS_HOURS_PER_PENDING_CASE
        },
        "todayCompleted": {
            "count": statistics.today_completed_count,
            "comparedToYesterday": statistics.today_completed_count
                - statistics.yesterday_completed_count
        },
        "anomalyIntercepted": {
            "count": statistics.anomaly_intercepted_count,
            "interceptorType": "notary-risk-control"
        },
        "monthlyPreservationTotal": {
            "count": statistics.monthly_case_count,
            "blockchainSyncStatus": if statistics.unsynced_completed_count == 0 {
                "OK"
            } else {
                "PENDING"
            }
        },
        "timestamp": now_iso8601()
    })
}

async fn monthly_report_to_value(
    month: &str,
    case_count: i64,
    body: &Value,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<Value, NotaryServiceError> {
    let format = string_field(body, &["format"]).unwrap_or_else(|| "pdf".to_string());
    let report = ports
        .drive
        .create_monthly_report(DriveCreateMonthlyReportCommand {
            month: month.to_string(),
            format: format.clone(),
            case_count,
        })
        .await?;
    Ok(json!({
        "downloadUrl": report.download_url,
        "reportId": report.report_id,
        "month": report.month,
        "format": report.format,
        "generatedAt": now_iso8601(),
        "expiresAt": expires_after_minutes(7),
        "fileSize": report.file_size,
        "caseCount": report.case_count
    }))
}

async fn require_notary_member(
    appbase: &dyn crate::AppbasePort,
    organization_id: &str,
    membership_id: &str,
    require_admin: bool,
) -> Result<AppbaseOrganizationMember, NotaryServiceError> {
    let member = appbase
        .get_organization_member(organization_id, membership_id)
        .await?
        .ok_or_else(|| NotaryServiceError::unauthorized("organization member is required"))?;

    if !member.enterprise_verified || !member.notary_enabled {
        return Err(NotaryServiceError::unauthorized(
            "organization member is not enabled for notary business",
        ));
    }

    if require_admin
        && !member
            .roles
            .iter()
            .any(|role| role == "notary_admin" || role == "owner")
    {
        return Err(NotaryServiceError::unauthorized(
            "notary business opening requires a notary admin role",
        ));
    }

    let has_notary_staff_role = member.roles.iter().any(|role| {
        matches!(
            role.as_str(),
            "notary" | "notary_admin" | "assistant" | "reviewer" | "approver"
        )
    });
    let has_notary_position = member
        .positions
        .iter()
        .any(|position| position.contains("公证") || position.eq_ignore_ascii_case("notary"));

    if !require_admin && !has_notary_staff_role && !has_notary_position {
        return Err(NotaryServiceError::unauthorized(
            "case assignment requires notary staff role or position",
        ));
    }

    Ok(member)
}

fn validate_context(context: &NotaryRuntimeContext) -> Result<(), NotaryServiceError> {
    if is_blank(Some(context.tenant_id.as_str())) {
        return Err(NotaryServiceError::unauthenticated("tenant_id is required"));
    }
    if is_blank(Some(context.user_id.as_str())) {
        return Err(NotaryServiceError::unauthenticated("user_id is required"));
    }
    if is_blank(Some(context.session_id.as_str())) {
        return Err(NotaryServiceError::unauthenticated(
            "session_id is required",
        ));
    }
    if is_blank(Some(context.app_id.as_str())) {
        return Err(NotaryServiceError::unauthenticated("app_id is required"));
    }

    Ok(())
}

fn validate_create_case_command(command: &NotaryCaseCommand) -> Result<(), NotaryServiceError> {
    if is_blank(Some(command.organization_id.as_str())) {
        return Err(NotaryServiceError::validation(
            "organization_id is required",
        ));
    }
    if is_blank(Some(command.sku_id.as_str())) {
        return Err(NotaryServiceError::validation("sku_id is required"));
    }
    if is_blank(Some(command.title.as_str())) {
        return Err(NotaryServiceError::validation("title is required"));
    }
    if is_blank(Some(command.applicant_name.as_str())) {
        return Err(NotaryServiceError::validation("applicant_name is required"));
    }
    if is_blank(Some(command.idempotency_key.as_str())) {
        return Err(NotaryServiceError::validation(
            "idempotency_key is required",
        ));
    }

    Ok(())
}

fn build_case_no(order_item_id: &str) -> String {
    let suffix = order_item_id
        .chars()
        .filter(|value| value.is_ascii_alphanumeric())
        .rev()
        .take(6)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("NT-{}-{}", now_compact_date(), suffix.to_uppercase())
}

fn expires_after_minutes(minutes: i64) -> String {
    use sdkwork_utils_rust::{add_minutes, format_datetime, now};
    format_datetime(add_minutes(now(), minutes), Some("%Y-%m-%dT%H:%M:%SZ"))
}

async fn load_case_for_context(
    context: &NotaryRuntimeContext,
    ports: &NotaryRuntimePorts<'_>,
    case_id: &str,
) -> Result<NotaryCaseRecord, NotaryServiceError> {
    let record = ports
        .repository
        .get_case(case_id)
        .await?
        .ok_or_else(|| NotaryServiceError::not_found("notary case not found"))?;
    ensure_case_access(context, &record)?;
    Ok(record)
}

fn ensure_organization_scope(
    context: &NotaryRuntimeContext,
    organization_id: &str,
) -> Result<(), NotaryServiceError> {
    // Tenant-default policy: a personal (tenant-level) session has no organization
    // context and is treated as organization "0"; only an actively claimed organization
    // must match the requested scope.
    let context_organization_id = context.organization_id.as_deref().unwrap_or("0");
    if context_organization_id != organization_id {
        return Err(NotaryServiceError::unauthorized(
            "organization scope does not match request context",
        ));
    }
    Ok(())
}

fn ensure_case_access(
    context: &NotaryRuntimeContext,
    record: &NotaryCaseRecord,
) -> Result<(), NotaryServiceError> {
    ensure_organization_scope(context, &record.organization_id)
}

async fn ensure_case_staff_access(
    context: &NotaryRuntimeContext,
    record: &NotaryCaseRecord,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<(), NotaryServiceError> {
    ensure_case_access(context, record)?;
    let membership_id = context.membership_id.as_deref().ok_or_else(|| {
        NotaryServiceError::unauthorized(
            "organization membership is required for notary staff operations",
        )
    })?;
    require_notary_member(ports.appbase, &record.organization_id, membership_id, false).await?;
    Ok(())
}

async fn load_case_for_staff_mutation(
    context: &NotaryRuntimeContext,
    ports: &NotaryRuntimePorts<'_>,
    case_id: &str,
) -> Result<NotaryCaseRecord, NotaryServiceError> {
    let record = load_case_for_context(context, ports, case_id).await?;
    ensure_case_staff_access(context, &record, ports).await?;
    Ok(record)
}

async fn load_mutable_case_for_staff(
    context: &NotaryRuntimeContext,
    ports: &NotaryRuntimePorts<'_>,
    case_id: &str,
) -> Result<NotaryCaseRecord, NotaryServiceError> {
    let record = load_case_for_staff_mutation(context, ports, case_id).await?;
    if record.status.is_terminal() {
        return Err(NotaryServiceError::invalid_state(format!(
            "notary case {} is terminal and cannot be modified",
            record.case_no
        )));
    }
    Ok(record)
}

async fn require_notary_admin_for_org(
    context: &NotaryRuntimeContext,
    organization_id: &str,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<(), NotaryServiceError> {
    ensure_organization_scope(context, organization_id)?;
    let membership_id = context.membership_id.as_deref().ok_or_else(|| {
        NotaryServiceError::unauthorized(
            "organization membership is required for notary admin operations",
        )
    })?;
    require_notary_member(ports.appbase, organization_id, membership_id, true).await?;
    Ok(())
}

async fn ensure_report_staff_access(
    context: &NotaryRuntimeContext,
    body: &Value,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<(), NotaryServiceError> {
    let organization_id = string_field(body, &["organizationId", "organization_id"])
        .or_else(|| context.organization_id.clone())
        .unwrap_or_else(|| "0".to_string());
    let membership_id = context.membership_id.as_deref().ok_or_else(|| {
        NotaryServiceError::unauthorized(
            "organization membership is required for notary report operations",
        )
    })?;
    require_notary_member(ports.appbase, &organization_id, membership_id, false).await?;
    ensure_organization_scope(context, &organization_id)?;
    Ok(())
}

fn report_organization_id(
    context: &NotaryRuntimeContext,
    body: &Value,
) -> Result<String, NotaryServiceError> {
    let organization_id = string_field(body, &["organizationId", "organization_id"])
        .or_else(|| context.organization_id.clone())
        .unwrap_or_else(|| "0".to_string());
    ensure_organization_scope(context, &organization_id)?;
    Ok(organization_id)
}

fn page_size_field(value: &Value, names: &[&str], default: i64) -> Result<i64, NotaryServiceError> {
    let page_size = names
        .iter()
        .find_map(|name| value.get(*name))
        .map(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str()?.parse::<i64>().ok())
                .ok_or_else(|| NotaryServiceError::validation("page_size must be an integer"))
        })
        .transpose()?
        .unwrap_or(default);
    validated_list_page_size(page_size)
}

fn member_display_name(member: &AppbaseOrganizationMember) -> String {
    if is_blank(Some(member.display_name.as_str())) {
        member.membership_id.clone()
    } else {
        member.display_name.trim().to_string()
    }
}

async fn update_case_from_body(
    context: &NotaryRuntimeContext,
    case_id: &str,
    body: &Value,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<NotaryCaseRecord, NotaryServiceError> {
    let current = load_mutable_case_for_staff(context, ports, case_id).await?;
    let status = string_field(body, &["status"]).map(|value| case_status_from_api(&value));
    let status = status.transpose()?;
    if let Some(next_status) = &status {
        if !current.status.allows_transition_to(next_status) {
            return Err(NotaryServiceError::invalid_state(format!(
                "cannot transition notary case from {} to {}",
                current.status.as_storage_value(),
                next_status.as_storage_value()
            )));
        }
    }
    let record = ports
        .repository
        .update_case(NotaryCaseUpdateCommand {
            case_id: case_id.to_string(),
            expected_version: version_field(body)?.unwrap_or(current.version),
            title: string_field(body, &["title"]),
            remarks: string_field(body, &["remarks", "description"]),
            status,
            chain_hash: string_field(body, &["chainHash", "chain_hash"]),
            reject_reason: None,
            event_type: "notary.case.updated".to_string(),
        })
        .await?;
    Ok(record)
}

fn create_case_command_from_body(
    context: &NotaryRuntimeContext,
    body: &Value,
    idempotency_key: Option<&str>,
) -> Result<NotaryCaseCommand, NotaryServiceError> {
    let organization_id = string_field(body, &["organizationId", "organization_id"])
        .or_else(|| context.organization_id.clone())
        .unwrap_or_else(|| "0".to_string());
    let sku_id = string_field(body, &["skuId", "sku_id"])
        .ok_or_else(|| NotaryServiceError::validation("skuId is required"))?;
    let title = string_field(body, &["title"])
        .ok_or_else(|| NotaryServiceError::validation("title is required"))?;
    let applicant_name = string_field(body, &["applicantName", "applicant_name", "applicant"])
        .ok_or_else(|| NotaryServiceError::validation("applicantName is required"))?;
    let idempotency_key = idempotency_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| NotaryServiceError::validation("Idempotency-Key header is required"))?
        .to_owned();

    Ok(NotaryCaseCommand {
        organization_id,
        sku_id,
        title,
        drive_folder_name: string_field(body, &["driveFolderName", "drive_folder_name"]),
        applicant_name,
        remarks: string_field(body, &["remarks", "description"]),
        primary_notary_membership_id: string_field(
            body,
            &["primaryNotaryMembershipId", "primary_notary_membership_id"],
        ),
        idempotency_key,
        parties: party_commands_from_body(body)?,
    })
}

fn party_commands_from_body(
    body: &Value,
) -> Result<Vec<sdkwork_notary_case_contract::NotaryPartyCommand>, NotaryServiceError> {
    let Some(parties) = body.get("parties") else {
        return Ok(Vec::new());
    };
    let parties = parties
        .as_array()
        .ok_or_else(|| NotaryServiceError::validation("parties must be an array"))?;

    parties.iter().map(party_command_from_value).collect()
}

fn party_command_from_value(
    party: &Value,
) -> Result<sdkwork_notary_case_contract::NotaryPartyCommand, NotaryServiceError> {
    let name = string_field(party, &["name"])
        .ok_or_else(|| NotaryServiceError::validation("party.name is required"))?;
    let party_role = string_field(party, &["role", "partyRole", "party_role"])
        .ok_or_else(|| NotaryServiceError::validation("party.role is required"))?;
    let identity_no = string_field(party, &["identityNo", "identity_no", "identityId"])
        .ok_or_else(|| NotaryServiceError::validation("party.identityNo is required"))?;
    Ok(sdkwork_notary_case_contract::NotaryPartyCommand {
        name,
        party_role,
        identity_no,
        phone: string_field(party, &["phone"]),
    })
}

fn party_update_command_from_body(
    case_id: &str,
    party_id: &str,
    body: &Value,
) -> NotaryPartyUpdateCommand {
    NotaryPartyUpdateCommand {
        case_id: case_id.to_string(),
        party_id: party_id.to_string(),
        name: string_field(body, &["name"]),
        party_role: string_field(body, &["role", "partyRole", "party_role"]),
        identity_no: string_field(body, &["identityNo", "identity_no", "identityId"]),
        phone: string_field(body, &["phone"]),
        signature_node_id: string_field(body, &["signatureNodeId", "signature_node_id"]),
    }
}

fn matter_command_from_body(
    context: &NotaryRuntimeContext,
    body: &Value,
) -> Result<CommerceMatterCommand, NotaryServiceError> {
    let title = string_field(body, &["title"])
        .ok_or_else(|| NotaryServiceError::validation("title is required"))?;
    let price_amount = string_field(body, &["priceAmount", "price_amount"])
        .ok_or_else(|| NotaryServiceError::validation("priceAmount is required"))?;
    let currency_code = string_field(body, &["currencyCode", "currency_code"])
        .ok_or_else(|| NotaryServiceError::validation("currencyCode is required"))?;
    let status = string_field(body, &["status"]).unwrap_or_else(|| "active".to_string());
    validate_matter_status(&status)?;

    Ok(CommerceMatterCommand {
        organization_id: string_field(body, &["organizationId", "organization_id"])
            .or_else(|| context.organization_id.clone()),
        title,
        description: string_field(body, &["description"]),
        price_amount,
        original_price_amount: string_field(
            body,
            &["originalPriceAmount", "original_price_amount"],
        ),
        currency_code,
        status,
        spec: body.get("spec").cloned().unwrap_or_else(|| json!({})),
        idempotency_key: string_field(body, &["idempotencyKey", "idempotency_key"])
            .unwrap_or_else(|| format!("notary-matter:{}:{}", context.user_id, uuid_seed(body))),
    })
}

fn matter_update_command_from_body(
    organization_id: &str,
    sku_id: &str,
    body: &Value,
) -> Result<CommerceMatterUpdateCommand, NotaryServiceError> {
    let title = optional_non_empty_string_patch(body, &["title"], "title")?;
    let description = nullable_string_patch(body, &["description"], "description")?;
    let price_amount =
        optional_non_empty_string_patch(body, &["priceAmount", "price_amount"], "priceAmount")?;
    let original_price_amount = nullable_string_patch(
        body,
        &["originalPriceAmount", "original_price_amount"],
        "originalPriceAmount",
    )?;
    let currency_code =
        optional_non_empty_string_patch(body, &["currencyCode", "currency_code"], "currencyCode")?;
    let status = optional_non_empty_string_patch(body, &["status"], "status")?;
    if let Some(status) = status.as_deref() {
        validate_matter_status(status)?;
    }

    let spec = body.get("spec").cloned();
    if title.is_none()
        && description.is_none()
        && price_amount.is_none()
        && original_price_amount.is_none()
        && currency_code.is_none()
        && status.is_none()
        && spec.is_none()
    {
        return Err(NotaryServiceError::validation(
            "notary matter update requires at least one field",
        ));
    }

    let sets_original_price = matches!(original_price_amount, Some(Some(_)));
    if (price_amount.is_some() || sets_original_price) && currency_code.is_none() {
        return Err(NotaryServiceError::validation(
            "currencyCode is required when updating a monetary amount",
        ));
    }
    if currency_code.is_some() && price_amount.is_none() {
        return Err(NotaryServiceError::validation(
            "priceAmount is required when changing currencyCode",
        ));
    }

    Ok(CommerceMatterUpdateCommand {
        organization_id: organization_id.to_string(),
        sku_id: sku_id.to_string(),
        title,
        description,
        price_amount,
        original_price_amount,
        currency_code,
        status,
        spec,
    })
}

fn order_status_is_terminal(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "fulfilled"
            | "completed"
            | "finished"
            | "cancelled"
            | "canceled"
            | "closed"
            | "expired"
            | "refunded"
    )
}

fn validate_profile_status(status: &str) -> Result<(), NotaryServiceError> {
    if matches!(status, "active" | "suspended" | "closed") {
        Ok(())
    } else {
        Err(NotaryServiceError::validation(format!(
            "unsupported notary organization profile status: {status}"
        )))
    }
}

fn validate_matter_status(status: &str) -> Result<(), NotaryServiceError> {
    if matches!(status, "draft" | "active" | "inactive") {
        Ok(())
    } else {
        Err(NotaryServiceError::validation(format!(
            "unsupported notary matter status: {status}"
        )))
    }
}

fn validate_assignment_role(role: &str) -> Result<(), NotaryServiceError> {
    if matches!(
        role,
        "primary_notary" | "assistant" | "reviewer" | "approver"
    ) {
        Ok(())
    } else {
        Err(NotaryServiceError::validation(format!(
            "unsupported notary assignment role: {role}"
        )))
    }
}

fn validate_video_invite_purpose(purpose: &str) -> Result<(), NotaryServiceError> {
    if matches!(
        purpose,
        "identity_verification" | "material_confirmation" | "remote_inquiry"
    ) {
        Ok(())
    } else {
        Err(NotaryServiceError::validation(format!(
            "unsupported notary video invite purpose: {purpose}"
        )))
    }
}

fn validate_signature_invite_purpose(purpose: &str) -> Result<(), NotaryServiceError> {
    if matches!(
        purpose,
        "remote_signature" | "onsite_signature_confirmation" | "material_signature"
    ) {
        Ok(())
    } else {
        Err(NotaryServiceError::validation(format!(
            "unsupported notary signature invite purpose: {purpose}"
        )))
    }
}

fn uuid_seed(value: &Value) -> String {
    let seed = value.to_string();
    let mut hash = 0xcbf29ce484222325u64;
    for byte in seed.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn case_record_to_value(record: &NotaryCaseRecord) -> Value {
    json!({
        "id": record.case_id,
        "caseId": record.case_id,
        "caseNo": record.case_no,
        "createTime": record.created_at,
        "createdAt": record.created_at,
        "processTime": record.updated_at,
        "updatedAt": record.updated_at,
        "applicant": record.applicant_name,
        "applicantName": record.applicant_name,
        "title": record.title,
        "notary": record.primary_notary_name.clone().unwrap_or_else(|| "Unassigned".to_string()),
        "primaryNotaryMembershipId": record.primary_notary_membership_id,
        "primaryNotaryName": record.primary_notary_name,
        "remarks": record.remarks.clone().unwrap_or_default(),
        "type": record.matter_title,
        "matterTitle": record.matter_title,
        "status": record.status.as_frontend_value(),
        "fee": record.fee_amount,
        "feeAmount": record.fee_amount,
        "currencyCode": record.currency_code,
        "hash": record.chain_hash.clone().unwrap_or_default(),
        "chainHash": record.chain_hash,
        "orderId": record.order_id,
        "orderItemId": record.order_item_id,
        "skuId": record.sku_id,
        "driveSpaceId": record.drive_space_id,
        "driveSpaceType": record.drive_space_type,
        "driveFolderNodeId": record.drive_folder_node_id,
        "version": record.version.to_string(),
        "parties": [],
        "documents": [],
        "timeline": []
    })
}

async fn list_all_parties(
    case_id: &str,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<Vec<NotaryPartyRecord>, NotaryServiceError> {
    let mut items = Vec::new();
    let mut cursor = None;
    loop {
        let page = ports
            .repository
            .list_parties(NotaryPartyListQuery {
                case_id: case_id.to_string(),
                page_size: NOTARY_MAX_LIST_PAGE_SIZE,
                cursor: cursor.clone(),
            })
            .await?;
        if page.items.is_empty() && !page.has_more {
            break;
        }
        items.extend(page.items);
        if !page.has_more {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(items)
}

async fn case_detail_to_value(
    record: &NotaryCaseRecord,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<Value, NotaryServiceError> {
    let parties = list_all_parties(&record.case_id, ports).await?;
    let events = list_all_case_events(&record.case_id, ports).await?;
    let files = list_all_case_drive_files(record, ports).await?;
    let mut value = case_record_to_value(record);
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "parties".to_string(),
            Value::Array(parties.iter().map(party_record_to_value).collect()),
        );
        object.insert(
            "documents".to_string(),
            Value::Array(
                files
                    .iter()
                    .map(|file| drive_node_to_document_value(file, record))
                    .collect(),
            ),
        );
        object.insert(
            "timeline".to_string(),
            Value::Array(events.iter().map(event_record_to_value).collect()),
        );
    }
    Ok(value)
}

async fn list_all_case_drive_files(
    record: &NotaryCaseRecord,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<Vec<DriveNodeReference>, NotaryServiceError> {
    let mut items = Vec::new();
    let mut cursor = None;
    loop {
        let page = ports
            .drive
            .list_nodes(DriveListNodesQuery {
                space_id: record.drive_space_id.clone(),
                space_type: record.drive_space_type.clone(),
                parent_node_id: record.drive_folder_node_id.clone(),
                category: None,
                page_size: NOTARY_MAX_LIST_PAGE_SIZE,
                cursor: cursor.clone(),
            })
            .await?;
        if page.items.is_empty() && !page.has_more {
            break;
        }
        items.extend(page.items);
        if !page.has_more {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(items)
}

async fn list_all_case_events(
    case_id: &str,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<Vec<NotaryCaseEventRecord>, NotaryServiceError> {
    let mut items = Vec::new();
    let mut cursor = None;
    loop {
        let page = ports
            .repository
            .list_events(NotaryCaseEventListQuery {
                case_id: case_id.to_string(),
                page_size: NOTARY_MAX_LIST_PAGE_SIZE,
                cursor: cursor.clone(),
            })
            .await?;
        if page.items.is_empty() && !page.has_more {
            break;
        }
        items.extend(page.items);
        if !page.has_more {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(items)
}

fn party_record_to_value(party: &NotaryPartyRecord) -> Value {
    let signature_url = party
        .signature_node_id
        .as_ref()
        .map(|node_id| format!("drive://notary/signatures/{node_id}"));
    json!({
        "id": party.party_id,
        "partyId": party.party_id,
        "caseId": party.case_id,
        "orderId": party.order_id,
        "orderItemId": party.order_item_id,
        "skuId": party.sku_id,
        "name": party.name,
        "role": party.party_role,
        "partyRole": party.party_role,
        "identityId": format!("****{}", party.identity_no_last4),
        "identityNoLast4": party.identity_no_last4,
        "phone": party.phone_masked,
        "phoneMasked": party.phone_masked,
        "verificationStatus": "pending",
        "status": party.status,
        "signatureNodeId": party.signature_node_id,
        "signatureUrl": signature_url
    })
}

fn event_record_to_value(event: &NotaryCaseEventRecord) -> Value {
    json!({
        "id": event.event_id,
        "eventId": event.event_id,
        "caseId": event.case_id,
        "time": event.occurred_at,
        "occurredAt": event.occurred_at,
        "event": event.event_title,
        "eventTitle": event.event_title,
        "eventType": event.event_type,
        "actor": event.actor_user_id.clone().unwrap_or_else(|| "System".to_string()),
        "actorUserId": event.actor_user_id
    })
}

fn download_package_to_value(package: &DriveDownloadPackageReference) -> Value {
    let mut value = json!({
        "packageId": package.package_id,
        "caseId": package.case_id,
        "driveSpaceId": package.drive_space_id,
        "driveSpaceType": package.drive_space_type,
        "status": package.status,
        "packageName": package.package_name
    });
    if let Some(download_url) = &package.download_url {
        value["downloadUrl"] = json!(download_url);
    }
    value
}

fn page_info_json(page_info: PageInfo) -> Value {
    serde_json::to_value(page_info).unwrap_or_else(|_| {
        json!({
            "mode": "cursor",
            "hasMore": false
        })
    })
}

fn cursor_page_info(page_size: i64, has_more: bool, next_cursor: Option<&str>) -> Value {
    page_info_json(PageInfo {
        mode: PageMode::Cursor,
        page: None,
        page_size: i32::try_from(page_size).ok(),
        total_items: None,
        total_pages: None,
        next_cursor: next_cursor.map(str::to_string),
        has_more: Some(has_more),
    })
}

fn cursor_page_info_with_total(
    page_size: i64,
    has_more: bool,
    next_cursor: Option<&str>,
    total_items: i64,
) -> Value {
    page_info_json(PageInfo {
        mode: PageMode::Cursor,
        page: None,
        page_size: i32::try_from(page_size).ok(),
        total_items: Some(total_items.to_string()),
        total_pages: None,
        next_cursor: next_cursor.map(str::to_string),
        has_more: Some(has_more),
    })
}

fn drive_list_page_info(page: &DriveListNodesPage, page_size: i64) -> Value {
    cursor_page_info(page_size, page.has_more, page.next_cursor.as_deref())
}

fn drive_node_to_document_value(node: &DriveNodeReference, record: &NotaryCaseRecord) -> Value {
    let mut document = json!({
        "nodeId": node.node_id,
        "driveNodeId": node.node_id,
        "driveSpaceId": record.drive_space_id,
        "driveSpaceType": record.drive_space_type,
        "parentNodeId": record.drive_folder_node_id,
        "name": node.node_name,
        "nodeName": node.node_name,
        "size": node.size_label,
        "sizeLabel": node.size_label,
        "status": node.status,
        "reviewStatus": node.status,
        "category": node.category
    });
    if let Some(material_code) = node.material_code.as_deref() {
        document["materialCode"] = json!(material_code);
    }
    if let Some(party_id) = node.party_id.as_deref() {
        document["partyId"] = json!(party_id);
    }
    document
}

fn file_create_to_document_value(
    body: &Value,
    record: &NotaryCaseRecord,
) -> Result<Value, NotaryServiceError> {
    let node_id = string_field(body, &["driveNodeId", "drive_node_id", "nodeId"])
        .ok_or_else(|| NotaryServiceError::validation("driveNodeId is required"))?;
    let category = string_field(body, &["category"]).unwrap_or_else(|| "evidence".to_string());
    let status = string_field(body, &["reviewStatus", "review_status", "status"])
        .unwrap_or_else(|| "pending".to_string());
    let name = string_field(body, &["materialCode", "material_code", "name"])
        .unwrap_or_else(|| node_id.clone());
    let mut document = json!({
        "nodeId": node_id,
        "driveNodeId": node_id,
        "driveSpaceId": record.drive_space_id,
        "driveSpaceType": record.drive_space_type,
        "parentNodeId": record.drive_folder_node_id,
        "name": name,
        "nodeName": name,
        "size": "",
        "sizeLabel": "",
        "status": status,
        "reviewStatus": status,
        "category": category
    });
    if let Some(material_code) = string_field(body, &["materialCode", "material_code"]) {
        document["materialCode"] = json!(material_code);
    }
    if let Some(party_id) = string_field(body, &["partyId", "party_id"]) {
        document["partyId"] = json!(party_id);
    }
    Ok(document)
}

async fn create_party_video_invite(
    context: &NotaryRuntimeContext,
    case_id: &str,
    party_id: &str,
    body: &Value,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<Value, NotaryServiceError> {
    let record = load_mutable_case_for_staff(context, ports, case_id).await?;
    let parties = list_all_parties(case_id, ports).await?;
    let party = parties
        .iter()
        .find(|party| party.party_id == party_id && party.status != "removed")
        .ok_or_else(|| NotaryServiceError::not_found("notary party not found"))?;
    let purpose =
        string_field(body, &["purpose"]).unwrap_or_else(|| "identity_verification".to_string());
    validate_video_invite_purpose(&purpose)?;

    let invite = ports
        .drive
        .create_party_video_invite(DriveCreatePartyVideoInviteCommand {
            case_id: record.case_id.clone(),
            party_id: party.party_id.clone(),
            party_name: party.name.clone(),
            purpose,
            drive_space_id: record.drive_space_id.clone(),
            drive_space_type: record.drive_space_type.clone(),
            drive_folder_node_id: record.drive_folder_node_id.clone(),
        })
        .await?;

    ports
        .repository
        .append_event(&record.case_id, "notary.party.video_invite.created")
        .await?;

    Ok(json!({
        "inviteId": invite.invite_id,
        "caseId": invite.case_id,
        "partyId": invite.party_id,
        "partyName": invite.party_name,
        "purpose": invite.purpose,
        "conversationId": invite.conversation_id,
        "inviteUrl": invite.invite_url,
        "expiresAt": expires_after_minutes(10),
        "driveSpaceId": invite.drive_space_id,
        "driveSpaceType": invite.drive_space_type,
        "driveFolderNodeId": invite.drive_folder_node_id
    }))
}

async fn create_party_signature_invite(
    context: &NotaryRuntimeContext,
    case_id: &str,
    party_id: &str,
    body: &Value,
    ports: &NotaryRuntimePorts<'_>,
) -> Result<Value, NotaryServiceError> {
    let record = load_mutable_case_for_staff(context, ports, case_id).await?;
    let parties = list_all_parties(case_id, ports).await?;
    let party = parties
        .iter()
        .find(|party| party.party_id == party_id && party.status != "removed")
        .ok_or_else(|| NotaryServiceError::not_found("notary party not found"))?;
    let purpose =
        string_field(body, &["purpose"]).unwrap_or_else(|| "remote_signature".to_string());
    validate_signature_invite_purpose(&purpose)?;

    let invite = ports
        .drive
        .create_party_signature_invite(DriveCreatePartySignatureInviteCommand {
            case_id: record.case_id.clone(),
            party_id: party.party_id.clone(),
            party_name: party.name.clone(),
            purpose,
            drive_space_id: record.drive_space_id.clone(),
            drive_space_type: record.drive_space_type.clone(),
            drive_folder_node_id: record.drive_folder_node_id.clone(),
        })
        .await?;

    ports
        .repository
        .append_event(&record.case_id, "notary.party.signature_invite.created")
        .await?;

    Ok(json!({
        "inviteId": invite.invite_id,
        "caseId": invite.case_id,
        "partyId": invite.party_id,
        "partyName": invite.party_name,
        "purpose": invite.purpose,
        "inviteUrl": invite.invite_url,
        "signingUrl": invite.invite_url,
        "expiresAt": expires_after_minutes(10),
        "driveSpaceId": invite.drive_space_id,
        "driveSpaceType": invite.drive_space_type,
        "driveFolderNodeId": invite.drive_folder_node_id
    }))
}

fn path_param<'a>(
    path_params: &'a BTreeMap<String, String>,
    name: &str,
) -> Result<&'a str, NotaryServiceError> {
    path_params
        .get(name)
        .or_else(|| path_params.get(&to_snake_case(name)))
        .map(String::as_str)
        .ok_or_else(|| NotaryServiceError::validation(format!("{name} path parameter is required")))
}

fn string_field(value: &Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        value
            .get(*name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn optional_non_empty_string_patch(
    value: &Value,
    names: &[&str],
    field_name: &str,
) -> Result<Option<String>, NotaryServiceError> {
    for name in names {
        let Some(raw) = value.get(*name) else {
            continue;
        };
        let text = raw.as_str().ok_or_else(|| {
            NotaryServiceError::validation(format!("{field_name} must be a string"))
        })?;
        let text = text.trim();
        if text.is_empty() {
            return Err(NotaryServiceError::validation(format!(
                "{field_name} must not be blank"
            )));
        }
        return Ok(Some(text.to_owned()));
    }
    Ok(None)
}

fn nullable_string_patch(
    value: &Value,
    names: &[&str],
    field_name: &str,
) -> Result<Option<Option<String>>, NotaryServiceError> {
    for name in names {
        let Some(raw) = value.get(*name) else {
            continue;
        };
        if raw.is_null() {
            return Ok(Some(None));
        }
        let text = raw.as_str().ok_or_else(|| {
            NotaryServiceError::validation(format!("{field_name} must be a string or null"))
        })?;
        let text = text.trim();
        if text.is_empty() {
            return Err(NotaryServiceError::validation(format!(
                "{field_name} must use null to clear the value"
            )));
        }
        return Ok(Some(Some(text.to_owned())));
    }
    Ok(None)
}

fn version_field(value: &Value) -> Result<Option<i64>, NotaryServiceError> {
    let Some(raw) = value.get("version") else {
        return Ok(None);
    };
    let version = raw
        .as_str()
        .ok_or_else(|| NotaryServiceError::validation("version must be an int64 string"))?
        .parse::<i64>()
        .map_err(|_| NotaryServiceError::validation("version must be an int64 string"))?;
    if version <= 0 {
        return Err(NotaryServiceError::validation(
            "version must be greater than zero",
        ));
    }
    Ok(Some(version))
}

fn status_to_storage(value: &str) -> String {
    match value {
        "PENDING_REVIEW" => "pending_review".to_string(),
        "PROCESSING" => "processing".to_string(),
        "COMPLETED" => "completed".to_string(),
        "REJECTED" => "rejected".to_string(),
        "CANCELLED" => "cancelled".to_string(),
        "CREATE_FAILED" => "create_failed".to_string(),
        _ => value.to_string(),
    }
}

fn case_status_from_api(value: &str) -> Result<NotaryCaseStatus, NotaryServiceError> {
    let storage_value = status_to_storage(value);
    NotaryCaseStatus::from_storage_value(&storage_value).ok_or_else(|| {
        NotaryServiceError::validation(format!("unsupported notary case status: {value}"))
    })
}

fn to_snake_case(value: &str) -> String {
    let mut result = String::new();
    for (index, character) in value.chars().enumerate() {
        if character.is_ascii_uppercase() {
            if index > 0 {
                result.push('_');
            }
            result.push(character.to_ascii_lowercase());
        } else {
            result.push(character);
        }
    }
    result
}
