//! Finding the desktop on the local network without typing IP addresses.
//!
//! The desktop answers UDP broadcasts on [`super::DISCOVERY_PORT`]. If the
//! phone cannot reach the address saved at pairing (e.g. the router gave the
//! PC a new IP), it broadcasts `INVSYNC?<server_id>` and the desktop replies
//! `INVSYNC!<server_id>:<http_port>`. Plain std networking — works on Windows
//! and Android without extra libraries.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

const ASK: &str = "INVSYNC?";
const ANSWER: &str = "INVSYNC!";

/// Best guess of this machine's LAN IPv4 address (the one used for the
/// default route). No packet is actually sent.
pub fn local_ip() -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    // Any routable address works; connect() on UDP only selects a route.
    for target in ["192.168.1.1:9", "10.0.0.1:9", "8.8.8.8:80"] {
        if sock.connect(target).is_ok() {
            if let Ok(SocketAddr::V4(a)) = sock.local_addr() {
                if !a.ip().is_unspecified() && !a.ip().is_loopback() {
                    return Some(*a.ip());
                }
            }
        }
    }
    None
}

/// Starts the desktop's discovery responder on a background thread.
/// Returns an error if the port is already in use.
pub fn spawn_responder(server_id: String, http_port: u16, udp_port: u16) -> std::io::Result<thread::JoinHandle<()>> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, udp_port))?;
    Ok(thread::Builder::new().name("inv-discovery".into()).spawn(move || {
        let mut buf = [0u8; 256];
        loop {
            let Ok((n, from)) = sock.recv_from(&mut buf) else {
                thread::sleep(Duration::from_millis(200));
                continue;
            };
            let msg = String::from_utf8_lossy(&buf[..n]);
            if let Some(wanted) = msg.strip_prefix(ASK) {
                let wanted = wanted.trim();
                // Empty id = "any hub" (used by the first-time manual search).
                if wanted.is_empty() || wanted == server_id {
                    let reply = format!("{ANSWER}{server_id}:{http_port}");
                    let _ = sock.send_to(reply.as_bytes(), from);
                }
            }
        }
    })?)
}

/// Broadcasts for the hub with `server_id` (empty = any) and returns its
/// `ip:port` if one answers within `timeout`.
pub fn find_hub(server_id: &str, udp_port: u16, timeout: Duration) -> Option<(String, String)> {
    find_hub_at(server_id, &[SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), udp_port)], timeout)
}

/// Like [`find_hub`] but asks specific addresses (also used in tests).
/// Returns `(server_id, "ip:port")`.
pub fn find_hub_at(server_id: &str, targets: &[SocketAddr], timeout: Duration) -> Option<(String, String)> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    let _ = sock.set_broadcast(true);
    sock.set_read_timeout(Some(Duration::from_millis(300))).ok()?;
    let ask = format!("{ASK}{server_id}");
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 256];
    while Instant::now() < deadline {
        for t in targets {
            let _ = sock.send_to(ask.as_bytes(), t);
        }
        let until = Instant::now() + Duration::from_millis(900);
        while Instant::now() < until.min(deadline) {
            let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
            let msg = String::from_utf8_lossy(&buf[..n]);
            let Some(rest) = msg.strip_prefix(ANSWER) else { continue };
            let Some((id, port)) = rest.trim().rsplit_once(':') else { continue };
            if !server_id.is_empty() && id != server_id {
                continue;
            }
            let Ok(port) = port.parse::<u16>() else { continue };
            return Some((id.to_string(), format!("{}:{port}", from.ip())));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responder_answers() {
        let port = 47_900 + (std::process::id() % 50) as u16;
        spawn_responder("hub123".into(), 47810, port).unwrap();
        let target = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let found = find_hub_at("hub123", &[target], Duration::from_secs(2));
        assert_eq!(found, Some(("hub123".to_string(), "127.0.0.1:47810".to_string())));
        assert_eq!(find_hub_at("other", &[target], Duration::from_millis(700)), None);
    }
}
