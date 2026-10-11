//! The few texts shown by the backend (tray menu, notifications). Everything
//! else is translated in the user interface.

pub struct Texts {
    pub show: &'static str,
    pub pause_all: &'static str,
    pub resume_all: &'static str,
    pub quit: &'static str,
    pub completed: &'static str,
    pub failed: &'static str,
    pub main_queue: &'static str,
}

pub fn texts(language: &str) -> &'static Texts {
    match language {
        "vi" => &VIETNAMESE,
        _ => &ENGLISH,
    }
}

static ENGLISH: Texts = Texts {
    show: "Show Supo Download Manager",
    pause_all: "Pause all",
    resume_all: "Resume all",
    quit: "Quit",
    completed: "Download complete",
    failed: "Download failed",
    main_queue: "Main queue",
};

static VIETNAMESE: Texts = Texts {
    show: "Mở Supo Download Manager",
    pause_all: "Tạm dừng tất cả",
    resume_all: "Tiếp tục tất cả",
    quit: "Thoát",
    completed: "Đã tải xong",
    failed: "Tải không thành công",
    main_queue: "Hàng đợi chính",
};
