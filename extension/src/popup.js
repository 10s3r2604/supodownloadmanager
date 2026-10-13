const DEFAULT_PREFS = { capture: true, videoButton: true, magnets: true, port: 17360, minSizeMb: 0 };
const ALL_SITES = { origins: ["<all_urls>"] };

const $ = (id) => document.getElementById(id);
const message = (key) => chrome.i18n.getMessage(key) || key;

for (const element of document.querySelectorAll("[data-i18n]")) {
  element.textContent = message(element.dataset.i18n);
}

let statusRequest = 0;
async function showStatus() {
  const request = ++statusRequest;
  $("retryStatus").disabled = true;
  $("status").textContent = message("connectionChecking");
  const connection = await chrome.runtime.sendMessage({ type: "connection-status" }).catch(() => null);
  if (request !== statusRequest) return;
  $("retryStatus").disabled = false;
  const status = connection?.status;
  const line = $("status");
  line.classList.remove("checking", "ok", "off");
  if (!status) {
    line.textContent = message(connection?.error === "appNotRunning" || connection?.error === "notConfigured" ? "statusOffline" : "statusConnectionError");
    line.classList.add("off");
  } else if (!status.enabled) {
    line.textContent = message("statusDisabled");
    line.classList.add("off");
  } else {
    line.textContent = `${message("statusConnected")} · v${status.version}`;
    line.classList.add("ok");
  }
  $("openApp").hidden = Boolean(status) || !connection?.canLaunch;
  $("transport").textContent = message(connection?.transport === "native" ? "connectionNative" : connection?.transport === "compatibility" ? "connectionCompatibility" : "connectionUnavailable");
  $("connectionError").hidden = !connection?.error;
  $("connectionError").textContent = connection?.error || "";
}

$("retryStatus").addEventListener("click", () => void showStatus());
$("openApp").addEventListener("click", async () => {
  $("openApp").disabled = true;
  try {
    const answer = await chrome.runtime.sendMessage({type:"open-app"});
    if (answer?.ok) {
      for (let attempt = 0; attempt < 8; attempt++) {
        await new Promise(resolve => setTimeout(resolve, 500));
        await showStatus();
        if ($("status").classList.contains("ok")) break;
      }
    } else await showStatus();
  } catch { await showStatus(); }
  finally { $("openApp").disabled = false; }
});

async function showPermission() {
  const granted = await chrome.permissions.contains(ALL_SITES).catch(() => true);
  $("permission").hidden = granted;
}

async function load() {
  const prefs = await chrome.storage.local.get(DEFAULT_PREFS);
  $("capture").checked = prefs.capture;
  $("videoButton").checked = prefs.videoButton;
  $("magnets").checked = prefs.magnets;
  $("minSize").value = prefs.minSizeMb;
  $("port").value = prefs.port;
}

$("capture").addEventListener("change", (event) => {
  chrome.storage.local.set({ capture: event.target.checked });
});

$("videoButton").addEventListener("change", (event) => {
  chrome.storage.local.set({ videoButton: event.target.checked });
});

$("magnets").addEventListener("change", (event) => {
  chrome.storage.local.set({ magnets: event.target.checked });
});

$("minSize").addEventListener("change", (event) => {
  const value = Math.max(0, Math.round(Number(event.target.value) || 0));
  event.target.value = value;
  chrome.storage.local.set({ minSizeMb: value });
});

$("port").addEventListener("change", async (event) => {
  let value = Math.round(Number(event.target.value));
  if (!(value >= 1024 && value <= 65535)) value = DEFAULT_PREFS.port;
  event.target.value = value;
  await chrome.storage.local.set({ port: value });
  showStatus();
});

$("grant").addEventListener("click", async () => {
  await chrome.permissions.request(ALL_SITES).catch(() => false);
  showPermission();
});

$("allLinks").addEventListener("click", async () => {
  const button = $("allLinks");
  const result = $("linksResult");
  button.disabled = true;
  const answer = await chrome.runtime.sendMessage({ type: "page-links" }).catch(() => null);
  button.disabled = false;
  result.hidden = false;
  result.classList.toggle("error", !answer?.ok);
  result.textContent = answer?.ok
    ? chrome.i18n.getMessage("linksSent", [String(answer.count)])
    : answer?.error || message("statusOffline");
  if (answer?.ok) setTimeout(() => window.close(), 900);
});

load();
showStatus();
showPermission();
