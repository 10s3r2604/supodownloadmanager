// Offers video qualities on the page, using SDM's cached yt-dlp results.

function sdmIsXPage(pageUrl) {
  try {
    return ["x.com", "www.x.com", "twitter.com", "www.twitter.com", "mobile.twitter.com"].includes(new URL(pageUrl).hostname);
  } catch { return false; }
}

function sdmXPostUrl(pageUrl) {
  try {
    const url = new URL(pageUrl);
    if (!sdmIsXPage(pageUrl) || !["http:", "https:"].includes(url.protocol)) return null;
    const post = url.pathname.match(/^\/(?:[^/]+\/status|i\/web\/status|statuses)\/\d+(?:\/video\/\d+)?(?=\/|$)/);
    return post ? `${url.origin}${post[0].replace(/^\/statuses\//, "/i/web/status/")}` : null;
  } catch { return null; }
}

// On X, each player in a timeline belongs to a different post. Find the
// nearest post link instead of asking yt-dlp to read the Home timeline.
function sdmVideoSource(video, pageUrl, title) {
  const src = video ? video.currentSrc || video.src : "";
  if (sdmIsXPage(pageUrl)) {
    const article = video?.closest('article, [role="article"], [data-testid="tweet"], [data-testid="cellInnerDiv"]');
    for (let scope = video?.parentElement; article && scope; scope = scope.parentElement) {
      const links = [...scope.querySelectorAll('a[href]')];
      if (scope.matches('a[href]')) links.unshift(scope);
      // Prefer an explicit link around this player, then the timestamp.
      // Keep /video/N: yt-dlp uses it to select a clip in a multi-video post.
      const rank = (link) => (link.contains(video) ? 4 : 0) + (link.querySelector("time") ? 2 : 0);
      links.sort((a, b) => rank(b) - rank(a));
      const post = links.flatMap((link) => {
        try {
          const url = sdmXPostUrl(new URL(link.getAttribute("href"), pageUrl).href);
          return url ? [url] : [];
        } catch { return []; }
      })
        .at(0);
      if (post) {
        pageUrl = post;
        break;
      }
      if (scope === article) break;
    }
  }
  return { pageUrl, title, mediaUrl: /^https?:/i.test(src) ? src : null };
}

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
  let contextVideo = null;
  let hideTimer = 0;
  let resetTimer = 0;
  let host = null;
  let button = null;
  let label = null;
  let menu = null;
  let menuOpen = false;
  let menuSource = null;
  let request = 0;
  let frame = 0;
  let pointer = { x: -1, y: -1 };
  let prefetched = { page: "", clientId: null, at: 0 };
  let probeTimer = 0;
  let probeWake = null;

  function refresh() {
    if (checking || Date.now() - checkedAt < RECHECK_MS) return checking;
    checking = chrome.runtime.sendMessage({ type: "video-enabled" })
      .then((value) => { enabled = Boolean(value); })
      .catch(() => { enabled = false; })
      .finally(() => { checkedAt = Date.now(); checking = null; });
    return checking;
  }

  function build() {
    if (host) return;
    host = document.createElement("sdm-video-button");
    host.setAttribute("popover", "manual");
    host.style.cssText = "all: initial; position: fixed; z-index: 2147483647; top: 0; left: 0; margin: 0; padding: 0; border: none; background: transparent; display: none; width: max-content; max-width: calc(100vw - 12px); transform: translateX(-100%);";
    const root = host.attachShadow({ mode: "closed" });
    root.innerHTML = `
      <style>
        :host { color-scheme: dark; }
        button { box-sizing: border-box; font: 600 13px/1.4 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif; cursor: pointer; }
        .trigger {
          display: inline-flex; align-items: center; gap: 7px; height: 32px; max-width: 100%; padding: 0 13px 0 9px;
          border: 1px solid rgb(255 255 255 / .16); border-radius: 9px; background: rgb(10 14 19 / .94); color: #eef3f8;
          box-shadow: 0 6px 20px rgb(0 0 0 / .35); white-space: nowrap;
        }
        .trigger span { overflow: hidden; text-overflow: ellipsis; }
        .trigger:hover, .trigger.done { background: #12b886; border-color: #2ee59d; color: #04130d; }
        .trigger.fail { background: #b42318; border-color: #f04438; color: #fff; }
        svg { width: 18px; height: 18px; flex: none; }
        .menu {
          box-sizing: border-box; position: absolute; right: 0; top: 40px; width: min(340px, calc(100vw - 12px));
          overflow-y: auto; padding: 5px; border: 1px solid #35404d; border-radius: 10px; background: #0e151e;
          color: #eef3f8; box-shadow: 0 8px 24px rgb(0 0 0 / .45); font: 13px/1.5 system-ui, sans-serif;
        }
        [hidden] { display: none; }
        .row { display: block; width: 100%; border: 0; border-radius: 6px; padding: 10px 9px; background: transparent; color: inherit; text-align: left; }
        .choice { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
        .choice > :first-child { min-width: 0; overflow-wrap: anywhere; }
        .quality { font-weight: 650; }
        .format { margin-top: 2px; font-size: 11px; color: #aebdca; }
        .size { flex: none; font-variant-numeric: tabular-nums; }
        .title { padding: 8px 9px; color: #aebdca; overflow-wrap: anywhere; font-size: 12px; }
        .row:hover, .row:focus-visible { background: #1b2b37; outline: 1px solid #2ee59d; }
        .row:disabled { opacity: .55; cursor: wait; }
        .more { margin-top: 4px; border-top: 1px solid #35404d; color: #6be0b3; }
        .status { padding: 10px 9px; overflow-wrap: anywhere; }
        .loading::before { content: ""; display: inline-block; width: 10px; height: 10px; margin-right: 8px; border: 2px solid #35404d; border-top-color: #2ee59d; border-radius: 50%; animation: spin .8s linear infinite; }
        .error { color: #ffb4a8; }
        @keyframes spin { to { transform: rotate(360deg); } }
        @media (prefers-reduced-motion: reduce) { .loading::before { animation: none; } }
      </style>
      <button class="trigger" type="button" aria-haspopup="menu" aria-expanded="false">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 4v11M7 10l5 5 5-5M5 20h14"/></svg>
        <span></span>
      </button>
      <div class="menu" role="menu" hidden></div>`;
    button = root.querySelector(".trigger");
    label = button.querySelector("span");
    menu = root.querySelector(".menu");
    menu.setAttribute("aria-label", text("videoQualities"));
    label.textContent = text("videoButton");
    button.addEventListener("click", openMenu);
    host.addEventListener("mouseenter", () => clearTimeout(hideTimer));
    host.addEventListener("mouseleave", scheduleHide);
    menu.addEventListener("keydown", (event) => {
      const rows = [...menu.querySelectorAll("button:not(:disabled)")];
      const index = rows.indexOf(root.activeElement);
      let next;
      if (event.key === "ArrowDown") next = (index + 1) % rows.length;
      else if (event.key === "ArrowUp") next = (index - 1 + rows.length) % rows.length;
      else if (event.key === "Home") next = 0;
      else if (event.key === "End") next = rows.length - 1;
      if (next != null && rows.length) { event.preventDefault(); rows[next].focus(); }
    });
    attach();
  }

  // A manual popover stays above fullscreen video, including a VIDEO element.
  function attach() {
    if (!host) return;
    const full = document.fullscreenElement;
    const parent = full && full.tagName !== "VIDEO" ? full : document.body || document.documentElement;
    if (host.parentNode !== parent) parent.appendChild(host);
    if (current && host.showPopover && !host.matches(":popover-open")) host.showPopover();
  }

  function source() {
    return sdmVideoSource(current, location.href, document.title);
  }

  async function probeDelay() {
    let wake;
    await new Promise((resolve) => {
      wake = resolve;
      probeWake = resolve;
      probeTimer = setTimeout(resolve, 1000);
    });
    if (probeWake === wake) probeWake = null;
  }

  async function probeMessage(message, token) {
    // A worker replacement can close a runtime port mid-request. Retry the
    // idempotent operation with its existing lease, within a small bound.
    for (let attempt = 0; attempt < 3; ++attempt) {
      try { return await chrome.runtime.sendMessage(message); }
      catch {
        if (!menuOpen || token !== request || attempt === 2) return null;
        await probeDelay();
        if (!menuOpen || token !== request) return null;
      }
    }
    return null;
  }

  function cancelProbe() {
    clearTimeout(probeTimer);
    probeWake?.();
    probeWake = null;
    const clientId = prefetched.clientId;
    prefetched = { page: "", clientId: null, at: 0 };
    if (clientId) chrome.runtime.sendMessage({ type: "video-probe-cancel", clientId }).catch(() => {});
  }

  function prefetch(fresh = false) {
    const selected = source();
    if (sdmIsXPage(selected.pageUrl) && !sdmXPostUrl(selected.pageUrl)) return;
    const page = `${selected.pageUrl.split("#")[0]}|${selected.mediaUrl || ""}`;
    // Opening a menu after a long hover renews the interest before the
    // desktop's 20-second idle lease can leave a cached expiry error.
    if (prefetched.page === page && prefetched.clientId && (!fresh || Date.now() - prefetched.at < 15_000)) return;
    cancelProbe();
    const clientId = crypto.randomUUID?.() || Array.from(crypto.getRandomValues(new Uint8Array(16)), (byte) => byte.toString(16).padStart(2, "0")).join("");
    prefetched = { page, clientId, at: Date.now() };
    chrome.runtime.sendMessage({ type: "video-prefetch", ...selected, clientId }).catch(() => {});
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
    // Virtualized feeds may reuse the same VIDEO element for another post.
    if (!current.isConnected) { hide(); return; }
    const selected = source();
    if (menuOpen && (selected.pageUrl !== menuSource?.pageUrl || selected.mediaUrl !== menuSource?.mediaUrl)) { closeMenu(); reset(); }
    const r = current.getBoundingClientRect();
    if (r.width < MIN_WIDTH || r.bottom < 0 || r.top > innerHeight) { hide(); return; }
    host.style.top = `${Math.max(6, Math.min(innerHeight - 38, r.top + 10))}px`;
    const width = menuOpen ? Math.min(340, innerWidth - 12) : button.getBoundingClientRect().width;
    host.style.left = `${Math.max(width + 6, Math.min(innerWidth - 6, r.right - 10))}px`;
    if (menuOpen) {
      const trigger = button.getBoundingClientRect();
      const below = innerHeight - trigger.bottom - 14;
      const above = trigger.top - 14;
      const up = below < 180 && above > below;
      menu.style.top = up ? "auto" : "40px";
      menu.style.bottom = up ? "40px" : "auto";
      menu.style.maxHeight = `${Math.max(40, up ? above : below)}px`;
    }
  }

  function show(video) {
    build();
    clearTimeout(hideTimer);
    if (current !== video) { cancelProbe(); current = video; reset(); }
    host.style.display = "block";
    attach();
    place();
    prefetch();
    scheduleHide();
  }

  function closeMenu(focus = false) {
    ++request;
    cancelProbe();
    menuOpen = false;
    menuSource = null;
    if (menu) menu.hidden = true;
    if (button) button.setAttribute("aria-expanded", "false");
    if (focus) button?.focus();
    scheduleHide();
  }

  function hide() {
    closeMenu();
    clearTimeout(hideTimer);
    if (host) {
      if (host.hidePopover && host.matches(":popover-open")) host.hidePopover();
      host.style.display = "none";
    }
    current = null;
  }

  function scheduleHide() {
    clearTimeout(hideTimer);
    if (!menuOpen) hideTimer = setTimeout(hide, HIDE_AFTER_MS);
  }

  function reset() {
    if (!button) return;
    clearTimeout(resetTimer);
    button.className = "trigger";
    label.textContent = text("videoButton");
  }

  function status(message, kind = "") {
    const line = document.createElement("div");
    line.className = `status ${kind}`;
    line.setAttribute("role", kind === "error" ? "alert" : "status");
    line.textContent = message;
    menu.replaceChildren(line);
  }

  function moreOptions() {
    if (sdmIsXPage(menuSource?.pageUrl || location.href)) return;
    const row = document.createElement("button");
    row.type = "button";
    row.className = "row more";
    row.setAttribute("role", "menuitem");
    row.textContent = text("videoMoreOptions");
    row.addEventListener("click", () => sendToApp());
    menu.appendChild(row);
  }

  function retryOptions() {
    const row = document.createElement("button");
    row.type = "button";
    row.className = "row more";
    row.setAttribute("role", "menuitem");
    row.textContent = text("videoRetry");
    row.addEventListener("click", (event) => { closeMenu(); openMenu(event); });
    menu.appendChild(row);
  }

  function optionLabel(option) {
    let quality = text("videoAudio");
    if (option.kind === "video") {
      quality = option.height ? `${option.height}p${option.fps > 30 ? option.fps : ""}` : text("videoBest");
      quality += option.height >= 2160 ? " 4K" : option.height >= 1440 ? " 2K" : option.height >= 720 ? " HD" : "";
    }
    let size = text("videoUnknownSize");
    if (Number.isFinite(option.size) && option.size >= 0) {
      const unit = option.size >= 1024 ** 3 ? "GB" : option.size >= 1024 ** 2 ? "MB" : option.size >= 1024 ? "KB" : "B";
      const divisor = { GB: 1024 ** 3, MB: 1024 ** 2, KB: 1024, B: 1 }[unit];
      size = `~${Number((option.size / divisor).toFixed(1))} ${unit}`;
    }
    const format = [option.vcodec, option.acodec, option.ext.toUpperCase()].filter(Boolean).join(" · ");
    return { quality, format, size };
  }

  async function openMenu(event) {
    event.preventDefault();
    event.stopPropagation();
    if (menuOpen) { closeMenu(); return; }
    reset();
    clearTimeout(hideTimer);
    menuOpen = true;
    menuSource = source();
    menu.hidden = false;
    button.setAttribute("aria-expanded", "true");
    const token = ++request;
    status(text("videoFindingQualities"), "loading");
    moreOptions();
    place();
    if (sdmIsXPage(menuSource.pageUrl) && !sdmXPostUrl(menuSource.pageUrl)) {
      status(text("videoPostNotFound"), "error");
      retryOptions();
      place();
      return;
    }
    // The document owns the lease and polls through independent short messages.
    // Its ID survives service-worker restarts and never carries cookies in URLs.
    prefetch(true);
    const clientId = prefetched.clientId;
    let result = await probeMessage({ type: "video-probe", ...menuSource, clientId }, token);
    while (menuOpen && token === request && result?.ok && result.state === "pending") {
      await probeDelay();
      if (!menuOpen || token !== request) return;
      result = await probeMessage({ type: "video-probe-status", clientId }, token);
    }
    if (!menuOpen || token !== request) return;
    if (!result?.ok) {
      status(result?.error || text("videoFailed"), "error");
      retryOptions();
    } else {
      const options = result.info?.options || [];
      menu.replaceChildren();
      if (!options.length) { status(text("videoNoQualities")); retryOptions(); }
      if (options.length && result.info.title) {
        const title = document.createElement("div");
        title.className = "title";
        title.textContent = result.info.title;
        menu.appendChild(title);
      }
      for (const option of options) {
        const row = document.createElement("button");
        row.type = "button";
        row.className = "row choice";
        row.setAttribute("role", "menuitem");
        const labels = optionLabel(option);
        row.setAttribute("aria-label", `${labels.quality} · ${labels.format} · ${labels.size}`);
        const description = document.createElement("span");
        const quality = document.createElement("div");
        quality.className = "quality";
        quality.textContent = labels.quality;
        const format = document.createElement("div");
        format.className = "format";
        format.textContent = labels.format;
        description.append(quality, format);
        const size = document.createElement("span");
        size.className = "size";
        size.textContent = labels.size;
        size.title = text("videoEstimatedSize");
        row.append(description, size);
        row.addEventListener("click", () => download(option.id));
        menu.appendChild(row);
      }
    }
    moreOptions();
    place();
    if (event.detail === 0) menu.querySelector('button:not(:disabled)')?.focus({ preventScroll: true });
  }

  async function download(option) {
    const token = ++request;
    for (const row of menu.querySelectorAll("button")) row.disabled = true;
    label.textContent = text("videoSending");
    const result = await chrome.runtime.sendMessage({ type: "video-add", ...menuSource, option }).catch(() => null);
    if (!menuOpen || token !== request) return;
    if (!result?.ok) {
      reset();
      status(result?.error || text("videoFailed"), "error");
      retryOptions();
      moreOptions();
      place();
      return;
    }
    button.className = "trigger done";
    label.textContent = text("videoDownloading");
    status(text("videoDownloading"));
    clearTimeout(resetTimer);
    resetTimer = setTimeout(() => { closeMenu(); reset(); hide(); }, 1500);
  }

  async function sendToApp() {
    const selected = menuSource;
    closeMenu();
    label.textContent = text("videoSending");
    const result = await chrome.runtime.sendMessage({ type: "video", ...selected }).catch(() => null);
    button.className = `trigger ${result?.ok ? "done" : "fail"}`;
    label.textContent = result?.ok ? text("videoSent") : text("videoFailed");
    clearTimeout(resetTimer);
    resetTimer = setTimeout(reset, 3000);
  }

  function onMove(event) {
    pointer = { x: event.clientX, y: event.clientY };
    if (menuOpen || frame) return;
    frame = requestAnimationFrame(async () => {
      frame = 0;
      await refresh();
      const video = visibleVideoAt(pointer.x, pointer.y);
      if (enabled && video && !menuOpen) show(video);
    });
  }

  document.addEventListener("mousemove", onMove, { capture: true, passive: true });
  document.addEventListener("contextmenu", (event) => {
    contextVideo = event.composedPath().find((element) => element.tagName === "VIDEO")
      || visibleVideoAt(event.clientX, event.clientY);
  }, true);
  chrome.runtime.onMessage.addListener((message, _sender, reply) => {
    if (message?.type !== "video-open-menu") return false;
    (async () => {
      await refresh();
      if (!enabled) { reply({ ok: false }); return; }
      const videos = [...document.querySelectorAll("video")];
      const video = contextVideo?.isConnected ? contextVideo
        : videos.find((v) => message.mediaUrl && (v.currentSrc || v.src) === message.mediaUrl);
      if (!video) { reply({ ok: false }); return; }
      closeMenu();
      show(video);
      openMenu({ preventDefault() {}, stopPropagation() {}, detail: 0 }).catch(() => {});
      reply({ ok: true });
    })().catch(() => reply({ ok: false }));
    return true;
  });
  document.addEventListener("pointerdown", (event) => {
    if (menuOpen && !event.composedPath().includes(host)) closeMenu();
  }, true);
  document.addEventListener("keydown", (event) => {
    if (menuOpen && event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      closeMenu(true);
    }
  }, true);
  addEventListener("scroll", place, { capture: true, passive: true });
  addEventListener("resize", place, { passive: true });
  addEventListener("pagehide", cancelProbe);
  document.addEventListener("fullscreenchange", () => { attach(); place(); });
})();
