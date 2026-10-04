// mdBook 내장 검색 대신 Pagefind 검색을 붙인다.
// 내장 검색은 한국어를 index하지 않으므로 book.toml에서 꺼 두었다.
// Pagefind index는 빌드 후 `pagefind --site book`이 book/pagefind/에 만든다(build.sh).
(function () {
  "use strict";

  // mdBook이 각 페이지 상단 inline script에서 정의하는 상대 경로("", "../" 등).
  // Pagefind는 bundlePath로 ES module을 import하는데, "pagefind/"처럼 "./"나 "/"로 시작하지 않는
  // 경로는 module 이름으로 해석되어 실패한다. 그래서 절대 URL로 바꿔 둔다.
  const root = typeof path_to_root === "string" ? path_to_root : "";
  const bundlePath = new URL(root + "pagefind/", document.baseURI).href;

  let assets = null;
  let ui = null;
  let panel = null;
  let button = null;

  function loadAssets() {
    if (assets) return assets;
    assets = new Promise(function (resolve, reject) {
      const css = document.createElement("link");
      css.rel = "stylesheet";
      css.href = bundlePath + "pagefind-ui.css";
      document.head.appendChild(css);

      const script = document.createElement("script");
      script.src = bundlePath + "pagefind-ui.js";
      script.onload = resolve;
      script.onerror = function () {
        reject(new Error("Pagefind index가 없다. guide/build.sh로 빌드했는지 확인한다."));
      };
      document.head.appendChild(script);
    });
    return assets;
  }

  function focusInput() {
    const input = panel.querySelector("input");
    if (input) input.focus();
  }

  function open() {
    panel.hidden = false;
    button.setAttribute("aria-expanded", "true");
    loadAssets()
      .then(function () {
        if (!ui) {
          ui = new PagefindUI({
            element: "#hel-search",
            bundlePath: bundlePath,
            showSubResults: true,
            showImages: false,
            resetStyles: false,
          });
        }
        focusInput();
      })
      .catch(function (err) {
        panel.querySelector("#hel-search").textContent = err.message;
      });
  }

  function close() {
    panel.hidden = true;
    button.setAttribute("aria-expanded", "false");
  }

  function toggle() {
    if (panel.hidden) open();
    else close();
  }

  function isTyping(target) {
    const tag = target && target.tagName;
    return tag === "INPUT" || tag === "TEXTAREA" || (target && target.isContentEditable);
  }

  function init() {
    const leftButtons = document.querySelector("#mdbook-menu-bar .left-buttons");
    const content = document.getElementById("mdbook-content");
    if (!leftButtons || !content) return;

    button = document.createElement("button");
    button.type = "button";
    button.className = "icon-button";
    button.id = "hel-search-toggle";
    button.title = "검색 (/)";
    button.setAttribute("aria-label", "검색 열기");
    button.setAttribute("aria-expanded", "false");
    button.setAttribute("aria-controls", "hel-search-panel");
    button.innerHTML =
      '<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">' +
      '<circle cx="10.5" cy="10.5" r="6.5" fill="none" stroke="currentColor" stroke-width="2.2"/>' +
      '<line x1="15.5" y1="15.5" x2="21" y2="21" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"/>' +
      "</svg>";
    button.addEventListener("click", toggle);
    leftButtons.appendChild(button);

    panel = document.createElement("div");
    panel.id = "hel-search-panel";
    panel.hidden = true;
    panel.innerHTML = '<div id="hel-search"></div>';
    content.parentNode.insertBefore(panel, content);

    document.addEventListener("keydown", function (event) {
      if (event.key === "Escape" && !panel.hidden) {
        close();
        return;
      }
      if ((event.key === "/" || event.key === "s") && !isTyping(event.target) && !event.ctrlKey && !event.metaKey && !event.altKey) {
        event.preventDefault();
        open();
      }
    });
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
