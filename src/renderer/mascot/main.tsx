// src/renderer/mascot/main.tsx
//
// 마스코트 창(이슈 #72)의 엔트리. 메인 창의 main.tsx와 달리 Pixi도, 부트
// 시퀀스도, 스토어도 없다 — 이 창은 main이 밀어주는 상태를 그리기만 한다.
// StrictMode의 이중 마운트는 이벤트 구독을 두 번 걸었다 푸는 것뿐이라
// 안전하지만, ready 핸드셰이크도 두 번 나가므로 굳이 켜지 않는다.
import ReactDOM from "react-dom/client";
// 별도 webview라 메인 창의 i18n 초기화를 물려받지 못한다 — 여기서도 켠다.
// 이 창은 설정을 읽지 않으므로 localStorage 캐시(메인 창이 남긴 값)가 곧 언어다.
import "../i18n";
import MascotApp from "./MascotApp";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(<MascotApp />);
