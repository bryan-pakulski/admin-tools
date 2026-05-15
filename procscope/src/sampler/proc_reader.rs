use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;

use crate::sampler::thread_reader::ThreadState;

#[derive(Debug, Clone)]
pub struct ProcessSample {
    pub pid: i32,
    pub comm: String,
    pub cmdline: String,
    pub state: ThreadState,
    pub num_threads: u32,
    pub utime: u64,
    pub stime: u64,
    pub vm_rss_kb: u64,
    pub vm_size_kb: u64,
    pub vm_peak_kb: u64,
    pub fd_count: u32,
    pub socket_count: u32,
    pub vol_ctxsw: u64,
    pub invol_ctxsw: u64,
    pub rchar: u64,
    pub wchar: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub starttime_ticks: u64,
}

pub fn read_process(pid: i32) -> io::Result<ProcessSample> {
    let stat_path = format!("/proc/{}/stat", pid);
    let stat = fs::read_to_string(&stat_path)?;
    let stat = parse_stat(&stat).ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "stat"))?;

    let status_path = format!("/proc/{}/status", pid);
    let status_raw = fs::read_to_string(&status_path).unwrap_or_default();
    let status = parse_status(&status_raw);

    let cmdline = read_cmdline(pid);
    let comm = fs::read_to_string(format!("/proc/{}/comm", pid))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| stat.comm.clone());

    let io_path = format!("/proc/{}/io", pid);
    let io_raw = fs::read_to_string(&io_path).unwrap_or_default();
    let pio = parse_io(&io_raw);

    let (fd_count, socket_count) = count_fds_and_sockets(pid);

    Ok(ProcessSample {
        pid,
        comm,
        cmdline,
        state: stat.state,
        num_threads: stat.num_threads,
        utime: stat.utime,
        stime: stat.stime,
        vm_rss_kb: status.vm_rss_kb,
        vm_size_kb: status.vm_size_kb,
        vm_peak_kb: status.vm_peak_kb,
        fd_count,
        socket_count,
        vol_ctxsw: status.vol_ctxsw,
        invol_ctxsw: status.invol_ctxsw,
        rchar: pio.rchar,
        wchar: pio.wchar,
        read_bytes: pio.read_bytes,
        write_bytes: pio.write_bytes,
        starttime_ticks: stat.starttime,
    })
}

pub fn read_starttime(pid: i32) -> io::Result<u64> {
    let stat = fs::read_to_string(format!("/proc/{}/stat", pid))?;
    parse_stat(&stat)
        .map(|s| s.starttime)
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "stat"))
}

pub(crate) struct StatFields {
    pub comm: String,
    pub state: ThreadState,
    pub utime: u64,
    pub stime: u64,
    pub num_threads: u32,
    pub processor: i32,
    pub starttime: u64,
}

pub(crate) fn parse_stat(s: &str) -> Option<StatFields> {
    // Format: pid (comm with spaces) state ...
    let lparen = s.find('(')?;
    let rparen = s.rfind(')')?;
    let comm = s[lparen + 1..rparen].to_string();
    let rest = s[rparen + 1..].trim();
    let fields: Vec<&str> = rest.split_whitespace().collect();
    if fields.len() < 22 {
        return None;
    }
    // After comm: state(0) ppid(1) pgrp(2) session(3) tty_nr(4) tpgid(5) flags(6)
    // minflt(7) cminflt(8) majflt(9) cmajflt(10) utime(11) stime(12) cutime(13) cstime(14)
    // priority(15) nice(16) num_threads(17) itrealvalue(18) starttime(19) ...
    // processor is field 36 in /proc/<pid>/stat (0-indexed in "after comm" array: 36-2=34? Let's enumerate)
    // /proc/<pid>/stat fields starting from "state" is index 2 of the man page (1-indexed).
    // After comm, the array index 0 = state.
    let state = ThreadState::from_char(fields[0].chars().next().unwrap_or('?'));
    let utime: u64 = fields[11].parse().ok()?;
    let stime: u64 = fields[12].parse().ok()?;
    let num_threads: u32 = fields[17].parse().ok()?;
    let starttime: u64 = fields[19].parse().ok()?;
    // processor: man proc /proc/[pid]/stat field 39 (1-indexed). After comm offset: 39 - 3 = 36.
    let processor: i32 = fields.get(36).and_then(|s| s.parse().ok()).unwrap_or(-1);

    Some(StatFields {
        comm,
        state,
        utime,
        stime,
        num_threads,
        processor,
        starttime,
    })
}

#[derive(Debug, Default)]
pub(crate) struct StatusFields {
    pub vm_rss_kb: u64,
    pub vm_size_kb: u64,
    pub vm_peak_kb: u64,
    pub vol_ctxsw: u64,
    pub invol_ctxsw: u64,
    pub name: String,
}

pub(crate) fn parse_status(s: &str) -> StatusFields {
    let mut out = StatusFields::default();
    for line in s.lines() {
        let Some((key, rest)) = line.split_once(':') else { continue };
        let val = rest.trim();
        match key {
            "Name" => out.name = val.to_string(),
            "VmRSS" => out.vm_rss_kb = parse_kb(val),
            "VmSize" => out.vm_size_kb = parse_kb(val),
            "VmPeak" => out.vm_peak_kb = parse_kb(val),
            "voluntary_ctxt_switches" => out.vol_ctxsw = val.parse().unwrap_or(0),
            "nonvoluntary_ctxt_switches" => out.invol_ctxsw = val.parse().unwrap_or(0),
            _ => {}
        }
    }
    out
}

fn parse_kb(v: &str) -> u64 {
    v.split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

#[derive(Debug, Default)]
pub(crate) struct IoFields {
    pub rchar: u64,
    pub wchar: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
}

pub(crate) fn parse_io(s: &str) -> IoFields {
    let mut out = IoFields::default();
    for line in s.lines() {
        let Some((key, val)) = line.split_once(':') else { continue };
        let v: u64 = val.trim().parse().unwrap_or(0);
        match key {
            "rchar" => out.rchar = v,
            "wchar" => out.wchar = v,
            "read_bytes" => out.read_bytes = v,
            "write_bytes" => out.write_bytes = v,
            _ => {}
        }
    }
    out
}

fn read_cmdline(pid: i32) -> String {
    let path = format!("/proc/{}/cmdline", pid);
    match fs::read(&path) {
        Ok(bytes) => bytes
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect::<Vec<_>>()
            .join(" "),
        Err(_) => String::new(),
    }
}

fn count_fds_and_sockets(pid: i32) -> (u32, u32) {
    let path = format!("/proc/{}/fd", pid);
    let Ok(dir) = fs::read_dir(&path) else {
        return (0, 0);
    };
    let mut fds = 0u32;
    let mut socks = 0u32;
    for entry in dir.flatten() {
        fds += 1;
        if let Ok(link) = fs::read_link(entry.path()) {
            let s = link.to_string_lossy();
            if s.starts_with("socket:") {
                socks += 1;
            }
        }
    }
    (fds, socks)
}

pub fn pid_exists(pid: i32) -> bool {
    Path::new(&format!("/proc/{}/stat", pid)).exists()
}
