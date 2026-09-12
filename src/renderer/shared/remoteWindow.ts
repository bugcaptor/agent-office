/** Whether this renderer belongs to the web-remote client window. */
export function isRemoteWindow(): boolean {
  return (
    typeof window !== "undefined" &&
    new URLSearchParams(window.location.search).get("remote") === "1"
  );
}
