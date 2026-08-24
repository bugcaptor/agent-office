// src/test-setup.ts — vitest `setupFiles`(테스트 파일마다 한 번씩 실행).
//
// 테스트의 UI 언어를 정본(ko)으로 못박는다. 그러지 않으면 jsdom의
// `navigator.language`가 `en-US`라 컴포넌트가 영어로 렌더되고, 화면 문구를
// 한국어로 단언하는 기존 테스트가 전부 깨진다.
//
// 방향을 뒤집어 테스트를 영어로 고치는 선택지도 있었지만, ko가 정본
// 카탈로그(SOURCE_LANGUAGE)이므로 **테스트가 곧 ko 문구의 명세**로 남는 편이
// 낫다. 영어 화면은 자동 단언이 아니라 phase별 눈검증으로 확인한다(문구가
// 맞는지가 아니라 길이가 레이아웃을 깨지 않는지가 관건이라 어차피 눈이 필요하다).
//
// 특정 테스트에서 다른 언어를 보고 싶으면 그 파일에서 `initI18nForTest("en")`을
// 부르고 `afterEach`로 되돌린다.
import { initI18nForTest } from "@renderer/i18n";

await initI18nForTest();
