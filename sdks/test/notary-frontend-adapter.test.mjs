// WORKSPACE-PATH:allow-fixture: fixtures name a foreign checkout root, drive, or home directory to exercise path handling, so the literal is the value under assertion rather than a binding this build resolves
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { imPcRoot, imPcTest, notaryPcRoot, workspaceRoot } from "./helpers/im-pc-root.mjs";

const notaryServicePath =
  "packages/sdkwork-notary-pc-notary/src/services/NotaryService.ts";

function readNotaryPcText(relativePath) {
  return readFileSync(path.join(notaryPcRoot, relativePath), "utf8");
}

function readNotaryServiceSource() {
  return readNotaryPcText(notaryServicePath);
}

function methodBody(source, methodName) {
  const match = new RegExp(`async\\s+${methodName}\\s*\\(`).exec(source);
  assert(match, `${methodName} must exist`);
  const start = match.index;
  const next = source.indexOf("\n    async ", start + 1);
  return next >= 0 ? source.slice(start, next) : source.slice(start);
}

function functionBody(source, functionName) {
  const match = new RegExp(`function\\s+${functionName}\\s*\\(`).exec(source);
  assert(match, `${functionName} must exist`);
  const start = match.index;
  const next = source.indexOf("\nfunction ", start + 1);
  return next >= 0 ? source.slice(start, next) : source.slice(start);
}

