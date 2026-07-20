// src/shared/types.ts
//
// FROZEN CONTRACT — this file is the source of truth for the renderer<->backend
// boundary. `src-tauri/src/types.rs` is a serde mirror and must stay in exact
// field-by-field agreement with the types below.
//
// Field mapping rule: TS camelCase <-> Rust struct #[serde(rename_all =
// "camelCase")] snake_case fields; TS string-literal unions <-> Rust enum
// #[serde(rename_all = "lowercase")] PascalCase variants; `T | undefined`
// fields <-> Rust `Option<T>` with `skip_serializing_if`.
//
// This file is a barrel: the actual type/enum/function definitions live in
// `./types/*.ts`, split by domain. TS resolves a sibling `types.ts` file
// before a `types/` directory, so every existing `from '.../shared/types'`
// import keeps working unchanged. `export *` re-exports both types and
// values (e.g. the `notificationType` helper) from each domain module.

export * from './types/common';
export * from './types/session';
export * from './types/notification';
export * from './types/bot';
export * from './types/profile';
export * from './types/diary';
export * from './types/usage';
export * from './types/settings';
export * from './types/markdown';
export * from './types/git';
export * from './types/api';
