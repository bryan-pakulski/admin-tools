use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct Capabilities {
    pub is_root: bool,
    pub syscall_readable: bool,
    pub per_thread_io_readable: bool,
    pub stack_readable: bool,
}

impl Capabilities {
    pub fn probe(pid: i32) -> Self {
        let is_root = unsafe { libc::geteuid() } == 0;

        // Pick the first task tid to probe per-thread files.
        let first_tid = first_tid(pid);

        let syscall_readable = if let Some(tid) = first_tid {
            fs::read_to_string(format!("/proc/{}/task/{}/syscall", pid, tid)).is_ok()
        } else {
            false
        };

        let per_thread_io_readable = if let Some(tid) = first_tid {
            fs::read_to_string(format!("/proc/{}/task/{}/io", pid, tid)).is_ok()
        } else {
            false
        };

        let stack_readable = if let Some(tid) = first_tid {
            Path::new(&format!("/proc/{}/task/{}/stack", pid, tid)).exists()
                && fs::read_to_string(format!("/proc/{}/task/{}/stack", pid, tid)).is_ok()
        } else {
            false
        };

        Self {
            is_root,
            syscall_readable,
            per_thread_io_readable,
            stack_readable,
        }
    }
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            is_root: false,
            syscall_readable: true,
            per_thread_io_readable: true,
            stack_readable: false,
        }
    }
}

fn first_tid(pid: i32) -> Option<i32> {
    fs::read_dir(format!("/proc/{}/task", pid))
        .ok()?
        .flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()))
        .next()
}
