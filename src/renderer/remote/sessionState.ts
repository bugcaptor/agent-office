import type { SessionStatus } from "@shared/types";

/** Translate the host snapshot's optional session state for the local store. */
export function remoteSessionStatus(state: string | undefined): SessionStatus {
  switch (state) {
    case "starting":
    case "running":
    case "exited":
      return state;
    default:
      return "idle";
  }
}
