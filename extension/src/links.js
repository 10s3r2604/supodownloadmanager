// Collects the links of a page (or of the selected part) for "Download all
// links with SDM". The extension asks for them; nothing runs otherwise.

/** The http(s) links and media of `root`, without duplicates. */
function sdmPageLinks(root, selection, baseUrl, limit = 5000) {
  const seen = new Set();
  const links = [];
  const add = (href, text) => {
    if (!href || links.length >= limit) return;
    let url;
    try {
      url = new URL(href, baseUrl);
    } catch {
      return;
    }
    if (url.protocol !== "http:" && url.protocol !== "https:") return;
    url.hash = "";
    if (seen.has(url.href)) return;
    seen.add(url.href);
    const label = (text || "").replace(/\s+/g, " ").trim().slice(0, 200);
    links.push(label ? { url: url.href, text: label } : { url: url.href });
  };
  const chosen = (element) => !selection || selection.containsNode(element, true);
  for (const link of root.querySelectorAll("a[href], area[href]")) {
    if (chosen(link)) add(link.href || link.getAttribute("href"), link.textContent || link.title);
  }
  for (const media of root.querySelectorAll("img[src], video[src], audio[src], source[src]")) {
    if (chosen(media)) add(media.currentSrc || media.src || media.getAttribute("src"), media.alt || media.title);
  }
  return links;
}

(() => {
  if (window.__sdmLinks || typeof chrome === "undefined" || !chrome.runtime?.onMessage) return;
  window.__sdmLinks = true;
  chrome.runtime.onMessage.addListener((message, _sender, reply) => {
    if (message?.type !== "collect-links") return false;
    const selection = message.selection ? window.getSelection() : null;
    reply({
      pageUrl: location.href,
      title: document.title,
      links: sdmPageLinks(document, selection && !selection.isCollapsed ? selection : null, document.baseURI),
    });
    return false;
  });
})();
