/** A repository discovered from recorded agent work, with a read-only git snapshot. */
export interface RepositoryAuditItem {
  path: string;
  name: string;
  lastWorkedAt: number;
  branch: string | null;
  upstream: string | null;
  hasRemote: boolean;
  ahead: number;
  behind: number;
  changedCount: number;
  conflictCount: number;
  timedOut: boolean;
  unavailable: boolean;
}
