// Win32 COLORREF values (BGR), shared by the launcher and AI answer window.
#[derive(Clone, Copy)]
pub(crate) struct Theme {
    pub(crate) bg: u32,
    pub(crate) text: u32,
    pub(crate) dim: u32,
    pub(crate) muted: u32,
    pub(crate) selected: u32,
    pub(crate) divider: u32,
    pub(crate) answer_bg: u32,
    pub(crate) code: u32,
    pub(crate) link: u32,
}

pub(crate) fn theme(light: bool) -> Theme {
    if light {
        Theme {
            bg: 0x00F7F5F4,
            text: 0x00242120,
            dim: 0x005B5450,
            muted: 0x00867B74,
            selected: 0x00F3DDCC,
            divider: 0x00DEDAD6,
            answer_bg: 0x00FFFFFF,
            code: 0x00664D33,
            link: 0x00CC6600,
        }
    } else {
        Theme {
            bg: 0x001E1A18,
            text: 0x00F2EDE9,
            dim: 0x00AAA39D,
            muted: 0x00908780,
            selected: 0x0052433B,
            divider: 0x00332E2A,
            answer_bg: 0x00282320,
            code: 0x00D4B59E,
            link: 0x00F0B080,
        }
    }
}
