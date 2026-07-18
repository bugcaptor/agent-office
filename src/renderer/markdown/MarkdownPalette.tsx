// src/renderer/markdown/MarkdownPalette.tsx
//
// VS Code Ctrl+P 유사 파일 팔레트(이슈 #10). 열리면 캐시된 목록을 즉시 보여주고
// (스토어 openPalette가 백그라운드 재스캔을 이미 트리거함), 입력으로 relPath를
// 퍼지 필터한다. ↑/↓ 선택 이동, Enter 열기, Esc 닫기. 키 이벤트는 여기서
// stopPropagation해 터미널/전역 단축키(AgentTabStrip window 리스너)로 새지 않게 한다.
//
// self-gate 관례(다이얼로그와 동일): 항상 마운트되며 팔레트가 없으면 null 렌더.
// 쿼리·선택은 스토어가 소유하므로 재오픈 시 openPalette가 초기화한다.
import { useEffect, useMemo, useRef } from "react";
import { useMarkdownStore } from "./markdownStore";
import { fuzzyFilter } from "./fuzzy";

export function MarkdownPalette() {
  const palette = useMarkdownStore((s) => s.palette);
  const listing = useMarkdownStore((s) => (s.palette ? s.listing[s.palette.root] : undefined));
  const setQuery = useMarkdownStore((s) => s.setQuery);
  const setSelectedIndex = useMarkdownStore((s) => s.setSelectedIndex);
  const closePalette = useMarkdownStore((s) => s.closePalette);
  const openFile = useMarkdownStore((s) => s.openFile);

  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);

  const query = palette?.query ?? "";
  const files = listing?.files;

  // 퍼지 필터 결과(원본 참조가 안정적이므로 query/files 변화 시에만 재계산).
  const results = useMemo(
    () => (files ? fuzzyFilter(files, query).map((r) => r.item) : []),
    [files, query],
  );

  // 선택 인덱스는 결과 길이 안으로 클램프해 표시한다(필터가 바뀌면 store 값이
  // 잠깐 범위를 벗어날 수 있음). 열기·이동은 이 clamped 값을 기준으로 한다.
  const selected = Math.min(Math.max(palette?.selectedIndex ?? 0, 0), Math.max(results.length - 1, 0));

  // 열릴 때 입력에 포커스.
  const open = palette !== null;
  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  // 선택 항목이 보이도록 스크롤(간단 처리). jsdom엔 scrollIntoView가 없으니 가드.
  useEffect(() => {
    const el = listRef.current?.children[selected] as HTMLElement | undefined;
    if (el && typeof el.scrollIntoView === "function") el.scrollIntoView({ block: "nearest" });
  }, [selected, results.length]);

  if (!palette) return null;

  const commitOpen = (index: number) => {
    const item = results[index];
    if (item) void openFile(palette.root, item.relPath, palette.agentId);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    // 전역/터미널로 새지 않게 항상 여기서 멈춘다.
    e.stopPropagation();
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex(Math.min(selected + 1, results.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex(Math.max(selected - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      commitOpen(selected);
    } else if (e.key === "Escape") {
      e.preventDefault();
      closePalette();
    }
  };

  return (
    <div
      className="md-overlay md-palette-overlay"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closePalette();
      }}
    >
      <div className="md-palette" role="dialog" aria-label="마크다운 문서 열기">
        <input
          ref={inputRef}
          className="md-palette-input"
          type="text"
          placeholder="파일 이름 또는 경로로 검색…"
          value={query}
          spellCheck={false}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
        />
        {listing?.truncated && (
          <div className="md-palette-note">파일이 많아 일부만 표시됩니다.</div>
        )}
        {files === undefined ? (
          <div className="md-palette-empty">목록을 불러오는 중…</div>
        ) : results.length === 0 ? (
          <div className="md-palette-empty">
            {files.length === 0 ? "마크다운 파일이 없습니다." : "일치하는 파일이 없습니다."}
          </div>
        ) : (
          <ul className="md-palette-list" ref={listRef} role="listbox">
            {results.map((item, i) => (
              <li
                key={item.relPath}
                role="option"
                aria-selected={i === selected}
                className={i === selected ? "md-palette-item md-palette-item-active" : "md-palette-item"}
                onMouseDown={(e) => {
                  // mousedown으로 포커스 이탈 전에 선택/열기 처리.
                  e.preventDefault();
                  setSelectedIndex(i);
                  commitOpen(i);
                }}
                onMouseEnter={() => setSelectedIndex(i)}
              >
                <span className="md-palette-item-name">{item.name}</span>
                <span className="md-palette-item-path">{item.relPath}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
