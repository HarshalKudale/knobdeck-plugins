//! Internet speed test against Cloudflare's public speed endpoints (the ones speed.cloudflare.com
//! itself uses): `__down?bytes=N` streams N bytes, `__up` accepts a POST body. No account, no key.

use knobdeck_sdk::{Error, Result};
use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

const BASE: &str = "https://speed.cloudflare.com";
/// Each direction stops after this long, whatever the size reached.
const PHASE: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Results {
    pub down_mbps: f64,
    pub up_mbps: f64,
    pub ping_ms: f64,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(5)).timeout(Duration::from_secs(20)).user_agent("KnobDeck speedtest").build()
}

fn net(e: ureq::Error) -> Error {
    Error::msg(format!("Speed test server did not answer ({e})"))
}

/// Median round trip of small requests on one kept-alive connection (the first one, which pays
/// for the TLS handshake, is not counted).
pub fn latency(agent: &ureq::Agent) -> Result<f64> {
    let mut times = vec![];
    for i in 0..6 {
        let t = Instant::now();
        agent.get(&format!("{BASE}/__down?bytes=0")).call().map_err(net)?.into_string().ok();
        if i > 0 {
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
    }
    Ok(median(&mut times))
}

pub fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let m = v.len() / 2;
    if v.len() % 2 == 0 {
        (v[m - 1] + v[m]) / 2.0
    } else {
        v[m]
    }
}

pub fn mbps(bytes: u64, secs: f64) -> f64 {
    if secs <= 0.0 {
        return 0.0;
    }
    bytes as f64 * 8.0 / secs / 1_000_000.0
}

/// Sizes the servers accept (they refuse some others with 403, e.g. 16 MB): the first one only
/// warms the connection up, the last one repeats until the phase time is used up.
const DOWN_SIZES: &[u64] = &[1_000_000, 10_000_000, 25_000_000];
const UP_SIZES: &[u64] = &[500_000, 2_000_000, 10_000_000, 25_000_000];

fn size_at(sizes: &[u64], i: usize) -> u64 {
    sizes[i.min(sizes.len() - 1)]
}

/// One phase: transfer chunks until the time is up; speed over the chunks after the first. A
/// refused chunk ends the phase early when some data was already measured.
fn phase(sizes: &[u64], mut chunk: impl FnMut(u64, Instant) -> Result<u64>) -> Result<f64> {
    let start = Instant::now();
    let (mut bytes, mut secs) = (0u64, 0.0f64);
    let mut i = 0;
    while start.elapsed() < PHASE {
        let t = Instant::now();
        match chunk(size_at(sizes, i), start) {
            Ok(got) if i > 0 => {
                bytes += got;
                secs += t.elapsed().as_secs_f64();
            }
            Ok(_) => {}
            Err(_) if bytes > 0 => break,
            Err(e) => return Err(e),
        }
        i += 1;
    }
    Ok(mbps(bytes, secs))
}

fn download(agent: &ureq::Agent) -> Result<f64> {
    let mut buf = vec![0u8; 64 * 1024];
    phase(DOWN_SIZES, |size, start| {
        let mut r = agent.get(&format!("{BASE}/__down?bytes={size}")).call().map_err(net)?.into_reader();
        let mut got = 0u64;
        loop {
            match r.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => got += n as u64,
                Err(e) => return Err(Error::msg(format!("Download stopped: {e}"))),
            }
            if start.elapsed() >= PHASE + Duration::from_secs(2) {
                break; // a slow line: count what arrived
            }
        }
        Ok(got)
    })
}

fn upload(agent: &ureq::Agent) -> Result<f64> {
    phase(UP_SIZES, |size, _| {
        let body = vec![b'0'; size as usize];
        agent.post(&format!("{BASE}/__up")).set("Content-Type", "text/plain").send_bytes(&body).map_err(net)?.into_string().ok();
        Ok(size)
    })
}

pub fn run() -> Result<Results> {
    let a = agent();
    let ping_ms = latency(&a)?;
    let down_mbps = download(&a)?;
    let up_mbps = upload(&a)?;
    Ok(Results { down_mbps, up_mbps, ping_ms })
}

/// TCP connect time to `host` (port 443 unless given): works without the raw sockets ICMP ping needs.
pub fn connect_ms(host: &str) -> Result<f64> {
    // "host:port" and "[v6]:port" are used as given; a bare host gets port 443.
    let target = if host.contains(':') { host.to_string() } else { format!("{host}:443") };
    let addrs: Vec<_> = target.to_socket_addrs().map_err(|e| Error::msg(format!("Unknown host {host}: {e}")))?.collect();
    // IPv4 first: a machine without an IPv6 route refuses or times out on v6 addresses.
    let mut addrs = addrs;
    addrs.sort_by_key(|a| a.is_ipv6());
    let mut last = Error::msg(format!("Unknown host {host}"));
    for addr in addrs {
        let t = Instant::now();
        match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
            Ok(_) => return Ok(t.elapsed().as_secs_f64() * 1000.0),
            Err(e) => last = Error::msg(format!("{host} did not answer ({e})")),
        }
    }
    Err(last)
}

/// Short number for a key: "940", "94.5", "9.45".
pub fn short(v: f64) -> String {
    if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maths() {
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&mut [4.0, 1.0, 2.0, 3.0]), 2.5);
        assert_eq!(median(&mut []), 0.0);
        assert_eq!(mbps(1_000_000, 1.0), 8.0);
        assert_eq!(short(940.4), "940");
        assert_eq!(short(94.54), "94.5");
        assert_eq!(short(9.456), "9.46");
    }

    #[test]
    #[ignore = "network"]
    fn live() {
        let r = run().unwrap();
        eprintln!("{r:?}");
        assert!(r.down_mbps > 0.0 && r.up_mbps > 0.0 && r.ping_ms > 0.0);
        assert!(connect_ms("one.one.one.one").unwrap() > 0.0);
    }
}
