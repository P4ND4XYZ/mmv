//! Observe OS URL-launch acceptance, not page-load success.
use std::io;

pub(crate) type BrowserLauncher = Box<dyn Fn(&str) -> io::Result<()>>;

#[cfg(windows)]
pub(crate) fn launch(url: &str) -> io::Result<()> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    if url.encode_utf16().any(|unit| unit == 0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "URL contains NUL",
        ));
    }
    let url: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
    let open: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    // ShellExecuteW treats the URL as a single file/URL argument; no shell
    // command or interpolated command line is constructed.
    #[allow(unsafe_code, reason = "ask the Windows shell to open the URL")]
    // SAFETY: Both UTF-16 buffers are NUL-terminated and live through the call.
    // The optional window, parameters, and working directory are null.
    let accepted = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            open.as_ptr(),
            url.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    } as isize;
    if accepted > 32 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "OS URL launch rejected ({accepted})"
        )))
    }
}

#[cfg(not(windows))]
pub(crate) fn launch(url: &str) -> io::Result<()> {
    let status = std::process::Command::new(if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    })
    .arg(url)
    .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("OS URL launch rejected"))
    }
}
