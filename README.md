# Supo Download Manager (SDM)

**A free and open-source download accelerator for Windows, macOS and Linux.**

SDM splits a file into segments and downloads them over several connections
at the same time. When a connection finishes its part, it takes over half of
the largest part that is still left, so every connection stays busy until the
very end. Downloads survive network errors and can be paused and resumed, even
after closing the program or restarting the computer.

> **Status: early development (v0.1).** The download engine and the `sdm`
> command line tool work today. The desktop app and the browser extension are
> next, see the [roadmap](docs/ROADMAP.md).
>
> 🇻🇳 [Đọc bằng tiếng Việt](#tiếng-việt)

## Features

- **Multi-connection downloads**: up to 32 connections per file, with
  dynamic segment splitting.
- **Pause and resume**: press `Ctrl+C` to pause, then run the same command
  again to continue. Before resuming, SDM checks the file's size, `ETag` and
  `Last-Modified` date with the server, so if the file changed, it starts
  over instead of mixing old and new data.
- **Robust**: automatic retries with exponential backoff. A connection that
  keeps failing hands its work to the others, which helps with servers that
  limit connections per user.
- **Works with any server**: if the server does not support resuming, SDM
  uses a single connection. It also handles files whose size the server does
  not report.
- **Speed limit** for all downloads together or per download.
- **Download queue**: several files in parallel (`-j`) and URL lists from a
  file (`-i`).
- **Smart file names**: taken from `Content-Disposition` or the URL, made
  safe for every operating system. Existing files are never overwritten
  unless you ask (`name (1).ext` is used instead).
- **SHA-256 verification** of finished files.
- **Custom headers, cookies, referer, user agent, HTTP and SOCKS5 proxies.**
- **One small self-contained program**: no runtime to install.

## Install

**Pre-built binaries** for Windows, macOS and Linux will be on the
[Releases](https://github.com/10s3r2604/supodownloadmanager/releases) page from
the first tagged version.

**From source** (needs [Rust](https://rustup.rs)):

```sh
cargo install --git https://github.com/10s3r2604/supodownloadmanager sdm-cli
```

or clone the repository and run `cargo build --release`. The program ends up
in `target/release/sdm` (`sdm.exe` on Windows).

## Usage

```sh
# Download into the current directory with 8 connections
sdm get https://example.com/big.iso

# 16 connections, save into ~/Downloads
sdm get -c 16 -d ~/Downloads https://example.com/big.iso

# Limit the speed to 2 MB/s
sdm get -l 2M https://example.com/big.iso

# Download every URL in urls.txt, 4 files at a time
sdm get -j 4 -i urls.txt

# Choose the file name and verify the checksum
sdm get -o ubuntu.iso --sha256 <checksum> https://example.com/ubuntu.iso

# Downloads that need a login: pass cookies, referer or other headers
sdm get --cookie "session=abc" --referer https://example.com/page https://example.com/file.zip
sdm get -H "Authorization: Bearer TOKEN" https://example.com/file.zip

# Ask the server about a file without downloading it
sdm info https://example.com/big.iso
```

Press `Ctrl+C` to pause and run the same command again to resume. Run
`sdm get --help` for all options.

## How it works

1. **Probe**: SDM asks the server for the first byte of the file
   (`Range: bytes=0-0`). The answer reveals the size, whether byte ranges
   (and therefore multiple connections and resuming) work, and the file name.
2. **Plan**: the file is divided into equal segments, one per connection.
3. **Download**: each connection fills its segment from front to back. When a
   connection finishes early, it splits the segment with the most bytes left
   and takes over the back half.
4. **Save progress**: data goes into `name.sdm-part`, and progress is saved to
   `name.sdm-state` every two seconds and whenever the download is paused.
   Only bytes that have reached the disk are recorded.
5. **Finish**: the part file is renamed to the final name and the state file
   is deleted.

## Project layout

| Path               | What it is                                                         |
| ------------------ | ------------------------------------------------------------------ |
| `crates/sdm-core`  | The download engine as a Rust library, shared by all front-ends    |
| `crates/sdm-cli`   | The `sdm` command line program                                      |
| `docs/ROADMAP.md`  | What is planned next                                                |

## Roadmap

1. ✅ Download engine and command line tool
2. 🔜 Desktop app (Tauri): download list, categories, tray icon, clipboard monitoring
3. Browser extension for Chrome, Edge and Firefox to capture downloads
4. Video grabber (HLS/DASH), scheduler, queues
5. Installers, auto-update, translations

Details in [docs/ROADMAP.md](docs/ROADMAP.md).

## Contributing

Contributions of every size are welcome: bug reports, ideas, translations,
documentation and code. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

SDM is free software under the [GNU General Public License v3.0 or later](LICENSE).
You may use, study, share and modify it. If you distribute a modified version,
it must remain free under the same license.

SDM is an independent project and is not affiliated with Internet Download
Manager or Tonec Inc.

---

## Tiếng Việt

**Supo Download Manager (SDM)** là trình tăng tốc tải xuống miễn phí, mã nguồn
mở, chạy trên Windows, macOS và Linux.

SDM chia file thành nhiều đoạn và tải chúng cùng lúc qua nhiều kết nối. Khi
một kết nối tải xong phần của mình, nó lấy một nửa của đoạn còn lại lớn nhất để
tải tiếp, nhờ vậy mọi kết nối đều bận đến tận cuối. Khi mất mạng, SDM tự kết
nối lại. Bạn có thể tạm dừng và tải tiếp, kể cả sau khi tắt chương trình hay
khởi động lại máy.

> **Trạng thái: đang phát triển (v0.1).** Engine tải và công cụ dòng lệnh `sdm`
> đã dùng được. Ứng dụng có giao diện và extension cho trình duyệt là các bước
> tiếp theo, xem [lộ trình](docs/ROADMAP.md).

### Tính năng

- **Tải đa luồng**: tối đa 32 kết nối cho mỗi file, tự chia lại các đoạn khi
  đang tải.
- **Tạm dừng / tải tiếp**: nhấn `Ctrl+C` để tạm dừng, chạy lại đúng lệnh cũ để
  tải tiếp. Trước khi tải tiếp, SDM kiểm tra lại kích thước, `ETag` và ngày sửa
  đổi với server. Nếu file trên server đã thay đổi, SDM tải lại từ đầu thay vì
  ghép lẫn dữ liệu cũ và mới.
- **Ổn định**: tự thử lại khi lỗi mạng, thời gian chờ tăng dần. Kết nối nào lỗi
  liên tục sẽ nhường phần việc cho các kết nối khác.
- **Tương thích mọi server**: dùng một kết nối nếu server không hỗ trợ tải
  tiếp. Tải được cả file mà server không báo kích thước.
- **Giới hạn tốc độ** cho tất cả các file hoặc từng file.
- **Hàng đợi**: tải nhiều file song song (`-j`), đọc danh sách link từ file
  (`-i`).
- **Đặt tên file thông minh**: lấy tên từ server hoặc từ link, an toàn trên mọi
  hệ điều hành. Không bao giờ ghi đè file có sẵn trừ khi bạn yêu cầu; SDM sẽ đặt
  tên `ten (1).ext`.
- **Kiểm tra SHA-256** sau khi tải xong.
- **Tùy chỉnh header, cookie, referer, user agent, proxy HTTP và SOCKS5.**
- **Chỉ một file chạy nhỏ gọn**, không cần cài thêm gì.

### Cài đặt

- **Bản dựng sẵn**: sẽ có ở trang
  [Releases](https://github.com/10s3r2604/supodownloadmanager/releases) từ phiên
  bản đầu tiên.
- **Từ mã nguồn** (cần cài [Rust](https://rustup.rs)):

  ```sh
  cargo install --git https://github.com/10s3r2604/supodownloadmanager sdm-cli
  ```

### Cách dùng

```sh
sdm get https://example.com/file.iso              # tải vào thư mục hiện tại
sdm get -c 16 -d ~/Downloads https://...          # 16 kết nối, lưu vào ~/Downloads
sdm get -l 2M https://...                         # giới hạn 2 MB/s
sdm get -j 4 -i danh-sach.txt                     # tải danh sách link, 4 file cùng lúc
sdm get --cookie "session=abc" https://...        # link cần đăng nhập
sdm info https://...                              # xem thông tin file, không tải
```

Nhấn `Ctrl+C` để tạm dừng, chạy lại lệnh cũ để tải tiếp. Xem mọi tùy chọn bằng
`sdm get --help`.

### Đóng góp

Mọi đóng góp đều được chào đón: báo lỗi, góp ý, dịch thuật, viết tài liệu, viết
code. Bạn có thể viết issue bằng tiếng Việt. Xem [CONTRIBUTING.md](CONTRIBUTING.md).

### Giấy phép

SDM là phần mềm tự do theo giấy phép [GNU GPL v3.0 trở lên](LICENSE). Bạn được tự
do sử dụng, chia sẻ và sửa đổi. Nếu phát hành bản sửa đổi, bản đó cũng phải là mã
nguồn mở theo cùng giấy phép.

SDM là dự án độc lập, không liên quan đến Internet Download Manager hay Tonec Inc.