imPcTest("real IM PC notary service preserves the existing UI service shape", () => {
  const source = readNotaryServiceSource();
  for (const method of [
    "getTasks",
    "getTaskById",
    "getStaff",
    "createTask",
    "assignNotary",
    "updateTaskStatus",
    "updateTask",
    "addParty",
    "addDocument",
    "listPartyDocuments",
    "uploadPartyDocument",
    "createVideoInvite",
    "createSignatureInvite",
    "downloadDocuments",
    "getDocumentUrl",
    "getPartyIdentityMediaUrls",
    "deleteTask",
    "removeDocument",
  ]) {
    assert.match(source, new RegExp(`async\\s+${method}\\s*\\(`), `${method} must be implemented`);
  }

  for (const token of [
    "NotaryTask",
    "Party",
    "NotaryDocument",
    "createNotaryApi",
    "getConfiguredNotaryAppSdkClient",
    "getConfiguredDriveAppSdkClient",
    "getConfiguredAppbaseAppSdkClient",
  ]) {
    assert(source.includes(token), `${notaryServicePath} must include ${token}`);
  }

  assert.doesNotMatch(
    source,
    /fetch\(|axios|Authorization|Access-Token|MockNotaryService|mockTasks|picsum\.photos/,
    "real notary service must not bypass SDKs, assemble auth headers, or keep mock data",
  );
});

imPcTest("real IM PC notary service maps generated SDK case models to existing task view models", () => {
  const source = readNotaryServiceSource();

  for (const field of [
    "orderId",
    "orderItemId",
    "skuId",
    "driveSpaceId",
    "driveFolderNodeId",
    "documents",
    "timeline",
    "PENDING_REVIEW",
    "PROCESSING",
    "COMPLETED",
    "REJECTED",
  ]) {
    assert(source.includes(field), `service must map ${field}`);
  }

  for (const sdkCall of [
    "notaryApi.listCases",
    "notaryApi.listStaff",
    "notaryApi.assignCase",
    "notaryApi.createCase",
    "notaryApi.uploadCaseFile",
    "notaryApi.createDownloadPackage",
    "notaryApi.createPartyVideoInvite",
    "notaryApi.createPartySignatureInvite",
  ]) {
    assert(source.includes(sdkCall), `service must call ${sdkCall}`);
  }
});

imPcTest("real IM PC notary service keeps operational case id separate from display case number", () => {
  const source = readNotaryServiceSource();

  assert(
    source.includes("id: stringValue(record.caseId ?? record.id)"),
    "task.id must use the backend case id used by notary app-api routes",
  );
  assert(source.includes("caseNo: optionalString(record.caseNo)"));
  assert(source.includes("caseId: optionalString(record.caseId ?? record.id)"));
  assert.doesNotMatch(
    source,
    /id:\s*stringValue\(record\.caseNo\s*\?\?\s*record\.caseId\s*\?\?\s*record\.id\)/,
    "task.id must not prefer display case number over operational case id",
  );
});

imPcTest("real IM PC notary service reuses the complete getCase aggregate for detail workflows", () => {
  const source = readNotaryServiceSource();
  const mapBody = functionBody(source, "mapCaseToTask");

  assert.match(
    source,
    /async function loadTask\(taskId: string\): Promise<NotaryTask> \{\s*return mapCaseToTask\(await notaryApi\.getCase\(taskId\)\);\s*\}/,
  );
  assert.doesNotMatch(source, /notaryApi\.listCaseEvents/);
  assert.doesNotMatch(source, /notaryApi\.listCaseFiles/);
  assert.match(mapBody, /extractItems\(record\.documents\)\.map\(mapDocument\)/);
  assert.match(mapBody, /extractItems\(record\.timeline\)\.map\(mapTimelineEvent\)/);
  assert.match(mapBody, /extractItems\(record\.parties\)\.map\(mapParty\)/);

  for (const methodName of [
    "listPartyDocuments",
    "getDocumentUrl",
    "getPartyIdentityMediaUrls",
    "removeDocument",
  ]) {
    assert.match(methodBody(source, methodName), /loadTask\(taskId\)/);
  }
});

imPcTest("real IM PC notary service creates cases with one primary staff assignment and creation uploads", () => {
  const source = readNotaryServiceSource();
  const createTaskBody = methodBody(source, "createTask");
  assert.match(createTaskBody, /data\.skuId/);
  assert.match(createTaskBody, /FALLBACK_APPLICANT_NAME/);

  for (const token of [
    "documents: data.documents ?? []",
    "primaryNotaryMembershipId: resolvePrimaryNotaryMembershipId(data)",
    "await syncInitialPartySignatures(createdTask.id, data.parties ?? [])",
    "const createdTaskForDocumentUpload = documents.some(hasDocumentPartyId)",
    "const createdPartyIdByClientPartyId = mapCreatedPartyIds(data.parties, createdTaskForDocumentUpload.parties ?? [])",
    "uploadCaseFileUnlessRegistered",
  ]) {
    assert(createTaskBody.includes(token), `createTask must include ${token}`);
  }

  assert.match(
    createTaskBody,
    /await syncInitialPartyIdentityMedia\(\s*createdTask\.id,\s*data\.parties \?\? \[\],\s*existingDocumentKeys,\s*idempotencyKey,\s*\)/,
  );
  assert.match(
    createTaskBody,
    /await syncInitialPartyAuxiliaryAttachments\(\s*createdTask\.id,\s*data\.parties \?\? \[\],\s*existingDocumentKeys,\s*idempotencyKey,\s*\)/,
  );
  assert.match(
    createTaskBody,
    /const partyId = resolveCreationDocumentPartyId\(document, createdPartyIdByClientPartyId\)[\s\S]*?partyId,\s*\n/,
  );
  assert.match(createTaskBody, /documentRecord\.uploadIntentId/);
  assert.match(createTaskBody, /uploadIntentId:/);

  assert.doesNotMatch(
    createTaskBody,
    /syncCaseAssignments|notaryApi\.assignCase/,
    "createCase already receives primaryNotaryMembershipId and must not create a duplicate assignment",
  );

  const resolveBody = functionBody(source, "resolvePrimaryNotaryMembershipId");
  assert.match(resolveBody, /primaryNotaryMembershipId/);
  assert.match(resolveBody, /notaryMembershipId/);
  assert.match(resolveBody, /selectedNotaryStaff/);
});

imPcTest("real IM PC notary create retries reuse a per-intent random idempotency key", () => {
  const source = readNotaryServiceSource();
  const createTaskBody = methodBody(source, "createTask");
  const createView = readNotaryPcText(
    "packages/sdkwork-notary-pc-notary/src/CreateNotaryTaskView.tsx",
  );
  const idempotencyKeys = readNotaryPcText(
    "packages/sdkwork-notary-pc-notary/src/utils/createCaseIdempotencyKey.ts",
  );

  assert.match(
    createView,
    /const \[idempotencyKey\] = useState\(createNotaryCaseIntentIdempotencyKey\)/,
    "one mounted create intent must keep the same key across submit retries",
  );
  assert.match(createView, /notaryService\.createTask\(\{\s*idempotencyKey,/);
  assert.match(source, /idempotencyKey\?: string/);
  assert.match(
    createTaskBody,
    /const idempotencyKey\s*=\s*resolveNotaryCaseIdempotencyKey\(data\.idempotencyKey\)/,
    "service must preserve a caller-owned intent key",
  );
  assert.match(
    createTaskBody,
    /primaryNotaryMembershipId:\s*resolvePrimaryNotaryMembershipId\(data\),[\s\S]*?idempotencyKey,\s*\n\s*\}\)/,
    "createCase must receive the resolved intent key",
  );
  assert.match(idempotencyKeys, /from ['"]@sdkwork\/utils\/id['"]/);
  assert.match(idempotencyKeys, /uuid\(\)/);
  assert.match(idempotencyKeys, /callerKey \|\| createNotaryCaseIntentIdempotencyKey\(\)/);
  assert.doesNotMatch(source, /buildIdempotencyKey/);
  assert.doesNotMatch(idempotencyKeys, /Date\.now|Math\.random|traceId|requestId/);
});

imPcTest("real IM PC notary creation deduplicates registered attachments with stable upload intents", () => {
  const source = readNotaryServiceSource();
  const createTaskBody = methodBody(source, "createTask");
  const createView = readNotaryPcText(
    "packages/sdkwork-notary-pc-notary/src/CreateNotaryTaskView.tsx",
  );
  const commonTypes = readNotaryPcText(
    "packages/sdkwork-notary-pc-commons/src/types/notary.ts",
  );
  const registrationBody = functionBody(source, "caseDocumentRegistrationKey");

  assert.match(commonTypes, /uploadIntentId\?: string/);
  assert.match(createView, /uploadIntentId:\s*attachment\.id/);
  assert.match(createTaskBody, /const existingDocumentKeys = new Set/);
  assert.match(createTaskBody, /createdTask\.documents\.map\(caseDocumentRegistrationKey\)/);
  assert.match(
    source,
    /async function uploadCaseFileUnlessRegistered[\s\S]*?if \(existingDocumentKeys\?\.has\(registrationKey\)\) \{\s*return;\s*\}[\s\S]*?await notaryApi\.uploadCaseFile\(input\);\s*existingDocumentKeys\?\.add\(registrationKey\);/,
  );
  assert.match(registrationBody, /category/);
  assert.match(registrationBody, /partyId/);
  assert.match(registrationBody, /materialCode/);
  assert.match(registrationBody, /toLowerCase\(\)/);
  assert.match(source, /uploadIntentId:\s*`\$\{uploadIntentPrefix\}:\$\{partyId\}:\$\{document\.materialCode\}`/);
  assert.match(source, /uploadIntentId:\s*`\$\{uploadIntentPrefix\}:\$\{partyId\}:\$\{index\}:\$\{materialCode\}`/);
  assert.match(createTaskBody, /optionalString\(documentRecord\.uploadIntentId\)/);
  assert.match(createTaskBody, /`\$\{idempotencyKey\}:document:\$\{index\}`/);
});

imPcTest("real IM PC notary service exposes dashboard, report, and matter SDK workflows", () => {
  const source = readNotaryServiceSource();
  const mattersBody = methodBody(source, "getMatters");

  for (const method of [
    "getDashboardStatistics",
    "getMonthlyReport",
    "getMatters",
  ]) {
    assert.match(source, new RegExp(`async\\s+${method}\\s*\\(`), `${method} must be implemented`);
  }

  for (const sdkCall of [
    "notaryApi.getDashboardStatistics",
    "notaryApi.getMonthlyReport",
    "notaryApi.listMatters",
    "notaryApi.getCase",
  ]) {
    assert(source.includes(sdkCall), `service must call ${sdkCall}`);
  }

  assert.match(mattersBody, /pageSize:\s*normalizePageSize\(filters\.pageSize,\s*20\)/);
});

imPcTest("real IM PC notary service loads staff, filters cases by SKU, and forwards pagination", () => {
  const source = readNotaryServiceSource();

  const staffBody = methodBody(source, "getStaff");
  assert.match(staffBody, /notaryApi\.listStaff\(\{/);
  assert.match(staffBody, /staffRole:\s*filters\.staffRole/);
  assert.match(staffBody, /q:\s*filters\.searchTerm/);
  assert.match(staffBody, /pageSize:\s*normalizePageSize\(filters\.pageSize,\s*20\)/);
  assert.match(staffBody, /cursor:\s*filters\.cursor/);
  assert.match(staffBody, /extractItems/);
  assert.doesNotMatch(staffBody, /fetch\(|axios|Authorization|Access-Token|MockNotaryService|mockTasks/);

  assert.match(source, /Math\.min\(100,\s*Math\.max\(1,\s*Math\.trunc\(value\)\)\)/);

  const queryBody = functionBody(source, "resolveListCaseQuery");
  for (const token of [
    "startsWith('sku-')",
    "FILTER_SKU_IDS_BY_TYPE",
    "ELECTRONIC",
    "IPR",
    "EVIDENCE",
    "VALID_CASE_STATUSES",
    "pageSize: normalizePageSize(filters?.pageSize, 20)",
    "cursor: filters?.cursor",
    "skuId",
    "businessType",
    "filters?.status",
  ]) {
    assert(source.includes(token) || queryBody.includes(token), `${notaryServicePath} must include ${token}`);
  }
  assert.doesNotMatch(
    source,
    /status:\s*filters\?\.status\s*&&\s*filters\.status\s*!==\s*["']ALL["']/,
    "service must not pass business type filter keys as notary status",
  );

  for (const [matterName, skuId] of [
    ["电子合同公证", "sku-notary-electronic-contract"],
    ["知识产权确权公证", "sku-notary-ipr"],
    ["电子证据固化", "sku-notary-evidence"],
    ["商业秘密确权", "sku-notary-trade-secret"],
    ["抽奖摇号公证", "sku-notary-lottery"],
    ["遗嘱公证", "sku-notary-will"],
    ["Electronic Contract Preservation", "sku-notary-electronic-contract"],
    ["Intellectual Property Confirmation", "sku-notary-ipr"],
    ["Electronic Evidence Preservation", "sku-notary-evidence"],
    ["Trade Secret Confirmation", "sku-notary-trade-secret"],
    ["Lottery Process Notarization", "sku-notary-lottery"],
    ["Will Notarization", "sku-notary-will"],
  ]) {
    assert(
      source.includes(`"${matterName}": "${skuId}"`) || source.includes(`'${matterName}': '${skuId}'`),
      `${notaryServicePath} must map ${matterName} to ${skuId}`,
    );
  }
});

imPcTest("real IM PC notary task table uses server cursor pages without client slicing", () => {
  const source = readNotaryServiceSource();
  const view = readNotaryPcText("packages/sdkwork-notary-pc-notary/src/NotaryView.tsx");
  const table = readNotaryPcText(
    "packages/sdkwork-notary-pc-notary/src/components/list/NotaryTaskTable.tsx",
  );
  const tasksBody = methodBody(source, "getTasks");

  assert.match(tasksBody, /notaryApi\.listCases/);
  assert.match(tasksBody, /items:\s*pageData\.items\.map\(mapCaseToTask\)/);
  assert.match(tasksBody, /pageInfo:\s*mapTaskPageInfo/);
  assert.match(source, /mode:\s*["']cursor["']/);
  assert.match(source, /nextCursor/);
  assert.match(source, /hasMore/);
  assert.match(view, /taskPageCursorByPageRef/);
  assert.match(view, /cursor:\s*pageCursor/);
  assert.match(view, /pageSize/);
  assert.doesNotMatch(view, /\.slice\(/, "server-backed task pages must not be sliced in the UI");
  assert.doesNotMatch(table, /paginatedTasks/);
});

imPcTest("real IM PC notary cancellation and party auxiliary attachments remain service-backed", () => {
  const source = readNotaryServiceSource();
  const view = readNotaryPcText("packages/sdkwork-notary-pc-notary/src/NotaryView.tsx");
  const partyDrawer = readNotaryPcText("packages/sdkwork-notary-pc-notary/src/PartyDrawer.tsx");
  const commonTypes = readNotaryPcText(
    "packages/sdkwork-notary-pc-commons/src/types/notary.ts",
  );

  const statusBody = functionBody(source, "mapStatus");
  assert.match(statusBody, /return ["']CANCELLED["']/);
  assert.match(commonTypes, /CANCELLED/);
  assert.match(view, /toast\.taskCancelled/);
  assert.match(view, /resetTaskPagination/);
  assert.match(partyDrawer, /auxiliaryAttachments:\s*attachments\.map/);
  assert.match(source, /syncPartyAuxiliaryAttachments/);
  assert.match(source, /category:\s*["']evidence["']/);
  assert.match(source, /notaryApi\.uploadCaseFile/);
});

imPcTest("real IM PC notary service keeps document operations inside generated SDK facades", () => {
  const source = readNotaryServiceSource();

  const downloadBody = methodBody(source, "downloadDocuments");
  assert.match(downloadBody, /createDownloadPackage\(taskId,\s*\{\s*\}\)/);
  assert.doesNotMatch(downloadBody, /\b(driveSpaceType|mode|documentName)\b/);

  const documentUrlBody = methodBody(source, "getDocumentUrl");
  assert.match(documentUrlBody, /typeof document === ["']string["']/);
  assert.match(documentUrlBody, /document\.nodeId \?\? document\.driveNodeId/);
  assert.match(documentUrlBody, /if \(!nodeId\)[\s\S]*const task = await loadTask\(taskId\)/);
  assert.doesNotMatch(documentUrlBody, /listCaseFiles/);
  assert.match(documentUrlBody, /createCaseFileDownloadUrl\(taskId,\s*\{/);
  assert.match(documentUrlBody, /nodeId/);
  assert.doesNotMatch(documentUrlBody, /createDownloadPackage|picsum\.photos/);

  const notaryView = readNotaryPcText("packages/sdkwork-notary-pc-notary/src/NotaryView.tsx");
  assert.match(notaryView, /getDocumentUrl\(selectedTask\.id,\s*doc,\s*\{/);
  assert.doesNotMatch(notaryView, /getDocumentUrl\(selectedTask\.id,\s*doc\.name/);

  const removeDocumentBody = methodBody(source, "removeDocument");
  assert.match(removeDocumentBody, /notaryApi\.deleteCaseFile/);
  assert.match(removeDocumentBody, /return loadTask\(taskId\)/);
  assert.doesNotMatch(removeDocumentBody, /createDownloadPackage|fetch\(|axios|Authorization|Access-Token/);
});

imPcTest("real IM PC notary service persists party identity media and party Drive files", () => {
  const source = readNotaryServiceSource();

  for (const token of [
    "getPartyIdentityMediaUrls",
    "syncPartyIdentityMedia",
    "extractPartyIdentityDocuments",
    "identity_front",
    "identity_back",
    "face_capture",
  ]) {
    assert(source.includes(token), `${notaryServicePath} must include ${token}`);
  }

  const identityUrlBody = methodBody(source, "getPartyIdentityMediaUrls");
  assert.match(identityUrlBody, /const task = await loadTask\(taskId\)/);
  assert.match(identityUrlBody, /document\.partyId === partyId/);
  assert.match(identityUrlBody, /document\.category === ["']identity["']/);
  assert.doesNotMatch(identityUrlBody, /listCaseFiles/);
  assert.match(identityUrlBody, /partyId/);
  assert.match(identityUrlBody, /createCaseFileDownloadUrl/);
  assert.match(identityUrlBody, /identityFrontUrl/);
  assert.match(identityUrlBody, /identityBackUrl/);
  assert.match(identityUrlBody, /faceImageUrl/);
  assert.doesNotMatch(identityUrlBody, /picsum\.photos/);

  const listBody = methodBody(source, "listPartyDocuments");
  assert.match(listBody, /const task = await loadTask\(taskId\)/);
  assert.match(listBody, /task\.documents\.filter/);
  assert.match(listBody, /partyId/);
  assert.doesNotMatch(listBody, /listCaseFiles/);
  assert.doesNotMatch(listBody, /fetch\(|axios|Authorization|Access-Token/);

  const uploadBody = methodBody(source, "uploadPartyDocument");
  assert.match(uploadBody, /uploadCaseFile\(\{/);
  assert.match(uploadBody, /caseId:\s*taskId/);
  assert.match(uploadBody, /category:\s*["']evidence["']/);
  assert.match(uploadBody, /partyId/);
  assert.match(uploadBody, /materialCode:\s*resolveFileMaterialCode\(file\)/);
  assert.match(uploadBody, /uploadIntentId:\s*`manual:\$\{partyId\}/);
  assert.match(uploadBody, /return loadTask\(taskId\)/);
  assert.doesNotMatch(uploadBody, /fetch\(|axios|Authorization|Access-Token/);
});

imPcTest("real IM PC notary service creates party video and signature invites through app SDK", () => {
  const source = readNotaryServiceSource();

  const videoBody = methodBody(source, "createVideoInvite");
  assert.match(videoBody, /notaryApi\.createPartyVideoInvite\(taskId,\s*partyId,\s*\{/);
  assert.match(videoBody, /purpose:\s*["']identity_verification["']/);
  assert.match(videoBody, /conversationId/);
  assert.match(videoBody, /inviteUrl/);
  assert.doesNotMatch(videoBody, /fetch\(|axios|Authorization|Access-Token/);

  const signatureInviteBody = methodBody(source, "createSignatureInvite");
  assert.match(signatureInviteBody, /notaryApi\.createPartySignatureInvite\(taskId,\s*partyId,\s*\{/);
  assert.match(signatureInviteBody, /purpose:\s*["']remote_signature["']/);
  assert.match(signatureInviteBody, /inviteUrl/);
  assert.match(signatureInviteBody, /signingUrl/);
  assert.doesNotMatch(signatureInviteBody, /fetch\(|axios|Authorization|Access-Token/);
});

imPcTest("real IM PC notary service synchronizes party edits and signatures through generated resources", () => {
  const source = readNotaryServiceSource();

  const updateBody = methodBody(source, "updateTask");
  assert.match(updateBody, /syncParties\(taskId,\s*updates\.parties\)/);
  assert.doesNotMatch(updateBody, /\bparties\s*:/);

  for (const token of [
    "notaryApi.updateParty",
    "notaryApi.addParty",
    "notaryApi.deleteParty",
    "syncPartySignature",
    "notaryApi.attachPartySignature",
    "signatureUrl: party.signatureUrl",
    "syncInitialPartySignatures(createdTask.id, data.parties ?? [])",
  ]) {
    assert(source.includes(token), `${notaryServicePath} must include ${token}`);
  }
  assert.match(methodBody(source, "addParty"), /syncPartySignature/);
  assert.doesNotMatch(functionBody(source, "mapPartyToUpdateRequest"), /signatureUrl/);
  assert.doesNotMatch(functionBody(source, "mapPartyToCreateRequest"), /signatureUrl/);
});

imPcTest("real IM PC notary service propagates aggregate versions through every case mutation", () => {
  const source = readNotaryServiceSource();
  const commonTypes = readNotaryPcText(
    "packages/sdkwork-notary-pc-commons/src/types/notary.ts",
  );
  const view = readNotaryPcText("packages/sdkwork-notary-pc-notary/src/NotaryView.tsx");
  const statusBody = methodBody(source, "updateTaskStatus");
  const updateBody = methodBody(source, "updateTask");
  const deleteBody = methodBody(source, "deleteTask");
  const mapBody = functionBody(source, "mapCaseToTask");

  assert.match(commonTypes, /version\?: string/);
  assert.match(mapBody, /version:\s*optionalString\(record\.version\)/);
  assert.match(source, /updateTaskStatus\(taskId: string, status: NotaryTask\['status'\], version\?: string\)/);
  assert.match(
    view,
    /notaryService\.updateTaskStatus\(selectedTask\.id,\s*status,\s*selectedTask\.version\)/,
  );
  assert.match(statusBody, /notaryApi\.acceptCase\(taskId,\s*\{\s*version\s*\}\)/);
  assert.match(statusBody, /notaryApi\.rejectCase\(taskId,\s*\{[\s\S]*?reason:[\s\S]*?version,/);
  assert.match(statusBody, /notaryApi\.completeCase\(taskId,\s*\{[\s\S]*?remarks:[\s\S]*?version,/);
  assert.match(updateBody, /notaryApi\.updateCase\(taskId,\s*\{[\s\S]*?version:\s*updates\.version/);
  assert.match(deleteBody, /notaryApi\.getCase\(taskId\)/);
  assert.match(deleteBody, /version:\s*optionalString\(current\.version\)/);
  assert.match(source, /getDelegate\(\)\.updateTaskStatus\(taskId, status, version\)/);
});

imPcTest("real IM PC notary service sends generated completion DTO fields and preserves party verification metadata", () => {
  const source = readNotaryServiceSource();
  const statusBody = methodBody(source, "updateTaskStatus");
  const partyBody = functionBody(source, "mapParty");

  assert.match(statusBody, /notaryApi\.completeCase\(taskId,\s*\{\s*remarks:\s*CASE_COMPLETION_REMARKS/);
  assert.doesNotMatch(statusBody, /completeCase\(taskId,\s*\{[^}]*\bresult\s*:/s);
  assert.match(statusBody, /throw new Error\(`Unsupported notary task status transition: \$\{status\}`\)/);
  assert.doesNotMatch(statusBody, /notaryApi\.updateCase/);
  assert.match(partyBody, /identityVerificationStatus/);
  assert.match(partyBody, /record\.verificationStatus/);
  assert.match(partyBody, /identityVerificationScore/);
  assert.match(partyBody, /faceCaptureTime/);
});
