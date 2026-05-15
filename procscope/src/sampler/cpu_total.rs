use std::fs;
use std::io;

#[derive(Debug, Clone, Copy, Default)]
pub struct CpuTotal {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
    pub total: u64,
}

pub fn read() -> io::Result<CpuTotal> {
    let raw = fs::read_to_string("/proc/stat")?;
    let first = raw
        .lines()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty /proc/stat"))?;
    let mut it = first.split_whitespace();
    let tag = it.next().unwrap_or("");
    if tag != "cpu" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing cpu line",
        ));
    }
    let nums: Vec<u64> = it.filter_map(|t| t.parse().ok()).collect();
    let g = |i: usize| nums.get(i).copied().unwrap_or(0);
    let user = g(0);
    let nice = g(1);
    let system = g(2);
    let idle = g(3);
    let iowait = g(4);
    let irq = g(5);
    let softirq = g(6);
    let steal = g(7);
    let total = nums.iter().sum::<u64>();
    Ok(CpuTotal {
        user,
        nice,
        system,
        idle,
        iowait,
        irq,
        softirq,
        steal,
        total,
    })
}
