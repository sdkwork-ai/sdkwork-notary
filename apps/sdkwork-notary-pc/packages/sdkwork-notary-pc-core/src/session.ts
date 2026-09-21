import { readBootstrapAccessTokenFromProcessEnv } from '@sdkwork/iam-credential-entry';
import type { IamAppContext } from '@sdkwork/iam-contracts';
import {
  createTokenManager,
  type AuthTokenManager,
  type AuthTokens,
} from '@sdkwork/sdk-common';

export interface NotaryPcSessionUser {
  avatar?: string;
  displayName?: string;
  email?: string;
  id?: string | number;
  name?: string;
  phone?: string;
  userId?: string;
  username?: string;
}

export interface NotaryPcSessionTokens {
  accessToken?: string;
  authToken?: string;
  refreshToken?: string;
}

export interface NotaryPcSession extends NotaryPcSessionTokens {
  context?: IamAppContext;
  expiresAt?: number;
  sessionId?: string;
  user?: NotaryPcSessionUser;
}

export interface NotaryPcSessionChangedDetail {
  session: NotaryPcSession | null;
}

const ACCESS_TOKEN_KEY = 'sdkwork.accessToken';
const AUTH_TOKEN_KEY = 'sdkwork.authToken';
const NOTARY_PC_SESSION_KEY = 'sdkwork-notary-pc:session:v1';
export const NOTARY_PC_SESSION_CHANGED_EVENT = 'sdkwork-notary-pc:auth-session-changed';

let notaryPcGlobalTokenManager: AuthTokenManager | null = null;

export interface NotaryPcTokenManagerOptions {
  onSessionRefresh?: () => void;
  onSessionReset?: () => void;
}

/**
 * Private bootstrap Access-Token fallback (`APP_SDK_INTEGRATION_SPEC.md` §4).
 *
 * Delegates to the shared IAM credential-entry reader instead of reading
 * `globalThis.process.env.SDKWORK_ACCESS_TOKEN` locally: the shared reader is
 * the only sanctioned consumer of the private bootstrap artifact, because it
 * also honours the dev-server handoff that the IAM Vite plugin injects
 * (`IAM_CREDENTIAL_ENTRY_SPEC.md` §2/§5).
 */
function readDevBootstrapAccessToken(): string | undefined {
  const value = (readBootstrapAccessTokenFromProcessEnv() ?? '').trim();
  return value.length > 0 ? value : undefined;
}

function readPersistedSessionRawValue(): string | null {
  if (typeof window === 'undefined') {
    return null;
  }

  const legacyRaw = window.sessionStorage.getItem(NOTARY_PC_SESSION_KEY);
  const raw = window.localStorage.getItem(NOTARY_PC_SESSION_KEY) ?? legacyRaw;
  if (legacyRaw && !window.localStorage.getItem(NOTARY_PC_SESSION_KEY)) {
    window.localStorage.setItem(NOTARY_PC_SESSION_KEY, legacyRaw);
    window.sessionStorage.removeItem(NOTARY_PC_SESSION_KEY);
  }
  return raw;
}

function writePersistedSessionRawValue(value: string | null): void {
  if (typeof window === 'undefined') {
    return;
  }

  if (value) {
    window.localStorage.setItem(NOTARY_PC_SESSION_KEY, value);
    window.sessionStorage.removeItem(NOTARY_PC_SESSION_KEY);
  } else {
    window.localStorage.removeItem(NOTARY_PC_SESSION_KEY);
    window.sessionStorage.removeItem(NOTARY_PC_SESSION_KEY);
  }
}

function readPersistedTokens(): AuthTokens | undefined {
  const raw = readPersistedSessionRawValue();
  if (!raw) {
    return undefined;
  }

  try {
    const parsed = JSON.parse(raw) as NotaryPcSession;
    if (!parsed.accessToken && !parsed.authToken) {
      return undefined;
    }
    return {
      accessToken: parsed.accessToken,
      authToken: parsed.authToken,
      refreshToken: parsed.refreshToken,
    };
  } catch {
    return undefined;
  }
}

function readInitialTokens(): AuthTokens | undefined {
  const devAccessToken = readDevBootstrapAccessToken();
  if (devAccessToken) {
    return { accessToken: devAccessToken };
  }

  return readPersistedTokens();
}

