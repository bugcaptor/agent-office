// src/shared/types/common.ts
//
// Domain slice of the frozen renderer<->backend type contract (see barrel
// src/shared/types.ts for the overview comment). Opaque id types shared
// across every other domain slice.

/** Opaque id types. Both are plain strings on the wire. */
export type AgentId = string;
export type SessionId = string;
