use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter};

static HOTKEY_THREAD_RUNNING: AtomicBool = AtomicBool::new(false);

/// Registered hotkey config stored globally for the message loop.
#[cfg(target_os = "windows")]
static PTT_KEY_CONFIG: std::sync::Mutex<Option<HotkeyConfig>> = std::sync::Mutex::new(None);
#[cfg(target_os = "windows")]
static VAD_KEY_CONFIG: std::sync::Mutex<Option<HotkeyConfig>> = std::sync::Mutex::new(None);
#[cfg(target_os = "windows")]
static HOTKEY_THREAD_ID: std::sync::Mutex<Option<u32>> = std::sync::Mutex::new(None);
#[cfg(target_os = "windows")]
static PTT_PRESSED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug)]
struct HotkeyConfig {
    key: u32,
    modifiers: u32,
}

/// Register the PTT (push-to-talk) global hotkey.
/// key: virtual key code (e.g. 0x11 = VK_CONTROL)
/// modifiers: MOD_ALT=0x0001, MOD_CONTROL=0x0002, MOD_SHIFT=0x0004, MOD_WIN=0x0008
#[tauri::command]
pub fn register_ptt_hotkey(
    app: AppHandle,
    key: u32,
    modifiers: u32,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        {
            let mut cfg = PTT_KEY_CONFIG.lock().map_err(|e| e.to_string())?;
            *cfg = Some(HotkeyConfig { key, modifiers });
        }
        restart_or_start_hotkey_thread(app)?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        log::warn!("Global hotkeys only supported on Windows (key={}, mods={})", key, modifiers);
        Ok(())
    }
}

/// Register the VAD toggle global hotkey.
#[tauri::command]
pub fn register_vad_toggle_hotkey(
    app: AppHandle,
    key: u32,
    modifiers: u32,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        {
            let mut cfg = VAD_KEY_CONFIG.lock().map_err(|e| e.to_string())?;
            *cfg = Some(HotkeyConfig { key, modifiers });
        }
        restart_or_start_hotkey_thread(app)?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        log::warn!("Global hotkeys only supported on Windows (key={}, mods={})", key, modifiers);
        Ok(())
    }
}

/// Unregister all hotkeys and stop the message loop thread.
#[tauri::command]
pub fn unregister_all_hotkeys() -> Result<(), String> {
    HOTKEY_THREAD_RUNNING.store(false, Ordering::SeqCst);
    #[cfg(target_os = "windows")]
    stop_hotkey_thread()?;

    #[cfg(target_os = "windows")]
    {
        PTT_PRESSED.store(false, Ordering::SeqCst);
        let mut ptt = PTT_KEY_CONFIG.lock().map_err(|e| e.to_string())?;
        *ptt = None;
        let mut vad = VAD_KEY_CONFIG.lock().map_err(|e| e.to_string())?;
        *vad = None;
    }
    Ok(())
}

/// Spawn the Windows hotkey message loop thread (idempotent).
#[cfg(target_os = "windows")]
fn ensure_hotkey_thread(app: AppHandle) {
    if HOTKEY_THREAD_RUNNING.swap(true, Ordering::SeqCst) {
        return; // already running
    }

    std::thread::spawn(move || {
        run_hotkey_loop(app);
    });
}

#[cfg(target_os = "windows")]
fn restart_or_start_hotkey_thread(app: AppHandle) -> Result<(), String> {
    if HOTKEY_THREAD_RUNNING.load(Ordering::SeqCst) {
        HOTKEY_THREAD_RUNNING.store(false, Ordering::SeqCst);
        stop_hotkey_thread()?;
    }

    ensure_hotkey_thread(app);
    Ok(())
}

