-- sdkwork:migration
-- id: 0001_organization_id_not_null
-- engine: postgres
-- module: sdkwork-notary
-- purpose: Enforce organization_id NOT NULL DEFAULT on all tables in the
--   consolidated baseline. NULL rows (pre-standard data anomalies) are
--   backfilled with the platform sentinel before NOT NULL is set, and
--   NOT NULL columns without an explicit default receive the sentinel
--   default, keeping existing deployments consistent with fresh baseline
--   installs.
-- reversible: false
-- rollback: forward-fix (sentinel backfill is the canonical fix; NULL
--   organization rows are data anomalies)
-- transactional: true
-- lock: lightweight
-- lock_timeout: 2s
-- statement_timeout: 30s

BEGIN;

ALTER TABLE notary_organization_profile ADD COLUMN IF NOT EXISTS organization_id TEXT NOT NULL DEFAULT '0';
UPDATE notary_organization_profile SET organization_id = '0' WHERE organization_id IS NULL;
ALTER TABLE notary_organization_profile ALTER COLUMN organization_id SET DEFAULT '0';
ALTER TABLE notary_organization_profile ALTER COLUMN organization_id SET NOT NULL;

ALTER TABLE notary_case ADD COLUMN IF NOT EXISTS organization_id TEXT NOT NULL DEFAULT '0';
UPDATE notary_case SET organization_id = '0' WHERE organization_id IS NULL;
ALTER TABLE notary_case ALTER COLUMN organization_id SET DEFAULT '0';
ALTER TABLE notary_case ALTER COLUMN organization_id SET NOT NULL;

ALTER TABLE notary_party ADD COLUMN IF NOT EXISTS organization_id TEXT NOT NULL DEFAULT '0';
UPDATE notary_party SET organization_id = '0' WHERE organization_id IS NULL;
ALTER TABLE notary_party ALTER COLUMN organization_id SET DEFAULT '0';
ALTER TABLE notary_party ALTER COLUMN organization_id SET NOT NULL;

ALTER TABLE notary_case_assignment ADD COLUMN IF NOT EXISTS organization_id TEXT NOT NULL DEFAULT '0';
UPDATE notary_case_assignment SET organization_id = '0' WHERE organization_id IS NULL;
ALTER TABLE notary_case_assignment ALTER COLUMN organization_id SET DEFAULT '0';
ALTER TABLE notary_case_assignment ALTER COLUMN organization_id SET NOT NULL;

ALTER TABLE notary_case_event ADD COLUMN IF NOT EXISTS organization_id TEXT NOT NULL DEFAULT '0';
UPDATE notary_case_event SET organization_id = '0' WHERE organization_id IS NULL;
ALTER TABLE notary_case_event ALTER COLUMN organization_id SET DEFAULT '0';
ALTER TABLE notary_case_event ALTER COLUMN organization_id SET NOT NULL;

COMMIT;
