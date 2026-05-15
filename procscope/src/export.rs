use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::freeze::FreezeFlag;
use crate::sampler::ThreadState;
use crate::state::Snapshot;

pub struct CsvLog {
    writer: Mutex<BufWriter<File>>,
    flush_counter: std::sync::atomic::AtomicU64,
}

const FLUSH_EVERY: u64 = 64;

impl CsvLog {
    pub fn create(path: &Path) -> io::Result<Self> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        writeln!(
            writer,
            "ts_unix_us,pid,tid,name,state,cpu_pct,sys_pct,ctxsw_vol_per_s,ctxsw_invol_per_s,wchan,syscall_name,syscall_arg0,syscall_arg1,syscall_arg2,syscall_arg3,syscall_arg4,syscall_arg5,freeze_reason,stuck_ms,rchar_bps,wchar_bps"
        )?;
        writer.flush()?;
        Ok(Self {
            writer: Mutex::new(writer),
            flush_counter: std::sync::atomic::AtomicU64::new(0),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn write_thread_row(
        &mut self,
        wall: SystemTime,
        pid: i32,
        tid: i32,
        name: &str,
        state: ThreadState,
        cpu_pct: f32,
        sys_pct: f32,
        ctxsw_v: f32,
        ctxsw_iv: f32,
        wchan: &str,
        syscall_name: Option<&str>,
        syscall_args: [u64; 6],
        freeze: Option<&FreezeFlag>,
        rchar_bps: f64,
        wchar_bps: f64,
    ) -> io::Result<()> {
        let ts_us = wall.duration_since(UNIX_EPOCH).map(|d| d.as_micros() as i128).unwrap_or(0);
        let (reason, stuck_ms) = match freeze {
            Some(f) => (
                f.reason.label(),
                f.since.elapsed().as_millis() as u64,
            ),
            None => ("", 0),
        };
        let safe_name = sanitize(name);
        let mut g = self.writer.lock().unwrap();
        writeln!(
            g,
            "{ts},{pid},{tid},{name},{state},{cpu:.3},{sys:.3},{ctxv:.1},{ctxiv:.1},{wchan},{sc},{a0:x},{a1:x},{a2:x},{a3:x},{a4:x},{a5:x},{reason},{stuck},{rbps:.1},{wbps:.1}",
            ts = ts_us,
            pid = pid,
            tid = tid,
            name = safe_name,
            state = state.label(),
            cpu = cpu_pct,
            sys = sys_pct,
            ctxv = ctxsw_v,
            ctxiv = ctxsw_iv,
            wchan = sanitize(wchan),
            sc = syscall_name.unwrap_or(""),
            a0 = syscall_args[0],
            a1 = syscall_args[1],
            a2 = syscall_args[2],
            a3 = syscall_args[3],
            a4 = syscall_args[4],
            a5 = syscall_args[5],
            reason = reason,
            stuck = stuck_ms,
            rbps = rchar_bps,
            wbps = wchar_bps,
        )?;
        let n = self
            .flush_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if n % FLUSH_EVERY == 0 {
            g.flush()?;
        }
        Ok(())
    }
}

pub fn write_snapshot(snapshot: &Snapshot, dir: &Path) -> io::Result<PathBuf> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("procscope-export-{}.csv", ts));
    let file = File::create(&path)?;
    let mut w = BufWriter::new(file);
    writeln!(
        w,
        "ts_unix,pid,tid,name,state,cpu_pct,sys_pct,mean_cpu_pct,ctxsw_vol_per_s,ctxsw_invol_per_s,iowait_pct,wchan,syscall_name,freeze_reason,stuck_ms,cpu_p50,cpu_p95,cpu_p99,cpu_max"
    )?;
    let pid = snapshot.process.pid;
    let ts_now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    for t in &snapshot.threads {
        let (reason, stuck_ms) = match &t.freeze {
            Some(f) => (f.reason.label(), f.since.elapsed().as_millis() as u64),
            None => ("", 0),
        };
        writeln!(
            w,
            "{ts},{pid},{tid},{name},{state},{cpu:.3},{sys:.3},{mean:.3},{cv:.1},{civ:.1},{iow:.1},{wchan},{sc},{reason},{stuck},{p50:.3},{p95:.3},{p99:.3},{maxv:.3}",
            ts = ts_now,
            pid = pid,
            tid = t.tid,
            name = sanitize(&t.name),
            state = t.state.label(),
            cpu = t.cpu_pct,
            sys = t.sys_pct,
            mean = t.mean_cpu_pct,
            cv = t.ctxsw_vol_per_s,
            civ = t.ctxsw_invol_per_s,
            iow = t.iowait_pct,
            wchan = sanitize(&t.wchan),
            sc = t.syscall_name.unwrap_or(""),
            reason = reason,
            stuck = stuck_ms,
            p50 = t.cpu_p50,
            p95 = t.cpu_p95,
            p99 = t.cpu_p99,
            maxv = t.cpu_max,
        )?;
    }
    w.flush()?;
    Ok(path)
}

fn sanitize(s: &str) -> String {
    s.replace(',', ";").replace('\n', " ").replace('\r', " ")
}
