//! Hide console windows for child processes (Windows GUI app — no cmd flashes).

#[cfg(windows)]
pub(crate) fn hide_console(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
#[inline]
pub(crate) fn hide_console(_cmd: &mut std::process::Command) {}

#[cfg(windows)]
pub(crate) fn hide_console_tokio(cmd: &mut tokio::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.as_std_mut().creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
#[inline]
pub(crate) fn hide_console_tokio(_cmd: &mut tokio::process::Command) {}

/// `CREATE_NO_WINDOW` on a console `python.exe` can break piped IPC on some Windows builds; `pythonw.exe` does not need it.
#[cfg(windows)]
pub(crate) fn hide_console_tokio_python(cmd: &mut tokio::process::Command, python_exe: &str) {
    if python_exe.to_ascii_lowercase().ends_with("pythonw.exe") {
        return;
    }
    hide_console_tokio(cmd);
}

#[cfg(not(windows))]
#[inline]
pub(crate) fn hide_console_tokio_python(_cmd: &mut tokio::process::Command, _python_exe: &str) {}
