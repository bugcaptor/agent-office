import { useMemo, useState, type MouseEvent, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import { marked } from "marked";
import type { ChatDownload } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { renderMarkdown } from "../markdown/renderMarkdown";
import type { Translate } from "../shared/textKey";

export type ShowNotice = (message: string) => void;

async function copyText(text: string): Promise<void> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return;
    }
  } catch {
    // 웹뷰에서 Clipboard API 쓰기가 막히면 선택 영역을 복사한다.
  }
  const focused = document.activeElement;
  const field = document.createElement("textarea");
  field.className = "chat-clipboard-field";
  field.readOnly = true;
  field.value = text;
  document.body.append(field);
  try {
    field.focus({ preventScroll: true });
    field.select();
    if (!document.execCommand("copy")) throw new Error("copy failed");
  } finally {
    field.remove();
    if (focused instanceof HTMLElement) focused.focus({ preventScroll: true });
  }
}

async function copyImage(img: HTMLImageElement): Promise<void> {
  if (!navigator.clipboard?.write || typeof ClipboardItem === "undefined") throw new Error("clipboard unavailable");
  // WebKit의 사용자 입력 유효 시간 안에 write를 호출하고, PNG 변환은 해석을 기다린다.
  const png = img.decode().then(() => new Promise<Blob>((resolve, reject) => {
    const canvas = document.createElement("canvas");
    canvas.width = img.naturalWidth;
    canvas.height = img.naturalHeight;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("canvas unavailable");
    ctx.drawImage(img, 0, 0);
    canvas.toBlob((blob) => {
      if (blob) resolve(blob);
      else reject(new Error("image conversion failed"));
    }, "image/png");
  }));
  try {
    await Promise.all([png, navigator.clipboard.write([new ClipboardItem({ "image/png": png })])]);
  } catch (error) {
    // ClipboardItem 생성 자체가 실패하면 변환의 실패를 별도로 처리한다.
    void png.catch(() => {});
    throw error;
  }
}

/** 복사하고 결과를 알린다. */
function copyWithNotice(text: string, onNotice: ShowNotice, t: Translate): void {
  void copyText(text).then(() => onNotice(t("chatView.copied")), () => onNotice(t("chatView.copyFailed")));
}

export function CopyButton({ text, message = false, onNotice }: { text: string; message?: boolean; onNotice: ShowNotice }) {
  const { t } = useTranslation("terminal");
  return (
    <button type="button" className="pixel-btn chat-action" onClick={() => copyWithNotice(text, onNotice, t)}>
      {t(message ? "chatView.copyMessage" : "chatView.copyText")}
    </button>
  );
}

export function CopyableText({ text, onNotice, className }: { text: string; onNotice: ShowNotice; className?: string }) {
  return (
    <div className="chat-text-block">
      <pre className={className}>{text}</pre>
      {text && <div className="chat-actions"><CopyButton text={text} onNotice={onNotice} /></div>}
    </div>
  );
}

export function DownloadButton({ agentId, source, filename, onNotice }: {
  agentId: string;
  source: ChatDownload;
  filename?: string;
  onNotice: ShowNotice;
}) {
  const { t } = useTranslation("terminal");
  const [saving, setSaving] = useState(false);
  const save = async () => {
    setSaving(true);
    try {
      const saved = await tauriApi.chatViewDownload(agentId, source);
      if (saved) onNotice(t("chatView.saved"));
    } catch (err) {
      onNotice(t(err === "image-gone" ? "chatView.imageGone" : "chatView.downloadFailed"));
    } finally {
      setSaving(false);
    }
  };
  return (
    <button type="button" className="pixel-btn chat-action" disabled={saving} onClick={() => void save()}>
      {filename === undefined ? t("chatView.downloadImage") : t("chatView.downloadFile", { name: filename })}
    </button>
  );
}

export function ImageActions({ agentId, imageId, imageRef, onNotice }: {
  agentId: string;
  imageId: string;
  imageRef: RefObject<HTMLImageElement>;
  onNotice: ShowNotice;
}) {
  const { t } = useTranslation("terminal");
  const copy = async () => {
    if (!imageRef.current) return;
    try {
      await copyImage(imageRef.current);
      onNotice(t("chatView.copied"));
    } catch {
      onNotice(t("chatView.imageCopyFailed"));
    }
  };
  return (
    <div className="chat-actions" onClick={(event) => event.stopPropagation()}>
      <button type="button" className="pixel-btn chat-action" onClick={() => void copy()}>{t("chatView.copyImage")}</button>
      <DownloadButton agentId={agentId} source={{ kind: "image", imageId }} onNotice={onNotice} />
    </div>
  );
}

export function ChatMarkdown({ text, onNotice }: { text: string; onNotice: ShowNotice }) {
  const { t } = useTranslation("terminal");
  const copyLabel = t("chatView.copyText");
  const html = useMemo(() => {
    const template = document.createElement("template");
    template.innerHTML = renderMarkdown(text);
    for (const code of template.content.querySelectorAll("pre > code")) {
      if (!code.textContent) continue;
      const actions = document.createElement("div");
      actions.className = "chat-actions";
      const button = document.createElement("button");
      button.className = "pixel-btn chat-action";
      button.type = "button";
      button.textContent = copyLabel;
      actions.append(button);
      code.parentElement?.after(actions);
    }
    return template.innerHTML;
  }, [text, copyLabel]);

  const handleClick = (event: MouseEvent<HTMLDivElement>) => {
    if (!(event.target instanceof Element)) return;
    if (event.target.closest("a")) event.preventDefault();
    const button = event.target.closest(".chat-actions > button.chat-action");
    if (!button || !event.currentTarget.contains(button)) return;
    const code = button.parentElement?.previousElementSibling?.querySelector(":scope > code");
    if (code?.textContent) copyWithNotice(code.textContent, onNotice, t);
  };
  return <div className="markdown-body" onClick={handleClick} dangerouslySetInnerHTML={{ __html: html }} />;
}

export function ChatPlainText({ text, onNotice }: { text: string; onNotice: ShowNotice }) {
  const tokens = useMemo(() => text.includes("```") || text.includes("~~~") ? marked.lexer(text) : null, [text]);
  if (!tokens?.some((token) => token.type === "code" && token.codeBlockStyle !== "indented")) return <div>{text}</div>;
  return (
    <div>
      {tokens.map((token, index) => token.type === "code" && token.codeBlockStyle !== "indented"
        ? <CopyableText key={index} text={token.text} onNotice={onNotice} />
        : token.raw)}
    </div>
  );
}