function persistTokens(tokens: AuthTokens): void {
  if (typeof window === 'undefined') {
    return;
  }

  if (tokens.accessToken) {
    window.localStorage.setItem(ACCESS_TOKEN_KEY, tokens.accessToken);
  } else {
    window.localStorage.removeItem(ACCESS_TOKEN_KEY);
  }

  if (tokens.authToken) {
    window.localStorage.setItem(AUTH_TOKEN_KEY, tokens.authToken);
  } else {
    window.localStorage.removeItem(AUTH_TOKEN_KEY);
  }
  window.sessionStorage.removeItem(ACCESS_TOKEN_KEY);
  window.sessionStorage.removeItem(AUTH_TOKEN_KEY);
}

function clearPersistedTokens(): void {
  if (typeof window === 'undefined') {
    return;
  }

  window.localStorage.removeItem(ACCESS_TOKEN_KEY);
  window.localStorage.removeItem(AUTH_TOKEN_KEY);
  window.localStorage.removeItem(NOTARY_PC_SESSION_KEY);
  window.sessionStorage.removeItem(ACCESS_TOKEN_KEY);
  window.sessionStorage.removeItem(AUTH_TOKEN_KEY);
  window.sessionStorage.removeItem(NOTARY_PC_SESSION_KEY);
}

function emitSessionChanged(session: NotaryPcSession | null): void {
  if (typeof window === 'undefined') {
    return;
  }

  window.dispatchEvent(new CustomEvent<NotaryPcSessionChangedDetail>(NOTARY_PC_SESSION_CHANGED_EVENT, {
    detail: { session },
  }));
}

export function isNotaryPcSessionAuthenticated(session: NotaryPcSession | null | undefined): boolean {
  return Boolean(session?.accessToken?.trim() || session?.authToken?.trim());
}

export function readNotaryPcSessionTokens(): NotaryPcSession | null {
  const raw = readPersistedSessionRawValue();
  if (!raw) {
    const tokens = readPersistedTokens();
    return tokens ? { ...tokens } : null;
  }

  try {
    const parsed = JSON.parse(raw) as NotaryPcSession;
    return isNotaryPcSessionAuthenticated(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

export function applyNotaryPcSessionTokens(session: NotaryPcSession | null): NotaryPcSession | null {
  const normalized = session && isNotaryPcSessionAuthenticated(session) ? session : null;
  if (normalized) {
    writePersistedSessionRawValue(JSON.stringify(normalized));
    persistTokens(normalized);
    getNotaryPcGlobalTokenManager().setTokens({
      accessToken: normalized.accessToken,
      authToken: normalized.authToken,
      refreshToken: normalized.refreshToken,
    });
  } else {
    writePersistedSessionRawValue(null);
    clearPersistedTokens();
    getNotaryPcGlobalTokenManager().clearTokens();
  }

  emitSessionChanged(normalized);
  return normalized;
}

export function clearNotaryPcSessionTokens(): void {
  applyNotaryPcSessionTokens(null);
}

export function createNotaryPcTokenManager(
  options: NotaryPcTokenManagerOptions = {},
): AuthTokenManager {
  const manager = createTokenManager(readInitialTokens(), {
    onTokenSet: (tokens: AuthTokens) => {
      persistTokens(tokens);
      options.onSessionRefresh?.();
    },
    onTokenCleared: () => {
      clearPersistedTokens();
      options.onSessionReset?.();
    },
  });
  setNotaryPcGlobalTokenManager(manager);
  return manager;
}

export function setNotaryPcGlobalTokenManager(manager: AuthTokenManager): void {
  notaryPcGlobalTokenManager = manager;
}

export function getNotaryPcGlobalTokenManager(): AuthTokenManager {
  if (!notaryPcGlobalTokenManager) {
    notaryPcGlobalTokenManager = createNotaryPcTokenManager();
  }
  return notaryPcGlobalTokenManager;
}

// Backward-compatible aliases for app bootstrap.
export const createNotaryPcTokenManagerFromBootstrap = createNotaryPcTokenManager;
export const getTokenManager = getNotaryPcGlobalTokenManager;
export const setTokenManager = setNotaryPcGlobalTokenManager;
