// WebView의 다시 로드 메뉴만 막고, 앱의 우클릭 핸들러에는 이벤트를 전달한다.
window.addEventListener("contextmenu", (event) => event.preventDefault(), { capture: true });
