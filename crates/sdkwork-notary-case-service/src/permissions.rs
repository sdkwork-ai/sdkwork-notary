use sdkwork_notary_case_contract::{NotaryRuntimeContext, NotaryServiceError};

static OPERATION_PERMISSIONS: &[(&str, &str)] = &[
    ("notary.cases.create", "notary.cases.create"),
    ("notary.cases.update", "notary.cases.update"),
    ("notary.cases.acceptances.create", "notary.cases.accept"),
    ("notary.cases.rejections.create", "notary.cases.reject"),
    ("notary.cases.completions.create", "notary.cases.complete"),
    ("notary.cases.parties.create", "notary.parties.create"),
    ("notary.cases.parties.update", "notary.parties.update"),
    ("notary.cases.parties.delete", "notary.parties.delete"),
    (
        "notary.cases.parties.signatures.create",
        "notary.parties.signatures.create",
    ),
    (
        "notary.cases.parties.videoInvites.create",
        "notary.parties.video_invites.create",
    ),
    (
        "notary.cases.parties.signatureInvites.create",
        "notary.parties.signature_invites.create",
    ),
    ("notary.cases.files.create", "notary.files.create"),
    (
        "notary.cases.downloadPackages.create",
        "notary.files.download",
    ),
    (
        "notary.cases.assignments.create",
        "notary.cases.assignments.create",
    ),
    (
        "notary.organizationProfiles.list",
        "notary.organization_profiles.read",
    ),
    (
        "notary.organizationProfiles.create",
        "notary.organization_profiles.create",
    ),
    (
        "notary.organizationProfiles.retrieve",
        "notary.organization_profiles.read",
    ),
    (
        "notary.organizationProfiles.update",
        "notary.organization_profiles.update",
    ),
    (
        "notary.matters.management.list",
        "notary.matters.management.read",
    ),
    ("notary.matters.create", "notary.matters.create"),
    ("notary.matters.update", "notary.matters.update"),
    (
        "notary.cases.management.list",
        "notary.cases.management.read",
    ),
    (
        "notary.cases.management.retrieve",
        "notary.cases.management.read",
    ),
    (
        "notary.cases.assignments.delete",
        "notary.cases.assignments.delete",
    ),
    (
        "notary.reports.caseSummary.retrieve",
        "notary.reports.case_summary.read",
    ),
];

pub fn permission_for_operation(operation_id: &str) -> Option<&'static str> {
    OPERATION_PERMISSIONS
        .iter()
        .find_map(|(operation, permission)| (*operation == operation_id).then_some(*permission))
}

pub fn require_operation_permission(
    context: &NotaryRuntimeContext,
    operation_id: &str,
) -> Result<(), NotaryServiceError> {
    let Some(required) = permission_for_operation(operation_id) else {
        return Ok(());
    };
    require_permission(context, required)
}

pub fn require_permission(
    context: &NotaryRuntimeContext,
    required: &str,
) -> Result<(), NotaryServiceError> {
    if context.permission_scopes.is_empty() && allows_dev_permission_bypass() {
        return Ok(());
    }
    if context
        .permission_scopes
        .iter()
        .any(|granted| permission_matches(granted, required))
    {
        return Ok(());
    }
    Err(NotaryServiceError::unauthorized(format!(
        "missing permission: {required}"
    )))
}

pub fn granted_notary_permissions(context: &NotaryRuntimeContext) -> Vec<String> {
    context
        .permission_scopes
        .iter()
        .filter(|scope| scope.starts_with("notary."))
        .cloned()
        .collect()
}

pub fn expand_notary_access_permissions(granted: &[String]) -> Vec<String> {
    if granted.is_empty() || granted.iter().any(|scope| scope == "notary.*") {
        return default_notary_staff_access_permissions();
    }
    granted.to_vec()
}

pub fn default_notary_staff_access_permissions() -> Vec<String> {
    vec![
        "notary.access.read".to_string(),
        "notary.matters.read".to_string(),
        "notary.cases.read".to_string(),
        "notary.cases.create".to_string(),
        "notary.cases.update".to_string(),
        "notary.files.read".to_string(),
        "notary.files.create".to_string(),
    ]
}

fn permission_matches(granted: &str, required: &str) -> bool {
    if granted == required || granted == "notary.*" {
        return true;
    }
    if let Some(prefix) = granted.strip_suffix(".*") {
        return required.starts_with(&format!("{prefix}."));
    }
    false
}

fn allows_dev_permission_bypass() -> bool {
    notary_runtime_environment_allows_dev_fallback()
}

fn notary_runtime_environment_allows_dev_fallback() -> bool {
    matches!(
        std::env::var("SDKWORK_NOTARY_ENVIRONMENT")
            .or_else(|_| std::env::var("SDKWORK_ENVIRONMENT"))
            .unwrap_or_else(|_| "production".to_owned())
            .to_ascii_lowercase()
            .as_str(),
        "development" | "dev" | "test" | "local"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_permission_matches_nested_codes() {
        assert!(permission_matches("notary.*", "notary.cases.read"));
        assert!(!permission_matches(
            "notary.cases.read",
            "notary.files.read"
        ));
    }
}
