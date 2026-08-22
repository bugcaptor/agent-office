// src/renderer/settings/ModelPicker.tsx
//
// 모델 id를 고르는 콤보박스(kbm #2fc). 설정 화면의 모델 칸 전부가 쓴다.
//
// 왜 `<input list=datalist>`가 아닌가: datalist는 웹뷰마다 렌더가 제각각이고
// (macOS WKWebView에서는 몇 줄만 보이고 필터도 접두사 우선이다) 목록이 수백
// 개가 되면 사실상 못 쓴다. 조회 상태("불러오는 중", "실패했으니 직접 적어라",
// "이 서비스는 목록을 안 준다")를 말해 줄 자리도 없다.
//
// 계약은 그대로 자유 텍스트다 — 목록은 힌트일 뿐이고, 사용자가 친 값은 어떤
// 경우에도 지워지거나 목록 값으로 교정되지 않는다. 목록에 없는 새 모델은
// 그냥 적어 넣으면 된다.
//
// 조회는 목록을 **처음 펼칠 때** 시작한다(`useModelCatalog(provider, opened)`).
// 설정창을 열기만 한 사용자는 네트워크도 로컬 CLI도 건드리지 않는다.
import { Fragment, useEffect, useId, useMemo, useRef, useState } from "react";
import type { ModelCatalogProvider } from "@shared/types";
import { useModelCatalog } from "./modelCatalog";

/** 한 번에 그리는 후보 상한. OpenRouter 카탈로그는 수백 개고, 그걸 다 DOM에
 *  올려 봐야 사람이 스크롤로 찾지 않는다 — 좁히려면 더 치면 된다. */
const MAX_VISIBLE = 60;

/** 입력한 조각들이 **모두** 들어 있으면 후보다(공백으로 끊어 AND 검색).
 *  `haiku 4.5`처럼 순서가 어긋나게 쳐도 걸리게 하려는 것이다. */
export function filterModels(models: string[], query: string): string[] {
  const tokens = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return models;
  return models.filter((m) => {
    const lower = m.toLowerCase();
    return tokens.every((t) => lower.includes(t));
  });
}

