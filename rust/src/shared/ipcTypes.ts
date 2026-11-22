// IPC payload types shared with the Rust backend (src-tauri). Types only.


import type { ArchivedTab } from './profileTypes';
import type { DohProviderId, ProxyProtocol, VpnConnectionState, VpnMode } from './vpnPresets';


export type { VpnMode, DohProviderId, ProxyProtocol, VpnConnectionState };


export interface HistoryEntry {
  id: string;
  url: string;
  title: string;
  timestamp: number;
  visitCount: number;
  favicon?: string;
  spaceId?: string;
}

export type HistoryClearRange = 'hour' | 'day' | 'all';

export interface HistoryQueryOptions {
  query?: string;
  limit?: number;
  offset?: number;
  spaceId?: string;
}

export interface VpnProxyConfig {
  protocol: ProxyProtocol;
  host: string;
  port: number;
  username?: string;
  password?: string;
  bypassRules: string;
}

export interface VpnConfig {
  enabled: boolean;
  mode: VpnMode;
  dohProvider: DohProviderId;
  webrtcGuard: boolean;
  killSwitch: boolean;
  proxy: VpnProxyConfig;
}

export interface OsVpnAdapter {
  name: string;
  addresses: string[];
}

export interface VpnIpCheckResult {
  ip: string;
  location?: string;
  colo?: string;
  latencyMs: number;
  checkedAt: number;
  via: string;
}

export interface VpnStatus {
  state: VpnConnectionState;
  config: VpnConfig;
  activeProxyRule: string | null;
  activeDnsUrl: string | null;
  webrtcProtected: boolean;
  killSwitchBlocking: boolean;
  killSwitchBlockedCount: number;
  detectedOsVpn: OsVpnAdapter | null;
  lastError: string | null;
  connectedSince: number | null;
  ipCheck: VpnIpCheckResult | null;
}

export interface ArchiveListEntry extends ArchivedTab {
  spaceName: string | null;
}

export interface CredentialListItem {
  id: string;
  origin: string;
  username: string;
  createdAt: number;
  lastUsedAt: number;
  unreadable: boolean;
}

export type PasswordErrorCode =
  | 'encryption-unavailable'
  | 'vault-corrupt'
  | 'invalid-origin'
  | 'not-found'
  | 'unreadable'
  | 'invalid-input'
  | 'invalid-action'
  | 'expired';

export interface PasswordError {
  ok: false;
  error: PasswordErrorCode;
  message: string;
}

export interface PromptInfo {
  promptId: string;
  kind: 'save' | 'update';
  profileId: string;
  webContentsId: number;
  origin: string;
  username: string;
}

export type PromptAction = 'save' | 'update' | 'never' | 'not-now' | 'dismiss';

export type PromptResult =
  | { ok: true; action: PromptAction; credential?: CredentialListItem }
  | PasswordError;

export type VerifyFailure = { status: 'cancelled' | 'failed'; message: string };

export type NeedsConfirm = { status: 'needs-confirm'; confirmId: string };

export type RevealResult =
  | { status: 'ok'; credentialId: string; password: string }
  | NeedsConfirm
  | VerifyFailure
  | ({ status: 'error' } & PasswordError);

export type CopyResult =
  | { status: 'copied'; credentialId: string; clearAfterMs: number }
  | NeedsConfirm
  | VerifyFailure
  | ({ status: 'error' } & PasswordError);

export interface ImportConflictInfo {
  index: number;
  origin: string;
  username: string;
  conflictsWith: 'existing' | 'import';
}

export type PrepareImportResult =
  | {
      ok: true;
      importId: string;
      counts: { imported: number; duplicate: number; conflict: number; invalid: number; total: number };
      conflicts: ImportConflictInfo[];
    }
  | PasswordError
  | { ok: false; error: 'csv-empty' | 'csv-missing-columns'; message: string; missing?: string[] };

export type ConflictChoice = 'keep' | 'replace';

export type CommitImportResult =
  | { ok: true; imported: number; replaced: number; kept: number; duplicate: number; invalid: number }
  | PasswordError;

export type NotPossibleCause =
  | 'unsupported-platform' // not Windows
  | 'no-browser-data' // no Chrome/Edge profile with a Login Data file
  | 'key-unavailable' // Local State missing/invalid, or DPAPI unwrap failed
  | 'app-bound' // v20 / App_Bound_Encryption blobs present
  | 'decrypt-failed' // v10 decrypt failed and nothing else imported
  | 'read-failed';

export interface CsvExportSteps {
  chrome: string[];
  edge: string[];
}

export interface DirectImportNotPossible {
  ok: false;
  reason: 'not-possible';
  cause: NotPossibleCause;
  csvSteps: CsvExportSteps;
}
