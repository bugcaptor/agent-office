// src/renderer/profile/ArchetypePicker.tsx
//
// 아키타입(종족)을 고르는 콤보박스. 목록은 힌트일 뿐이고, 계약은 자유
// 텍스트다 — 목록에 없는 종족(예: "드래곤", "고양이 마법사")을 그대로 적어
// 넣을 수 있고 친 값이 임의로 교정되지 않는다.
//
// 적어 넣은 종족은 스프라이트 파츠가 없으므로 도트 생성은 human으로 폴백하고
// (archetypes.resolveArchetype), 초상/픽셀아트 프롬프트에는 그 문구가 주제
// 서술자로 그대로 들어간다(promptBuilder).
//
// 겉모습·키보드 조작은 설정창 모델 콤보박스(ModelPicker)와 같은 관례를 쓴다
// — 스타일 클래스(.combo-picker*)도 공유한다.
import { useEffect, useId, useMemo, useRef, useState } from "react";
import {
  ARCHETYPE_SELECT_OPTIONS,
  archetypeInputText,
  normalizeArchetypeInput,
  customArchetypeSubject,
} from "../office/gen/archetypes";

/** 친 글자가 라벨/id 어디에든 들어 있으면 후보. 빈 질의면 전부. */
export function filterArchetypeOptions(
  query: string,
): ReadonlyArray<{ value: string; label: string }> {
  const q = query.trim().toLowerCase();
  if (!q) return ARCHETYPE_SELECT_OPTIONS;
  const hit = ARCHETYPE_SELECT_OPTIONS.filter(
    (o) => o.label.toLowerCase().includes(q) || o.value.toLowerCase().includes(q),
  );
  // 걸리는 게 없으면 목록을 숨기지 않고 전부 보여 준다 — 커스텀을 치는 중에
  // 목록이 사라졌다 나타났다 하면 산만하고, 되돌아갈 길도 없어진다.
  return hit.length > 0 ? hit : ARCHETYPE_SELECT_OPTIONS;
}

export function ArchetypePicker({
  value,
  onChange,
  ariaLabel = "아키타입",
}: {
  /** 저장값: "auto" | 알려진 id | 자유 텍스트. */
  value: string;
  onChange: (value: string) => void;
  ariaLabel?: string;
}) {
  const baseId = useId();
  const listId = `${baseId}-list`;
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const popRef = useRef<HTMLDivElement | null>(null);

  const text = archetypeInputText(value);
  const custom = customArchetypeSubject(value);
  // 목록에서 고른 값이면(=커스텀이 아니면) 좁히지 않는다 — 라벨을 그대로
  // 질의로 쓰면 고르자마자 목록이 그 한 줄로 쪼그라들어 다시 못 바꾼다.
  const visible = useMemo(() => filterArchetypeOptions(custom ?? ""), [custom]);

  useEffect(() => {
    if (!open) return;
    popRef.current?.scrollIntoView?.({ block: "nearest" });
  }, [open]);

  useEffect(() => {
    setActive((a) => (a >= visible.length ? 0 : a));
  }, [visible.length]);

  const commit = (v: string) => {
    onChange(v);
    setOpen(false);
    const input = inputRef.current;
    if (input && document.activeElement !== input) input.focus();
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (!open) setOpen(true);
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
      // 목록을 펼쳐 고르는 중일 때만 가로챈다 — 자유 텍스트를 치는 중의
      // Enter까지 삼키면 친 값이 멋대로 목록 값으로 바뀐 것처럼 보인다.
      if (open && visible.length > 0) {
        e.preventDefault();
        commit(visible[active].value);
      }
      return;
    }
    if (e.key === "Escape") {
      if (open) {
        e.stopPropagation(); // 다이얼로그까지 닫히면 안 된다 — 목록만 접는다
        setOpen(false);
      }
      return;
    }
    if (e.key === "Tab") setOpen(false);
  };

  return (
    <div
      className="combo-picker"
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setOpen(false);
      }}
    >
      <div className="combo-picker-field">
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
          placeholder="자동(시드) — 목록에 없는 종족도 적을 수 있습니다"
          value={text}
          onFocus={() => setOpen(true)}
          onChange={(e) => {
            onChange(normalizeArchetypeInput(e.target.value));
            setOpen(true);
          }}
          onKeyDown={onKeyDown}
        />
        <button
          type="button"
          className="pixel-btn combo-picker-toggle"
          aria-label={`${ariaLabel} 목록`}
          aria-expanded={open}
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => {
            if (open) setOpen(false);
            else {
              setOpen(true);
              inputRef.current?.focus();
            }
          }}
        >
          ▾
        </button>
      </div>

      {custom && (
        <div className="combo-picker-status">
          목록에 없는 종족 — 그림 의뢰에 그대로 쓰이고, 도트 캐릭터는 인간 체형을
          씁니다.
        </div>
      )}

      {open && (
        <div
          className="combo-picker-pop"
          ref={popRef}
          id={listId}
          role="listbox"
          aria-label={ariaLabel}
        >
          {visible.map((o, i) => (
            <button
              key={o.value}
              type="button"
              id={`${listId}-opt-${i}`}
              role="option"
              aria-selected={i === active}
              className="combo-picker-option"
              onMouseDown={(e) => e.preventDefault()}
              onMouseEnter={() => setActive(i)}
              onClick={() => commit(o.value)}
            >
              {o.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
