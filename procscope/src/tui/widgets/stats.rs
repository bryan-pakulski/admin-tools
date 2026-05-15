pub fn fmt_bytes(n: u64) -> String {
    const K: f64 = 1024.0;
    let f = n as f64;
    if f >= K * K * K {
        format!("{:.2}G", f / (K * K * K))
    } else if f >= K * K {
        format!("{:.1}M", f / (K * K))
    } else if f >= K {
        format!("{:.0}K", f / K)
    } else {
        format!("{}B", n)
    }
}

pub fn fmt_pct(v: f32) -> String {
    if v >= 100.0 {
        format!("{:>3.0}%", v.min(999.0))
    } else if v >= 10.0 {
        format!("{:>3.1}%", v)
    } else {
        format!("{:>3.2}%", v)
    }
}

pub fn fmt_rate(v: f32) -> String {
    if v >= 1000.0 {
        format!("{:.1}k", v / 1000.0)
    } else if v >= 100.0 {
        format!("{:.0}", v)
    } else if v >= 10.0 {
        format!("{:.1}", v)
    } else {
        format!("{:.2}", v)
    }
}

pub fn fmt_bps(v: f64) -> String {
    if v < 1.0 {
        "—".to_string()
    } else if v >= 1024.0 * 1024.0 {
        format!("{:.1}M/s", v / (1024.0 * 1024.0))
    } else if v >= 1024.0 {
        format!("{:.1}K/s", v / 1024.0)
    } else {
        format!("{:.0}B/s", v)
    }
}

pub fn fmt_duration_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let secs = ms / 1000;
        let m = secs / 60;
        let s = secs % 60;
        format!("{}m{:02}s", m, s)
    }
}
