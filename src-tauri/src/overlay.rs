use crate::{models::RectDto, utils::win_err};
use once_cell::sync::OnceCell;
use windows::{
    core::w,
    Win32::{
        Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::CreateSolidBrush,
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, RegisterClassW,
            SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow, CS_HREDRAW,
            CS_VREDRAW, GWL_EXSTYLE, HWND_TOPMOST, LWA_ALPHA, SWP_NOACTIVATE, SWP_NOOWNERZORDER,
            SW_HIDE, SW_SHOWNOACTIVATE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
        },
    },
};

#[derive(Default)]
pub struct Overlay {
    hwnds: Vec<HWND>,
}

unsafe impl Send for Overlay {}

impl Overlay {
    pub fn show(&mut self, rect: RectDto) -> Result<(), String> {
        if rect.width <= 0 || rect.height <= 0 {
            self.hide();
            return Ok(());
        }

        let size = 3;
        let color = 0x00ff8a00isize;
        let pieces = [
            (rect.x, rect.y, rect.width, size),
            (rect.x, rect.y + rect.height - size, rect.width, size),
            (rect.x, rect.y, size, rect.height),
            (rect.x + rect.width - size, rect.y, size, rect.height),
        ];

        if self.hwnds.len() != 4 {
            self.hide();
            for (x, y, w, h) in pieces {
                let hwnd = create_overlay_window(x, y, w, h, color)?;
                self.hwnds.push(hwnd);
            }
        } else {
            for (i, (x, y, w, h)) in pieces.iter().enumerate() {
                unsafe {
                    let _ = SetWindowPos(
                        self.hwnds[i],
                        Some(HWND_TOPMOST),
                        *x,
                        *y,
                        *w,
                        *h,
                        SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                    );
                    let _ = ShowWindow(self.hwnds[i], SW_SHOWNOACTIVATE);
                }
            }
        }
        Ok(())
    }

    pub fn hide(&mut self) {
        for hwnd in self.hwnds.drain(..) {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}

fn create_overlay_window(x: i32, y: i32, w: i32, h: i32, color: isize) -> Result<HWND, String> {
    unsafe {
        register_overlay_class()?;
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED,
            w!("PenSwitcherOverlay"),
            w!(""),
            WS_POPUP,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(HINSTANCE(GetModuleHandleW(None).unwrap_or_default().0)),
            None,
        )
        .map_err(win_err)?;
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            style | (WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE).0 as isize,
        );
        let _ = SetLayeredWindowAttributes(
            hwnd,
            windows::Win32::Foundation::COLORREF(color as u32),
            210,
            LWA_ALPHA,
        );
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            w,
            h,
            SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        Ok(hwnd)
    }
}

unsafe extern "system" fn overlay_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn register_overlay_class() -> Result<(), String> {
    static REGISTERED: OnceCell<()> = OnceCell::new();
    REGISTERED.get_or_try_init(|| unsafe {
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(overlay_proc),
            hInstance: HINSTANCE(GetModuleHandleW(None).unwrap_or_default().0),
            lpszClassName: w!("PenSwitcherOverlay"),
            hbrBackground: CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x00ff8a00)),
            ..Default::default()
        };
        let atom = RegisterClassW(&class);
        if atom == 0 {
            return Err(format!("注册覆盖框窗口类失败: {:?}", GetLastError()));
        }
        Ok(())
    })?;
    Ok(())
}
