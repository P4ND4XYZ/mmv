//! Windows closes all handles when the viewer exits, including during generation.
//! A kill-on-close job prevents Node/Chromium children from outliving the viewer.

use std::{
    ffi::c_void,
    mem::{size_of, zeroed},
    os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle},
    process::Child,
};

#[repr(C)]
struct BasicLimits {
    process_time: i64,
    job_time: i64,
    flags: u32,
    min_working_set: usize,
    max_working_set: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

#[repr(C)]
struct ExtendedLimits {
    basic: BasicLimits,
    io_counters: [u64; 6],
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory: usize,
    peak_job_memory: usize,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
    fn SetInformationJobObject(job: *mut c_void, class: i32, info: *const c_void, size: u32)
    -> i32;
    fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
}

pub struct ChildJob {
    _handle: OwnedHandle,
}

impl ChildJob {
    pub fn attach(child: &Child) -> Result<Self, String> {
        // SAFETY: unnamed job, default security attributes, valid ABI structures,
        // and borrowed process/job handles that remain open throughout each call.
        unsafe {
            let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if raw.is_null() {
                return Err(format!(
                    "creating Mermaid job: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let handle = OwnedHandle::from_raw_handle(raw);
            let mut limits: ExtendedLimits = zeroed();
            limits.basic.flags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            if SetInformationJobObject(
                handle.as_raw_handle(),
                9, // JobObjectExtendedLimitInformation
                &limits as *const _ as *const c_void,
                size_of::<ExtendedLimits>() as u32,
            ) == 0
            {
                return Err(format!(
                    "configuring Mermaid job: {}",
                    std::io::Error::last_os_error()
                ));
            }
            if AssignProcessToJobObject(handle.as_raw_handle(), child.as_raw_handle()) == 0 {
                return Err(format!(
                    "assigning Mermaid job: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(Self { _handle: handle })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::windows::process::CommandExt as _,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    #[test]
    fn closing_the_job_terminates_a_running_process() {
        let mut child = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 60",
            ])
            .creation_flags(0x0800_0000)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let job = ChildJob::attach(&child);
        if job.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        drop(job.unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        let terminated = loop {
            if child.try_wait().unwrap().is_some() {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let _ = child.kill();
        let _ = child.wait();
        assert!(terminated, "closing the job did not terminate its process");
    }
}
