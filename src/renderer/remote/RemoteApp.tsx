import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import App from "../App";
import { bootRemoteApp } from "../remoteBootstrap";
import { acceptRemoteSnapshot, currentRemoteState, remoteStateKey, connectRemote, createRemoteApi, disconnectRemote, loadRemoteSnapshot } from "../ipc/remoteApi";
import { setTauriApiDelegate, setTauriApiFailClosed } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";

type ConnectionState = "form" | "connecting" | "connected" | "reconnecting" | "disconnected";

export default function RemoteApp() {
  const { t } = useTranslation("remote");
  const [url, setUrl] = useState("");
  const [token, setToken] = useState("");
  const [state, setState] = useState<ConnectionState>("form");
  const [error, setError] = useState<string | null>(null);
  const [hostName, setHostName] = useState<string | null>(null);
  const [opened, setOpened] = useState(false);
  const ready = useRef(false);
  const disposeBoot = useRef<(() => void) | null>(null);

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
          useAppStore.getState().setSessionState({ agentId: id, status: session?.state === "running" ? "running" : session?.state === "exited" ? "exited" : "idle" });
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
    setError(null);
    setState("connecting");
    try {
      const connection = await connectRemote(url.trim(), token);
      // The token remains only in the native connection state. Do not put it
      // in localStorage, URL query parameters, or a React error message.
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
      setError(cause instanceof Error ? cause.message : t("connection.failed"));
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
        <label>{t("connection.address")}<input required value={url} onChange={(e) => setUrl(e.target.value)} placeholder={t("connection.addressPlaceholder")} autoComplete="url" /></label>
        <label>{t("connection.token")}<input required type="password" value={token} onChange={(e) => setToken(e.target.value)} autoComplete="off" /></label>
        <p className="remote-token-help">{t("connection.tokenHelp")}</p>
        {error && <p role="alert" className="remote-error">{error}</p>}
        <button disabled={state === "connecting"}>{state === "connecting" ? t("connection.connecting") : t("connection.connect")}</button>
      </form>
    </main>
  );
}
