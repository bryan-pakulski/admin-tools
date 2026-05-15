//! A contrived target for procscope verification.
//!
//! Spawns named threads exhibiting distinct behavior:
//!   - `net-recv-1`  : blocking recv() on a TCP socket no one writes to
//!                     → S + sk_wait_data / tcp_recvmsg forever
//!   - `net-recv-2`  : recv() with a short read_timeout in a loop
//!                     → high voluntary ctxsw, never flagged
//!   - `worker-busy` : tight CPU loop on prime sieve
//!                     → R, ~100% on one core
//!   - `gc-sleeper`  : sleep(50ms) loop
//!                     → S + hrtimer_nanosleep, low cpu, healthy
//!   - `futex-wait`  : blocked on a Mutex held by another thread
//!                     → S + futex_wait_queue, vol_ctxsw stops growing → NoCtxSwitch
//!
//! Run alongside procscope:
//!   $ cargo run --release --example stuck-recv
//!   $ procscope --pid $(pgrep -n stuck-recv) --interval-ms 100

use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn main() -> std::io::Result<()> {
    // Bind a local listener so a peer can connect — but we never write to the connection.
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    eprintln!("listening on 127.0.0.1:{port}; pid {}", std::process::id());

    // Background acceptor that drops connections (so they stay open).
    let _accept = thread::Builder::new()
        .name("acceptor".into())
        .spawn(move || {
            for incoming in listener.incoming() {
                if let Ok(stream) = incoming {
                    // Hold the socket so the client side stays connected.
                    Box::leak(Box::new(stream));
                }
            }
        })?;

    let net_recv_1 = thread::Builder::new()
        .name("net-recv-1".into())
        .spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
            let mut buf = [0u8; 64];
            // Blocking recv that will never receive — perfect freeze.
            let _ = s.read(&mut buf);
        })?;

    let net_recv_2 = thread::Builder::new()
        .name("net-recv-2".into())
        .spawn(move || {
            let s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
            s.set_read_timeout(Some(Duration::from_millis(100))).ok();
            let mut buf = [0u8; 64];
            loop {
                // Each call wakes after 100ms with WouldBlock — high voluntary ctxsw.
                let _ = (&s).read(&mut buf);
            }
        })?;

    let worker_busy = thread::Builder::new()
        .name("worker-busy".into())
        .spawn(|| loop {
            // Tight CPU loop; prime sieve to defeat constant folding.
            let mut x: u64 = 1;
            for n in 2u64..200_000 {
                let mut is_prime = true;
                let mut i = 2u64;
                while i * i <= n {
                    if n % i == 0 {
                        is_prime = false;
                        break;
                    }
                    i += 1;
                }
                if is_prime {
                    x = x.wrapping_add(n);
                }
            }
            std::hint::black_box(x);
        })?;

    let gc_sleeper = thread::Builder::new()
        .name("gc-sleeper".into())
        .spawn(|| loop {
            thread::sleep(Duration::from_millis(50));
        })?;

    // futex-wait: one thread holds the mutex for 10s at a time, blocking the other.
    let lock = Arc::new(Mutex::new(()));
    let holder_lock = lock.clone();
    let _holder = thread::Builder::new()
        .name("futex-holder".into())
        .spawn(move || loop {
            let _g = holder_lock.lock().unwrap();
            thread::sleep(Duration::from_secs(10));
        })?;
    let waiter_lock = lock.clone();
    let _waiter = thread::Builder::new()
        .name("futex-wait".into())
        .spawn(move || loop {
            let _g = waiter_lock.lock().unwrap();
            thread::sleep(Duration::from_millis(50));
        })?;

    net_recv_1.join().ok();
    net_recv_2.join().ok();
    worker_busy.join().ok();
    gc_sleeper.join().ok();
    Ok(())
}
