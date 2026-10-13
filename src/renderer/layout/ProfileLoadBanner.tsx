// src/renderer/layout/ProfileLoadBanner.tsx
//
// profiles.json을 읽지 못했을 때(수정 F2) 상단에 붙는 경고. 이 동안 프로필
// 저장은 막혀 있으므로(persist.ts) 사용자가 그 사실과 백엔드 사유(백업 경로
// 포함)를 보고 재시도할 수 있게 한다. 정상 상태에서는 아무것도 그리지 않는다.
import { useTranslation } from "react-i18next";

import { retryProfileLoad, useProfileLoadStore } from "../store/profileLoad";

export function ProfileLoadBanner() {
  const { t } = useTranslation("app");
  const error = useProfileLoadStore((s) => s.error);
  const retrying = useProfileLoadStore((s) => s.retrying);
  if (error === null) return null;
  return (
    <div className="profile-load-banner" role="alert">
      <div className="profile-load-banner-text">
        <p>{t("profileLoad.body")}</p>
        <p className="profile-load-banner-detail">{error}</p>
      </div>
      <button className="pixel-btn" disabled={retrying} onClick={() => void retryProfileLoad()}>
        {t(retrying ? "profileLoad.retrying" : "profileLoad.retry")}
      </button>
    </div>
  );
}