export function ModelPicker({
  provider,
  value,
  onChange,
  placeholder,
  ariaLabel,
}: {
  /** 목록을 물어볼 서비스. 바뀌면 목록도 그 서비스 것으로 갈린다. */
  provider: ModelCatalogProvider;
  value: string;
  onChange: (value: string) => void;
  /** 비워 뒀을 때 실제로 쓰이는 기본 모델 id. */
  placeholder?: string;
  ariaLabel: string;
}) {
  const baseId = useId();
  const listId = `${baseId}-list`;
  const [open, setOpen] = useState(false);
  // 한 번이라도 펼쳤으면 그 뒤로는 조회를 유지한다 — 닫을 때마다 캐시된
  // 결과를 버리고 로딩 문구가 다시 깜빡이면 산만하다.
  const [everOpened, setEverOpened] = useState(false);
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const popRef = useRef<HTMLDivElement | null>(null);

  const catalog = useModelCatalog(provider, everOpened);
  const filtered = useMemo(
    () => filterModels(catalog.models, value),
    [catalog.models, value],
  );
  const visible = filtered.slice(0, MAX_VISIBLE);
  // 프리셋은 목록 앞쪽 `presetCount`개다 — 필터를 통과한 뒤에도 어느 쪽
  // 출신인지 알아야 구분 머리말을 넣을 수 있다.
  const presetSet = useMemo(
    () => new Set(catalog.models.slice(0, catalog.presetCount)),
    [catalog.models, catalog.presetCount],
  );
  const hidden = filtered.length - visible.length;

  // 설정 탭 본문은 스크롤 컨테이너다(overflow-y: auto) — 화면 아래쪽 줄에서
  // 펼치면 팝업이 그 경계에 잘린다. 펼치는 순간 보이는 데까지 스크롤해 준다.
  useEffect(() => {
    if (!open) return;
    popRef.current?.scrollIntoView?.({ block: "nearest" });
  }, [open]);

  // 후보가 줄어들면 하이라이트가 목록 밖을 가리킬 수 있다.
  useEffect(() => {
    setActive((a) => (a >= visible.length ? 0 : a));
  }, [visible.length]);

  const openList = () => {
    setEverOpened(true);
    setOpen(true);
  };

  const commit = (model: string) => {
    onChange(model);
    setOpen(false);
    // 마우스로 골랐을 때도 포커스는 입력에 남아 있어야 이어서 고칠 수 있다
    // (옵션 mousedown은 preventDefault라 대개 이미 그렇다). 이미 포커스가
    // 있으면 건드리지 않는다 — focus()가 onFocus를 다시 때려 방금 접은
    // 목록이 도로 펼쳐진다.
    const input = inputRef.current;
    if (input && document.activeElement !== input) input.focus();
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (!open) openList();
      else setActive((a) => (visible.length === 0 ? 0 : (a + 1) % visible.length));
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      if (open && visible.length > 0) {
        setActive((a) => (a - 1 + visible.length) % visible.length);
      }
      return;
    }
    if (e.key === "Enter") {
      // 목록에서 고르는 중일 때만 가로챈다 — 그냥 타이핑 중의 Enter까지
      // 삼키면 사용자가 친 값이 임의로 바뀐 것처럼 보인다.
      if (open && visible.length > 0) {
        e.preventDefault();
        commit(visible[active]);
      }
      return;
    }
    if (e.key === "Escape") {
      if (open) {
        // 다이얼로그까지 닫히면 안 된다 — 목록만 접는다.
        e.stopPropagation();
        setOpen(false);
      }
      return;
    }
    if (e.key === "Tab") setOpen(false);
  };

  return (
    <div
      className="model-picker"
      onBlur={(e) => {
        // 픽커 바깥으로 포커스가 나갈 때만 접는다(입력 ↔ 목록 이동은 유지).
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setOpen(false);
      }}
    >
      <div className="model-picker-field">
        <input
          ref={inputRef}
          type="text"
          role="combobox"
          aria-label={ariaLabel}
          aria-expanded={open}
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={
            open && visible.length > 0 ? `${listId}-opt-${active}` : undefined
          }
          autoComplete="off"
          spellCheck={false}
          placeholder={placeholder}
          value={value}
          onFocus={openList}
          onChange={(e) => {
            onChange(e.target.value);
            openList();
          }}
          onKeyDown={onKeyDown}
        />
        <button
          type="button"
          className="pixel-btn model-picker-toggle"
          aria-label={`${ariaLabel} 목록`}
          aria-expanded={open}
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => {
            if (open) setOpen(false);
            else {
              openList();
              inputRef.current?.focus();
            }
          }}
        >
          ▾
        </button>
      </div>

      {open && (
        <>
          <div className="model-picker-status">
            {catalog.loading
              ? "모델 목록을 불러오는 중…"
              : catalog.failed
                ? "목록을 불러오지 못했습니다 — 모델 id를 직접 적어도 됩니다."
                : `${filtered.length}개${hidden > 0 ? ` (앞 ${MAX_VISIBLE}개 표시)` : ""}`}
            {!catalog.loading && (
              <button
                type="button"
                onMouseDown={(e) => e.preventDefault()}
                onClick={catalog.reload}
              >
                새로고침
              </button>
            )}
          </div>
          <div
            className="model-picker-pop"
            ref={popRef}
            id={listId}
            role="listbox"
            aria-label={ariaLabel}
          >
            {visible.length === 0 ? (
              <div className="model-picker-empty">
                {catalog.loading ? "불러오는 중…" : "맞는 모델이 없습니다 — 그대로 써도 됩니다."}
              </div>
            ) : (
              visible.map((m, i) => {
                // 프리셋(자주 쓰는 것)과 서비스 카탈로그의 경계에 머리말을
                // 넣는다 — 필터에 걸려 한쪽만 남으면 머리말도 하나만 뜬다.
                const preset = presetSet.has(m);
                const header =
                  i === 0
                    ? preset
                      ? "자주 쓰는 모델"
                      : "서비스 목록"
                    : !preset && presetSet.has(visible[i - 1])
                      ? "서비스 목록"
                      : null;
                return (
                  <Fragment key={m}>
                    {header && (
                      <div className="model-picker-group" role="presentation">
                        {header}
                      </div>
                    )}
                    <button
                      type="button"
                      id={`${listId}-opt-${i}`}
                      role="option"
                      aria-selected={i === active}
                      className="model-picker-option"
                      onMouseDown={(e) => e.preventDefault()}
                      onMouseEnter={() => setActive(i)}
                      onClick={() => commit(m)}
                    >
                      {m}
                    </button>
                  </Fragment>
                );
              })
            )}
          </div>
        </>
      )}
    </div>
  );
}