/// Windows RegisterHotKey + GetMessage loop.
/// Emits Tauri events: ptt-press, ptt-release, vad-toggle.
#[cfg(target_os = "windows")]
fn run_hotkey_loop(app: AppHandle) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetMessageW, MSG, WM_HOTKEY, WM_QUIT,
    };

    const ID_PTT: i32 = 1;
    const ID_VAD: i32 = 2;

    let mut registered_ptt = false;
    let mut registered_vad = false;

    if let Ok(mut thread_id) = HOTKEY_THREAD_ID.lock() {
        *thread_id = Some(unsafe { GetCurrentThreadId() });
    }

    // Register hotkeys based on current config.
    let mut ptt_key_for_release = None;
    {
        if let Ok(Some(ptt)) = PTT_KEY_CONFIG.lock().as_deref().map(|c| c.as_ref().cloned()) {
            unsafe {
                let ok = RegisterHotKey(
                    HWND(std::ptr::null_mut()),
                    ID_PTT,
                    HOT_KEY_MODIFIERS(ptt.modifiers),
                    ptt.key,
                );
                registered_ptt = ok.is_ok();
                if registered_ptt {
                    ptt_key_for_release = Some(ptt.key);
                } else {
                    let _ = app.emit(
                        "hotkey-error",
                        format!("Failed to register PTT hotkey: key={}, modifiers={}", ptt.key, ptt.modifiers),
                    );
                }
            }
        }
        if let Ok(Some(vad)) = VAD_KEY_CONFIG.lock().as_deref().map(|c| c.as_ref().cloned()) {
            unsafe {
                let ok = RegisterHotKey(
                    HWND(std::ptr::null_mut()),
                    ID_VAD,
                    HOT_KEY_MODIFIERS(vad.modifiers),
                    vad.key,
                );
                registered_vad = ok.is_ok();
                if !registered_vad {
                    let _ = app.emit(
                        "hotkey-error",
                        format!("Failed to register VAD hotkey: key={}, modifiers={}", vad.key, vad.modifiers),
                    );
                }
            }
        }
    }

    loop {
        if !HOTKEY_THREAD_RUNNING.load(Ordering::SeqCst) {
            break;
        }

        let mut msg = MSG::default();
        unsafe {
            // GetMessageW blocks until a message arrives
            if GetMessageW(&mut msg, HWND(std::ptr::null_mut()), 0, 0).as_bool() {
                if msg.message == WM_QUIT {
                    break;
                }
                if msg.message == WM_HOTKEY {
                    let id = msg.wParam.0 as i32;
                    // lParam low word = modifiers, high word = vkCode
                    let vk_code = ((msg.lParam.0 >> 16) & 0xFFFF) as u32;
                    let _is_release = vk_code == 0;

                    match id {
                        ID_PTT => {
                            if !PTT_PRESSED.swap(true, Ordering::SeqCst) {
                                let _ = app.emit("ptt-press", ());
                                if let Some(key) = ptt_key_for_release {
                                    poll_ptt_release(&app, key);
                                }
                            }
                        }
                        ID_VAD => {
                            let _ = app.emit("vad-toggle", ());
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Cleanup
    unsafe {
        if registered_ptt {
            let _ = UnregisterHotKey(HWND(std::ptr::null_mut()), ID_PTT);
        }
        if registered_vad {
            let _ = UnregisterHotKey(HWND(std::ptr::null_mut()), ID_VAD);
        }
    }

    PTT_PRESSED.store(false, Ordering::SeqCst);
    if let Ok(mut thread_id) = HOTKEY_THREAD_ID.lock() {
        *thread_id = None;
    }
}

#[cfg(target_os = "windows")]
fn stop_hotkey_thread() -> Result<(), String> {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};

    let thread_id = HOTKEY_THREAD_ID
        .lock()
        .map_err(|e| e.to_string())?
        .to_owned();

    if let Some(thread_id) = thread_id {
        unsafe {
            let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    Ok(())
}

/// PTT release detection — Windows does not send WM_HOTKEY on key-up for RegisterHotKey.
/// We poll the key state instead.
#[cfg(target_os = "windows")]
pub fn poll_ptt_release(app: &AppHandle, key: u32) {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

    std::thread::spawn({
        let app = app.clone();
        move || loop {
            std::thread::sleep(std::time::Duration::from_millis(16));
            let state = unsafe { GetAsyncKeyState(key as i32) };
            // Bit 15 set = key is down
            if (state & -0x8000i16) == 0 {
                PTT_PRESSED.store(false, Ordering::SeqCst);
                let _ = app.emit("ptt-release", ());
                break;
            }
        }
    });
}
