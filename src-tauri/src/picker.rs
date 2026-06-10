use crate::{models::ProcessWindow, state::AppState, uia::picked_payload, utils::lock_err};
use once_cell::sync::Lazy;
use std::{ptr::null_mut, sync::Mutex, thread, time::Duration};
use tauri::{AppHandle, Emitter};
use windows::Win32::{
    Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
        TranslateMessage, UnhookWindowsHookEx, HHOOK, MSG, MSLLHOOKSTRUCT, WH_MOUSE_LL,
        WM_LBUTTONDOWN, WM_QUIT,
    },
};

pub struct PickWorker {
    thread_id: u32,
    handle: thread::JoinHandle<()>,
}

struct PickContext {
    app: AppHandle,
    process: ProcessWindow,
    hook: HHOOK,
}

unsafe impl Send for PickContext {}

static PICK_CONTEXT: Lazy<Mutex<Option<PickContext>>> = Lazy::new(|| Mutex::new(None));
static PICK_THREAD_ID: Lazy<Mutex<Option<u32>>> = Lazy::new(|| Mutex::new(None));

pub fn start_pick(app: AppHandle, state: &AppState, process: ProcessWindow) -> Result<(), String> {
    cancel_pick_worker(state)?;
    let handle = thread::spawn(move || run_pick_loop(app, process));
    thread::sleep(Duration::from_millis(30));
    let thread_id = PICK_THREAD_ID.lock().map_err(lock_err)?.unwrap_or_default();
    *state.pick_worker.lock().map_err(lock_err)? = Some(PickWorker { thread_id, handle });
    tracing::info!(thread_id, "元素拾取线程已启动");
    Ok(())
}

pub fn cancel_pick_worker(state: &AppState) -> Result<(), String> {
    let worker = state.pick_worker.lock().map_err(lock_err)?.take();
    if let Some(worker) = worker {
        unsafe {
            let _ = PostThreadMessageW(worker.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        let _ = worker.handle.join();
        tracing::info!("元素拾取线程已停止");
    }
    Ok(())
}

fn run_pick_loop(app: AppHandle, process: ProcessWindow) {
    unsafe {
        let thread_id = windows::Win32::System::Threading::GetCurrentThreadId();
        if let Ok(mut id) = PICK_THREAD_ID.lock() {
            *id = Some(thread_id);
        }
        if let Ok(mut context) = PICK_CONTEXT.lock() {
            *context = Some(PickContext {
                app,
                process,
                hook: HHOOK(null_mut()),
            });
        }
        let module = GetModuleHandleW(None).unwrap_or_default();
        let hook = SetWindowsHookExW(
            WH_MOUSE_LL,
            Some(mouse_pick_proc),
            Some(HINSTANCE(module.0)),
            0,
        )
        .unwrap_or(HHOOK(null_mut()));
        if let Ok(mut context) = PICK_CONTEXT.lock() {
            if let Some(ctx) = context.as_mut() {
                ctx.hook = hook;
            }
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if !hook.0.is_null() {
            let _ = UnhookWindowsHookEx(hook);
        }
        if let Ok(mut context) = PICK_CONTEXT.lock() {
            *context = None;
        }
        if let Ok(mut id) = PICK_THREAD_ID.lock() {
            *id = None;
        }
    }
}

unsafe extern "system" fn mouse_pick_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let message = wparam.0 as u32;
        if message == WM_LBUTTONDOWN {
            let data = unsafe { *(lparam.0 as *const MSLLHOOKSTRUCT) };
            let context = PICK_CONTEXT.lock().ok().and_then(|context| {
                context
                    .as_ref()
                    .map(|ctx| (ctx.app.clone(), ctx.process.clone()))
            });

            if let Some((app, process)) = context {
                match picked_payload(&process, data.pt.x, data.pt.y) {
                    Ok(payload) => {
                        let _ = app.emit(crate::PICK_EVENT, payload);
                        return LRESULT(1);
                    }
                    Err(error) => {
                        tracing::warn!(%error, "拾取 UIA 元素失败");
                    }
                }
            }
        }
    }
    unsafe { CallNextHookEx(Some(HHOOK(null_mut())), code, wparam, lparam) }
}
