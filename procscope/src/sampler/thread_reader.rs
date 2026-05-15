use std::fs;
use std::io::{self, ErrorKind};

use crate::sampler::proc_reader::{parse_io, parse_stat, parse_status};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum ThreadState {
    Running,
    Sleeping,
    Disk,
    Stopped,
    Tracing,
    Zombie,
    Dead,
    Idle,
    Unknown,
}

impl ThreadState {
    pub fn from_char(c: char) -> Self {
        match c {
            'R' => Self::Running,
            'S' => Self::Sleeping,
            'D' => Self::Disk,
            'T' => Self::Stopped,
            't' => Self::Tracing,
            'Z' => Self::Zombie,
            'X' | 'x' => Self::Dead,
            'I' => Self::Idle,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "R",
            Self::Sleeping => "S",
            Self::Disk => "D",
            Self::Stopped => "T",
            Self::Tracing => "t",
            Self::Zombie => "Z",
            Self::Dead => "X",
            Self::Idle => "I",
            Self::Unknown => "?",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SyscallInfo {
    pub nr: u64,
    pub args: [u64; 6],
    pub sp: u64,
    pub pc: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SchedStat {
    /// Time spent running on CPU. Nanoseconds on modern kernels despite the historical name.
    pub run_ns: u64,
    /// Time spent waiting on a runqueue. Nanoseconds.
    pub wait_ns: u64,
    pub pcount: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ThreadIo {
    pub rchar: u64,
    pub wchar: u64,
}

#[derive(Debug, Clone)]
pub struct ThreadSample {
    pub tid: i32,
    pub name: String,
    pub state: ThreadState,
    pub utime: u64,
    pub stime: u64,
    pub starttime_ticks: u64,
    pub processor: i32,
    pub vol_ctxsw: u64,
    pub invol_ctxsw: u64,
    pub wchan: String,
    pub syscall: Option<SyscallInfo>,
    pub schedstat: SchedStat,
    pub io: Option<ThreadIo>,
}

pub fn list_tids(pid: i32) -> io::Result<Vec<i32>> {
    let dir = fs::read_dir(format!("/proc/{}/task", pid))?;
    let mut out = Vec::new();
    for entry in dir.flatten() {
        if let Some(s) = entry.file_name().to_str() {
            if let Ok(tid) = s.parse::<i32>() {
                out.push(tid);
            }
        }
    }
    Ok(out)
}

pub fn read_thread(pid: i32, tid: i32) -> io::Result<ThreadSample> {
    let base = format!("/proc/{}/task/{}", pid, tid);

    let stat_raw = fs::read_to_string(format!("{}/stat", base))?;
    let stat = parse_stat(&stat_raw)
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "task/stat"))?;

    let status_raw = fs::read_to_string(format!("{}/status", base)).unwrap_or_default();
    let status = parse_status(&status_raw);
    let name = if !status.name.is_empty() {
        status.name
    } else {
        // /proc/<pid>/task/<tid>/comm
        fs::read_to_string(format!("{}/comm", base))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| stat.comm.clone())
    };

    let wchan = read_wchan(&base);
    let syscall = read_syscall(&base);
    let schedstat = read_schedstat(&base);
    let io = read_thread_io(&base);

    Ok(ThreadSample {
        tid,
        name,
        state: stat.state,
        utime: stat.utime,
        stime: stat.stime,
        starttime_ticks: stat.starttime,
        processor: stat.processor,
        vol_ctxsw: status.vol_ctxsw,
        invol_ctxsw: status.invol_ctxsw,
        wchan,
        syscall,
        schedstat,
        io,
    })
}

fn read_wchan(base: &str) -> String {
    match fs::read_to_string(format!("{}/wchan", base)) {
        Ok(s) => {
            let t = s.trim();
            if t.is_empty() || t == "0" {
                String::new()
            } else if t.chars().all(|c| c.is_ascii_hexdigit() || c == 'x') && !t.contains('_') {
                format!("0x{}", t.trim_start_matches("0x"))
            } else {
                t.to_string()
            }
        }
        Err(_) => String::new(),
    }
}

fn read_syscall(base: &str) -> Option<SyscallInfo> {
    let raw = fs::read_to_string(format!("{}/syscall", base)).ok()?;
    let line = raw.trim();
    if line == "running" || line.is_empty() {
        return None;
    }
    let mut it = line.split_whitespace();
    let nr_token = it.next()?;
    let nr: i64 = nr_token.parse().ok()?;
    if nr < 0 {
        return None;
    }
    let mut args = [0u64; 6];
    for slot in args.iter_mut() {
        if let Some(tok) = it.next() {
            *slot = parse_u64_hex(tok).unwrap_or(0);
        }
    }
    let sp = it.next().and_then(parse_u64_hex).unwrap_or(0);
    let pc = it.next().and_then(parse_u64_hex).unwrap_or(0);
    Some(SyscallInfo {
        nr: nr as u64,
        args,
        sp,
        pc,
    })
}

fn parse_u64_hex(s: &str) -> Option<u64> {
    let s = s.trim_start_matches("0x");
    u64::from_str_radix(s, 16).ok()
}

fn read_schedstat(base: &str) -> SchedStat {
    let raw = match fs::read_to_string(format!("{}/schedstat", base)) {
        Ok(s) => s,
        Err(_) => return SchedStat::default(),
    };
    let mut it = raw.split_whitespace();
    let run_ns = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let wait_ns = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let pcount = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    SchedStat {
        run_ns,
        wait_ns,
        pcount,
    }
}

fn read_thread_io(base: &str) -> Option<ThreadIo> {
    let raw = fs::read_to_string(format!("{}/io", base)).ok()?;
    let io = parse_io(&raw);
    Some(ThreadIo {
        rchar: io.rchar,
        wchar: io.wchar,
    })
}
