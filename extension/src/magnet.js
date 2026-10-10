// Opens magnet links in Supo Download Manager. When SDM is not running,
// the browser handles them as usual (it asks which program to use).

(() => {
  if (window.__sdmMagnets) return;
  window.__sdmMagnets = true;

  const SELECTOR = 'a[href^="magnet:" i]';
  const RECHECK_MS = 30_000;

  // Clicks must be decided right away, so the answer is asked beforehand:
  // when the page has magnet links and when the pointer reaches one.
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

  document.addEventListener(
    "pointerover",
    (event) => {
      if (magnetOf(event.target)) refresh();
    },
    { capture: true, passive: true },
  );

  document.addEventListener(
    "click",
    (event) => {
      if (!enabled || event.defaultPrevented || event.button !== 0) return;
      if (event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
      const link = magnetOf(event.target);
      if (!link) return;
      event.preventDefault();
      event.stopPropagation();
      chrome.runtime.sendMessage({ type: "magnet", url: link.href }).catch(() => null);
    },
    true,
  );
})();
