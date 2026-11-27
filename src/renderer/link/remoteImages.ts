// src/renderer/link/remoteImages.ts
//
// 머신 연동 호스트: 다른 머신 캐릭터의 초상·커스텀 시트·미니미. 스냅숏(`link-remote`)에 실린 판(epoch ms)이
// 바뀌면 원격 id(`기기id:캐릭터id`)로 `load_portrait`·`load_sprite`·`load_minimi`를 불러(백엔드가 그 머신에 묻는다)
// 이 머신 캐릭터와 같은 캐시(스토어 초상·미리보기, 시트·미니미 덮어쓰기)에 원격 id로 채운다. 그래서 오피스 층·
// 업무 패널 얼굴·토의 분신이 따로 손대지 않아도 커스텀 모습을 그린다.
//
// 끊긴 머신은 묻지 않는다(다시 붙으면 그때 받는다). 판이 사라지면 그 그림을 버린다.
import type { RemoteOffice } from "@shared/types";
import { useAppStore } from "../store/appStore";
import { loadPortraitsFor } from "../portrait/portraitCache";
import { loadSpritesFor } from "../sprite/spriteCache";
import { loadMinimisFor } from "../sprite/minimiCache";
import { clearSpriteOverride } from "../office/gen/spriteOverrides";
import { clearMinimiOverride } from "../office/gen/minimiOverrides";
import { useRemoteFloors } from "./remoteFloors";

type Kind = "portrait" | "sprite" | "minimi";

export interface RemoteImageIo {
  load: (kind: Kind, ids: string[]) => Promise<void>;
  clear: (kind: Kind, id: string) => void;
}

const defaultIo: RemoteImageIo = {
  load: (kind, ids) => (kind === "portrait" ? loadPortraitsFor(ids) : kind === "sprite" ? loadSpritesFor(ids) : loadMinimisFor(ids)),
  clear: (kind, id) => {
    const s = useAppStore.getState();
    if (kind === "portrait") s.removePortrait(id);
    else if (kind === "sprite") {
      clearSpriteOverride(id);
      s.removeSpritePreview(id);
    } else {
      clearMinimiOverride(id);
      s.removeMinimiPreview(id);
    }
  },
};

/** 원격 id·종류별로 받아 둔(또는 받는 중인) 판. */
const loaded = new Map<string, number>();

const keyOf = (kind: Kind, id: string) => `${kind}|${id}`;

/** 스냅숏에 맞춰 받을 것은 받고 버릴 것은 버린다. 받은 뒤 `imagesRev`를 올려 씬이 층을 다시 맞추게 한다. */
export function syncRemoteImages(office: RemoteOffice, io: RemoteImageIo = defaultIo): Promise<void> {
  const want = new Map<string, { kind: Kind; id: string; version: number | null; online: boolean }>();
  for (const m of office.machines) {
    for (const c of m.characters) {
      const versions: Record<Kind, number | null | undefined> = {
        portrait: c.portrait,
        sprite: c.appearance?.spriteUpdatedAt,
        minimi: c.appearance?.minimiUpdatedAt,
      };
      for (const kind of ["portrait", "sprite", "minimi"] as const) {
        want.set(keyOf(kind, c.id), { kind, id: c.id, version: versions[kind] ?? null, online: m.online });
      }
    }
  }
  // 사라진 캐릭터(해제된 머신 포함)의 그림을 버린다.
  for (const key of [...loaded.keys()]) {
    if (want.get(key)?.version != null) continue;
    const [kind, id] = key.split("|", 2) as [Kind, string];
    loaded.delete(key);
    io.clear(kind, id);
  }
  const jobs: Promise<void>[] = [];
  for (const [key, w] of want) {
    if (w.version == null) continue;
    if (!w.online) {
      // 받아 둔 것은 그대로 보이고, 못 받은 것은 다시 붙은 뒤에 받는다.
      continue;
    }
    if (loaded.get(key) === w.version) continue;
    loaded.set(key, w.version);
    jobs.push(io.load(w.kind, [w.id]));
  }
  if (!jobs.length) return Promise.resolve();
  return Promise.all(jobs).then(() => {
    useRemoteFloors.setState((s) => ({ imagesRev: s.imagesRev + 1 }));
  });
}

/** 테스트용: 받아 둔 판을 비운다. */
export function resetRemoteImages(): void {
  loaded.clear();
}

/** 스냅숏이 바뀔 때마다 맞춘다. bootstrap에서 한 번. */
export function installRemoteImages(): () => void {
  void syncRemoteImages(useRemoteFloors.getState().office);
  return useRemoteFloors.subscribe((s, prev) => {
    if (s.office !== prev.office) void syncRemoteImages(s.office);
  });
}
