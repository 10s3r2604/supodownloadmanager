// Shows a "Download this video" button over videos on web pages.
// A click sends the page to the Supo Download Manager app, which finds the
// video with yt-dlp and asks for the quality.

(() => {
  if (window.__sdmVideoButton) return;
  window.__sdmVideoButton = true;

  const MIN_WIDTH = 200;
  const MIN_HEIGHT = 112;
  const HIDE_AFTER_MS = 1800;
  const RECHECK_MS = 30_000;

  const text = (key) => chrome.i18n.getMessage(key) || key;

  let enabled = false;
  let checkedAt = 0;
  let checking = null;
  let current = null;
  let hideTimer = 0;
  let resetTimer = 0;
  let host = null;
  let button = null;
  let label = null;
  let frame = 0;
  let pointer = { x: -1, y: -1 };

  /** Asks the extension whether the button should show (app running, option on). */
  function refresh() {
    if (checking || Date.now() - checkedAt < RECHECK_MS) return checking;
    checking = chrome.runtime
      .sendMessage({ type: "video-enabled" })
      .then((value) => {
        enabled = Boolean(value);
      })
      .catch(() => {
        enabled = false;
      })
      .finally(() => {
        checkedAt = Date.now();
        checking = null;
      });
    return checking;
  }

  function build() {
    if (host) return;
    host = document.createElement("sdm-video-button");
    host.style.cssText =
      "all: initial; position: fixed; z-index: 2147483647; top: 0; left: 0; display: none; width: max-content; transform: translateX(-100%);";
    const root = host.attachShadow({ mode: "closed" });
    root.innerHTML = `
      <style>
        button {
          display: inline-flex; align-items: center; gap: 7px; height: 32px; padding: 0 13px 0 9px; white-space: nowrap;
          border: 1px solid rgb(255 255 255 / 0.16); border-radius: 9px; cursor: pointer;
          background: rgb(10 14 19 / 0.88); color: #eef3f8; backdrop-filter: blur(6px);
          font: 600 13px/1 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
          box-shadow: 0 6px 20px rgb(0 0 0 / 0.35); transition: background 0.15s, transform 0.15s;
        }
        button:hover { background: #12b886; border-color: #2ee59d; color: #04130d; transform: translateY(-1px); }
        button.done { background: #12b886; border-color: #2ee59d; color: #04130d; }
        button.fail { background: #b42318; border-color: #f04438; color: #fff; }
        svg { width: 18px; height: 18px; flex: none; }
      </style>
      <button type="button">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M12 4v11M7 10l5 5 5-5M5 20h14"/>
        </svg>
        <span></span>
      </button>`;
    button = root.querySelector("button");
    label = root.querySelector("span");
    label.textContent = text("videoButton");
    button.addEventListener("click", send);
    button.addEventListener("mouseenter", () => clearTimeout(hideTimer));
    button.addEventListener("mouseleave", scheduleHide);
    attach();
  }

  /** Keeps the button inside the element shown in full screen, if any. */
  function attach() {
    if (!host) return;
    const full = document.fullscreenElement;
    const parent = full && full.tagName !== "VIDEO" ? full : document.body || document.documentElement;
    if (host.parentNode !== parent) parent.appendChild(host);
  }

  function visibleVideoAt(x, y) {
    for (const video of document.querySelectorAll("video")) {
      const r = video.getBoundingClientRect();
      if (r.width < MIN_WIDTH || r.height < MIN_HEIGHT) continue;
      if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) return video;
    }
    return null;
  }

  function place() {
    if (!current || !host) return;
    const r = current.getBoundingClientRect();
    if (r.width < MIN_WIDTH || r.bottom < 0 || r.top > innerHeight) {
      hide();
      return;
    }
    host.style.top = `${Math.max(6, r.top + 10)}px`;
    host.style.left = `${Math.min(innerWidth - 6, r.right - 10)}px`;
  }

  function show(video) {
    build();
    clearTimeout(hideTimer);
    if (current !== video) {
      current = video;
      reset();
    }
    place();
    host.style.display = "block";
    scheduleHide();
  }

  function hide() {
    if (host) host.style.display = "none";
    current = null;
  }

  function scheduleHide() {
    clearTimeout(hideTimer);
    hideTimer = setTimeout(hide, HIDE_AFTER_MS);
  }

  function reset() {
    if (!button) return;
    clearTimeout(resetTimer);
    button.className = "";
    label.textContent = text("videoButton");
  }

  async function send(event) {
    event.preventDefault();
    event.stopPropagation();
    const video = current;
    const src = video ? video.currentSrc || video.src : "";
    label.textContent = text("videoSending");
    const result = await chrome.runtime
      .sendMessage({
        type: "video",
        pageUrl: location.href,
        title: document.title,
        mediaUrl: /^https?:/i.test(src) ? src : null,
      })
      .catch(() => null);
    button.className = result?.ok ? "done" : "fail";
    label.textContent = result?.ok ? text("videoSent") : text("videoFailed");
    clearTimeout(resetTimer);
    resetTimer = setTimeout(reset, 3000);
  }

  function onMove(event) {
    pointer = { x: event.clientX, y: event.clientY };
    if (frame) return;
    frame = requestAnimationFrame(async () => {
      frame = 0;
      const video = visibleVideoAt(pointer.x, pointer.y);
      if (!video) return;
      await refresh();
      if (enabled) show(video);
    });
  }

  document.addEventListener("mousemove", onMove, { capture: true, passive: true });
  addEventListener("scroll", place, { capture: true, passive: true });
  addEventListener("resize", place, { passive: true });
  document.addEventListener("fullscreenchange", () => {
    attach();
    place();
  });
})();
