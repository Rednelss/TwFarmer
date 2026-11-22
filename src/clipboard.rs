use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    io::{self, IsTerminal, Write},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub enum CopyOutcome {
    Copied,
    TerminalRequested,
}

impl CopyOutcome {
    pub fn message(&self) -> &'static str {
        match self {
            Self::Copied => tr!(
                "Sign-in URL copied to clipboard.",
                "Ссылка входа скопирована в буфер обмена."
            ),
            Self::TerminalRequested => tr!(
                "Clipboard requested from terminal; copy the URL manually if declined.",
                "Копирование запрошено у терминала; при отказе выделите адрес вручную."
            ),
        }
    }
}

fn try_copy(program: &str, args: &[&str]) -> bool {
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let written = child.stdin.take().is_some_and(|mut input| {
        input
            .write_all(crate::auth::NATIVE_LOGIN_URL.as_bytes())
            .is_ok()
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    if written {
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                _ => break,
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    false
}

fn terminal_request() -> String {
    format!(
        "\x1b]52;c;{}\x07",
        STANDARD.encode(crate::auth::NATIVE_LOGIN_URL)
    )
}

pub fn copy_login_link() -> io::Result<CopyOutcome> {
    // Over SSH the clipboard belongs to the user's terminal, not the server desktop.
    if std::env::var_os("SSH_CONNECTION").is_none() && std::env::var_os("SSH_TTY").is_none() {
        #[cfg(target_os = "windows")]
        if try_copy("clip.exe", &[]) {
            return Ok(CopyOutcome::Copied);
        }
        #[cfg(target_os = "macos")]
        if try_copy("pbcopy", &[]) {
            return Ok(CopyOutcome::Copied);
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            if std::env::var_os("WAYLAND_DISPLAY").is_some()
                && try_copy("wl-copy", &["--type", "text/plain"])
            {
                return Ok(CopyOutcome::Copied);
            }
            if std::env::var_os("DISPLAY").is_some()
                && (try_copy("xclip", &["-selection", "clipboard"])
                    || try_copy("xsel", &["--clipboard", "--input"]))
            {
                return Ok(CopyOutcome::Copied);
            }
        }
    }
    if !io::stdout().is_terminal() {
        return Err(io::Error::other(tr!(
            "Clipboard unavailable; copy the displayed URL.",
            "Буфер недоступен; скопируйте показанный адрес."
        )));
    }
    let mut output = io::stdout().lock();
    write!(output, "{}", terminal_request())?;
    output.flush()?;
    Ok(CopyOutcome::TerminalRequested)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_clipboard_request_contains_the_complete_public_url() {
        let request = terminal_request();
        let encoded = request
            .strip_prefix("\x1b]52;c;")
            .unwrap()
            .strip_suffix('\x07')
            .unwrap();
        assert_eq!(
            STANDARD.decode(encoded).unwrap(),
            crate::auth::NATIVE_LOGIN_URL.as_bytes()
        );
        assert!(!request.contains('\n'));
    }
}
