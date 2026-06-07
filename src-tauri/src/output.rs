/// Text output module — sends transcribed text to the foreground window
/// using Windows SendInput API.

#[tauri::command]
pub fn send_text(text: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        send_text_windows(&text)
    }
    #[cfg(not(target_os = "windows"))]
    {
        log::info!("send_text (stub, non-Windows): {}", text);
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn send_text_windows(text: &str) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
        KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    };

    // Build INPUT events — one KEYDOWN + KEYUP per Unicode codepoint.
    let mut inputs: Vec<INPUT> = Vec::new();

    for ch in text.chars() {
        let code = ch as u16;

        // Key down
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: Default::default(),
                    wScan: code,
                    dwFlags: KEYEVENTF_UNICODE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });

        // Key up
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: Default::default(),
                    wScan: code,
                    dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
    }

    if inputs.is_empty() {
        return Ok(());
    }

    let sent = unsafe {
        SendInput(
            inputs.as_slice(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };

    if sent != inputs.len() as u32 {
        Err(format!(
            "SendInput: sent {}/{} events",
            sent,
            inputs.len()
        ))
    } else {
        Ok(())
    }
}
