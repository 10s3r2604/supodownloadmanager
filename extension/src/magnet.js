// Opens magnet links in Supo Download Manager. When SDM is not running,
// the browser handles them as usual (it asks which program to use).

(() => {
  if (window.__sdmMagnets) return;
  window.__sdmMagnets = true;

  const SELECTOR = 'a[href^="magnet:" i]';
  const RECHECK_MS = 30_000;

  let enabled = false;
  let checkedAt = 0;
  let checking = null;

  function refresh() {
    if (checking || Date.now() - checkedAt < RECHECK_MS) return;
    checking = chrome.runtime
      .sendMessage({ type: "magnets-enabled" })
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
  }

  const magnetOf = (target) => (target instanceof Element ? target.closest(SELECTOR) : null);

  if (document.querySelector(SELECTOR)) refresh();

  // Watch for dynamically loaded magnet links (SPA, AJAX, modal dialogs)
  if (typeof MutationObserver === "function") {
    const observer = new MutationObserver(() => {
      if (!enabled && document.querySelector(SELECTOR)) refresh();
    });
    observer.observe(document.documentElement || document.body, { childList: true, subtree: true });
  }

  // Pre-check on hover, touch/pointer press, or keyboard focus
  const onTrigger = (event) => {
    if (magnetOf(event.target)) refresh();
  };
  document.addEventListener("pointerover", onTrigger, { capture: true, passive: true });
  document.addEventListener("pointerdown", onTrigger, { capture: true, passive: true });
  document.addEventListener("focusin", onTrigger, { capture: true, passive: true });

  document.addEventListener(
    "click",
    (event) => {
      if (event.defaultPrevented || event.button !== 0) return;
      if (event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
      const link = magnetOf(event.target);
      if (!link) return;

      if (enabled) {
        event.preventDefault();
        event.stopPropagation();
        chrome.runtime.sendMessage({ type: "magnet", url: link.href }).catch(() => null);
        return;
      }

      // If status hasn't been checked yet (e.g. quick click without hover), attempt handover
      // and fall back to browser default only if SDM is unavailable.
      if (checkedAt === 0) {
        event.preventDefault();
        event.stopPropagation();
        chrome.runtime
          .sendMessage({ type: "magnet", url: link.href })
          .then((res) => {
            if (!res?.ok) location.href = link.href;
          })
          .catch(() => {
            location.href = link.href;
          });
      }
    },
    true,
  );
})();
