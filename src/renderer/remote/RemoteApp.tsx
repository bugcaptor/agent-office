import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import App from "../App";
import { bootRemoteApp } from "../remoteBootstrap";
import { acceptRemoteSnapshot, currentRemoteState, remoteStateKey, connectRemote, createRemoteApi, disconnectRemote, loadRemoteSnapshot, loadRemoteConnection } from "../ipc/remoteApi";
import { setTauriApiDelegate, setTauriApiFailClosed } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import { remoteSessionStatus } from "./sessionState";

type ConnectionState = "form" | "connecting" | "connected" | "reconnecting" | "disconnected";

export default function RemoteApp() {
  const { t } = useTranslation("remote");
  const [url, setUrl] = useState("");
  const [token, setToken] = useState("");
  const [state, setState] = useState<ConnectionState>("form");
  const [error, setError] = useState<string | null>(null);
  const [hostName, setHostName] = useState<string | null>(null);
  const [opened, setOpened] = useState(false);
  const [loadingSavedConnection, setLoadingSavedConnection] = useState(true);
  const connectionEdited = useRef(false);
  const ready = useRef(false);
  const disposeBoot = useRef<(() => void) | null>(null);

  useEffect(() => {
    let disposed = false;
    void loadRemoteConnection().then((saved) => {
      // A slow disk read must not overwrite a new address/token being typed.
      if (disposed || connectionEdited.current || !saved) return;
      setUrl(saved.url);
      setToken(saved.token);
    }).catch(() => {
      if (!disposed) setError(t("connection.savedLoadFailed"));
    }).finally(() => {
      if (!disposed) setLoadingSavedConnection(false);
    });
    return () => { disposed = true; };
  }, []);

  useEffect(() => {
    let disposed = false;
    const teardowns: Array<() => void> = [];
    let refreshTimer: ReturnType<typeof setTimeout> | null = null;
    const refresh = () => {
      if (refreshTimer !== null) clearTimeout(refreshTimer);
      // Let the local profile debounce save first, then fetch its resulting
      // revision so a server echo never overwrites a just-edited profile.
      refreshTimer = setTimeout(() => void loadRemoteSnapshot().then((next) => {
        if (disposed || !ready.current) return;
        if (acceptRemoteSnapshot(next) && remoteStateKey(currentRemoteState()) !== remoteStateKey(next.state)) {
          useAppStore.getState().hydrate(next.state);
        }
        const live = new Map(next.sessions.map((session) => [session.agentId, session]));
        for (const id of useAppStore.getState().agentOrder) {
          const session = live.get(id);
          useAppStore.getState().setSessionState({ agentId: id, status: remoteSessionStatus(session?.state) });
          if (session) useAppStore.getState().setSessionSize(id, session.cols, session.rows);
        }
      }).catch(() => {}), 650);
    };
    const onError = (event: Event) => setError((event as CustomEvent<string>).detail);
    window.addEventListener("remote-client-error", onError);
    void listen<{ status: ConnectionState; error?: string }>("remote-connection", (event) => {
      if (disposed) return;
      if (!ready.current && event.payload.status === "connected") return;
      setState(event.payload.status);
      setError(event.payload.error ?? null);
      if (event.payload.status === "connected") refresh();
    }).then((unlisten) => {
      if (disposed) unlisten();
      else teardowns.push(unlisten);
    });
    void listen("remote:agents", refresh).then((unlisten) => {
      if (disposed) unlisten();
      else teardowns.push(unlisten);
    });
    void listen("remote:error", () => {
      if (!disposed) setError(t("connection.operationFailed"));
    }).then((unlisten) => {
      if (disposed) unlisten();
      else teardowns.push(unlisten);
    });
    return () => {
      disposed = true;
      teardowns.forEach((unlisten) => unlisten());
      window.removeEventListener("remote-client-error", onError);
      if (refreshTimer !== null) clearTimeout(refreshTimer);
      disconnectRemote();
      disposeBoot.current?.();
      disposeBoot.current = null;
      setTauriApiFailClosed();
      delete document.documentElement.dataset.remoteWindow;
    };
  }, []);

  const connect = async (event: React.FormEvent) => {
    event.preventDefault();
    if (loadingSavedConnection || state === "connecting") return;
    setError(null);
    setState("connecting");
    try {
      const connection = await connectRemote(url.trim(), token);
      // Native connection success remembers the credentials on this PC.
      // Never put the token in localStorage, URLs, or error messages.
      const snapshot = await loadRemoteSnapshot();
      setTauriApiDelegate(createRemoteApi(snapshot));
      disposeBoot.current?.();
      disposeBoot.current = await bootRemoteApp(snapshot);
      document.documentElement.dataset.remoteWindow = "true";
      setHostName(connection.hostName);
      ready.current = true;
      setOpened(true);
      setState("connected");
      setToken("");
    } catch (cause) {
      ready.current = false;
      disconnectRemote();
      disposeBoot.current?.();
      disposeBoot.current = null;
      setTauriApiFailClosed();
      delete document.documentElement.dataset.remoteWindow;
      const code = cause instanceof Error ? cause.message : cause;
      setError(t(code === "remote-connection-save-failed" ? "connection.savedSaveFailed" : "connection.failed"));
      setState("form");
    }
  };

  if (opened) {
    const banner = state === "reconnecting"
      ? t("connection.reconnecting")
      : state === "disconnected"
        ? t("connection.disconnected")
        : t("connection.connected", { host: hostName });
    return <div className="remote-window"><div className="remote-banner">{banner}{error && <span role="alert"> — {t("connection.operationFailed")}</span>}</div><App /></div>;
  }
  return (
    <main className="remote-connect-shell">
      <form className="remote-connect-card" onSubmit={connect}>
        <h1>{t("connection.title")}</h1>
        <p>{t("connection.description")}</p>
        <label>{t("connection.address")}<input required value={url} onChange={(e) => { connectionEdited.current = true; setUrl(e.target.value); }} placeholder={t("connection.addressPlaceholder")} autoComplete="url" /></label>
        <label>{t("connection.token")}<input required type="password" value={token} onChange={(e) => { connectionEdited.current = true; setToken(e.target.value); }} autoComplete="off" /></label>
        <p className="remote-token-help">{t("connection.tokenHelp")}</p>
        {error && <p role="alert" className="remote-error">{error}</p>}
        <button disabled={loadingSavedConnection || state === "connecting"}>{state === "connecting" ? t("connection.connecting") : t("connection.connect")}</button>
      </form>
    </main>
  );
}
