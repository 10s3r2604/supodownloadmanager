# Code signing policy

## Status and scope

Windows signing through SignPath is prepared but **not activated**. The
published v0.5.0 installers are unsigned. SignPath Foundation must accept the
project and configure production signing before a signed release can be made.

The prepared pipeline signs the SDM desktop executable, the Windows NSIS
installer and the Windows MSI installer. It does not sign the standalone CLI,
the NSIS-generated uninstaller, macOS packages or third-party executables.
Bundled yt-dlp, Deno and FFmpeg retain their upstream signatures, if present.

The intended arrangement, subject to approval, is:

> Free code signing provided by [SignPath.io](https://signpath.io), certificate
> by [SignPath Foundation](https://signpath.org).

The free foundation certificate normally identifies **SignPath Foundation**
as the publisher. It does not give SDM a certificate in its own company name.
Acceptance is discretionary; an open-source license alone does not guarantee
approval.

## Team roles

- Author / maintainer: [10s3r2604](https://github.com/10s3r2604).
- Reviewer: [10s3r2604](https://github.com/10s3r2604).
- Release signing approver: [10s3r2604](https://github.com/10s3r2604).

These are the proposed roles for the repository owner. Confirm them during
SignPath onboarding. Non-maintainer contributions must be reviewed. Use
multi-factor authentication for GitHub and SignPath, and manually approve each
production signing request in SignPath. The workflow makes two requests per
release: one for the application and one for the installers.

## Build and verification

Signed release builds must come from a commit on `main` with a stable version
tag matching the app configuration. The application is signed before packaging.
Tauri then bundles it with binary patching disabled so its signature remains
valid. The installers are signed afterwards.

Before publication, the workflow verifies Windows Authenticode trust, the
approved signing certificate's thumbprint, a signing timestamp, the expected
file names and the application/NSIS product name and version. It also checks
that packaging did not change the signed application. When signing is enabled,
missing configuration or failed verification stops the release; there is no
fallback to unsigned Windows installers.

Release assets for an already published Windows version are not replaced.
Use a new version for the first signed release. Only verified installer
artifacts are selected for publication; intermediate unsigned artifacts are
kept separately for seven days.

The SignPath artifact configurations are in [`.signpath`](../.signpath).
They select SDM's executable and installer files explicitly, rather than
signing every bundled `.exe`.

## Privacy

See [Privacy policy](PRIVACY.md). SDM has no analytics or user accounts. It
contacts file/video servers to perform downloads, official tool release
services to install/update video tools, and BitTorrent peers and trackers for
torrents. Torrent participants can see each other's IP addresses.

For signing, GitHub Actions sends the application and installers to SignPath;
this happens during the release build, not on users' computers.

## Activate signing

1. Apply through [SignPath Foundation](https://signpath.org/apply). Supply the
   [repository](https://github.com/10s3r2604/supodownloadmanager), the existing
   [release](https://github.com/10s3r2604/supodownloadmanager/releases/tag/v0.5.0),
   the GPL-3.0-or-later license, this policy and the privacy policy. Explain
   that installers bundle upstream yt-dlp, Deno and FFmpeg, which will not be
   re-signed as SDM binaries. Enter contact/identity details directly in the
   application form.
2. After acceptance, connect the GitHub repository using SignPath's
   [trusted build integration](https://docs.signpath.io/trusted-build-systems/github).
   Restrict production signing to this repository's release workflow and
   reviewed release commits/tags. Configure the owner as manual approver.
3. Import [windows-app.xml](../.signpath/windows-app.xml) as artifact
   configuration **`windows-app`** and
   [windows-installers.xml](../.signpath/windows-installers.xml) as
   **`windows-installers`**. Use a **`release-signing`** policy with the approved
   production certificate, SHA-256 signatures and trusted timestamps. Validate
   both configurations in SignPath's editor; service-side validation requires
   the SignPath account.
4. In **GitHub → Settings → Secrets and variables → Actions**, set:

   | Type | Name | Value |
   | --- | --- | --- |
   | Secret | `SIGNPATH_API_TOKEN` | SignPath token authorized for the project and release policy |
   | Variable | `SIGNPATH_ORGANIZATION_ID` | Organization ID supplied by SignPath |
   | Variable | `SIGNPATH_PROJECT_SLUG` | The approved project's slug |
   | Variable | `SIGNPATH_CERTIFICATE_THUMBPRINT` | Approved production certificate's SHA-1 thumbprint, 40 hex characters without spaces |
   | Variable | `WINDOWS_SIGNING_ENABLED` | `true`, set **last**, once the service and other values are ready |

   A certificate thumbprint is an identifier, not the signature's digest
   algorithm. The signatures use SHA-256. When SignPath rotates the certificate,
   verify its replacement with the service and update the pinned thumbprint.
   Keep the API token in GitHub Secrets; do not put it in commits or chat.
5. Merge the workflow changes, update this page's status and credit after
   acceptance, bump the app/workspace/npm versions, and release a new tag such
   as `v0.5.1`. Run **Release** from `main` or push that tag on `main`.
6. Approve the application signing request, then the installer signing request
   in SignPath. Each request waits up to one hour for completion. Check the
   resulting `.exe` and `.msi` signatures and publisher on Windows before
   distributing the new release.

With `WINDOWS_SIGNING_ENABLED` unset or `false`, the existing unsigned release
process remains available and its release notes say the installers are unsigned.
Do not enable signing until production credentials are ready.

## SmartScreen

A trusted signature replaces "Unknown publisher" with the certificate's
publisher and detects modifications to the signed file. It does **not**
guarantee the absence of Microsoft Defender SmartScreen warnings. Newly signed
applications can still need reputation to build. See Microsoft's
[SmartScreen reputation guidance](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation).

## Tiếng Việt

Quy trình ký Windows đã được chuẩn bị, chưa kích hoạt. Anh đăng ký SignPath
Foundation, chờ duyệt dự án rồi cấu hình các giá trị trong bảng trên ở GitHub.
Không cần mua chứng thư nếu dự án được chương trình miễn phí chấp nhận. Chữ ký
thường hiện nhà phát hành **SignPath Foundation**. Bản v0.5.0 vẫn chưa ký;
sẽ phát hành một phiên bản mới sau khi kích hoạt. SmartScreen vẫn có thể cảnh
báo lúc ứng dụng chưa đủ uy tín.
