# Changelog

## 0.6.1

- Browser-captured downloads open a small, separate window with file name, size,
  destination and Download now / Download later choices.
- X feed videos resolve the selected post and clip. Choose quality, frame rate,
  format and estimated size directly in the browser menu.
- Video discovery uses short polling requests that survive extension worker
  restarts. Closing the menu cancels its request; discovery has bounded active
  work, queue length and timeouts.
- GitHub EXE/ZIP capture handles redirects and late file-name updates, while
  preserving browser cookies and referrer for the download.
- Choose SDM's temporary folder in Settings. Downloads keep their own temporary
  subfolder, retain paused data and clean completed or removed data, including
  when the destination is on another drive.
- Settings are validated before saving or changing the running configuration.
  Save failures stay visible inside Settings. Saved data has a last-good backup;
  recovery preserves damaged originals and reports the affected paths.
- New downloads are saved before starting. Resume validates complete segment
  coverage and partial-file length, syncs bytes before saved progress, and uses
  HTTP validators to avoid mixing changed server content.
- A completion journal reconciles a downloaded file with history after an
  interrupted final save. Completion cleanup waits for history to be saved.

The browser extension is version 0.6.3. Its video menu requires SDM 0.6.1 or newer.
Existing extension 0.6.2 clients can still use the desktop bridge's legacy API.
