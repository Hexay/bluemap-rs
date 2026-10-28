//! Who a request is from, for the `server-unpack` fair share (docs/18): the peer address, or behind a trusted
//! reverse proxy the client it names.

use std::net::{IpAddr, Ipv4Addr};

use bm_config::IpCidr;
use http::HeaderMap;

/// An IPv4 address, or an IPv6 /64 (one host owns a whole /64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ClientKey(u128);

impl ClientKey {
    pub(crate) fn of_ip(ip: IpAddr) -> Self {
        match ip.to_canonical() {
            // the mapped form: its low 64 bits are never zero, so it can't collide with a /64
            IpAddr::V4(v4) => Self(v4.to_ipv6_mapped().to_bits()),
            IpAddr::V6(v6) => Self(v6.to_bits() & !u128::from(u64::MAX)),
        }
    }

    /// `peer`, or when it is a trusted proxy the right-most `X-Forwarded-For` entry that is not one (all entries
    /// trusted: the left-most), else its `X-Real-IP`. An entry that doesn't parse ends the walk at the hop before it.
    pub(crate) fn of(peer: Option<IpAddr>, headers: &HeaderMap, trusted: &[IpCidr]) -> Self {
        let peer = peer.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        let is_trusted = |ip: IpAddr| trusted.iter().any(|net| net.contains(ip));
        if !is_trusted(peer) {
            return Self::of_ip(peer);
        }
        let mut forwarded = headers
            .get_all("x-forwarded-for")
            .iter()
            .flat_map(|v| v.to_str().unwrap_or("").split(','))
            .rev()
            .map(parse_ip)
            .peekable();
        if forwarded.peek().is_none() {
            let real_ip = headers.get("x-real-ip").and_then(|v| v.to_str().ok()).and_then(parse_ip);
            return Self::of_ip(real_ip.unwrap_or(peer));
        }
        let mut client = peer;
        for hop in forwarded {
            let Some(hop) = hop else { break };
            client = hop;
            if !is_trusted(hop) {
                break;
            }
        }
        Self::of_ip(client)
    }
}

/// `1.2.3.4`, `1.2.3.4:5678`, `2001:db8::1`, `[2001:db8::1]:443`.
fn parse_ip(s: &str) -> Option<IpAddr> {
    let s = s.trim();
    if let Ok(ip) = s.parse() {
        return Some(ip);
    }
    if let Some(rest) = s.strip_prefix('[') {
        return rest.split_once(']')?.0.parse().ok();
    }
    s.rsplit_once(':')?.0.parse::<Ipv4Addr>().ok().map(IpAddr::V4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> ClientKey {
        ClientKey::of_ip(s.parse().unwrap())
    }

    fn of(peer: &str, headers: &[(&str, &str)], trusted: &[&str]) -> ClientKey {
        let mut map = HeaderMap::new();
        for (k, v) in headers {
            map.append(http::HeaderName::from_bytes(k.as_bytes()).unwrap(), v.parse().unwrap());
        }
        let trusted: Vec<IpCidr> = trusted.iter().map(|t| t.parse().unwrap()).collect();
        ClientKey::of(Some(peer.parse().unwrap()), &map, &trusted)
    }

    #[test]
    fn ipv6_is_grouped_by_64_and_ipv4_kept_whole() {
        assert_eq!(key("2001:db8:1:2:aaaa::1"), key("2001:db8:1:2:ffff:1:2:3"));
        assert_ne!(key("2001:db8:1:2::1"), key("2001:db8:1:3::1"));
        assert_ne!(key("10.0.0.1"), key("10.0.0.2"));
        assert_eq!(key("::ffff:10.0.0.1"), key("10.0.0.1"), "dual-stack listeners see IPv4 peers mapped");
        assert_ne!(key("0.0.0.1"), key("::"));
    }

    #[test]
    fn forwarded_for_only_counts_from_trusted_peers() {
        let xff = [("X-Forwarded-For", "203.0.113.9")];
        assert_eq!(of("198.51.100.7", &xff, &[]), key("198.51.100.7"), "nothing trusted: header ignored");
        assert_eq!(of("198.51.100.7", &xff, &["127.0.0.1"]), key("198.51.100.7"), "spoofed by an untrusted peer");
        assert_eq!(of("127.0.0.1", &xff, &["127.0.0.1"]), key("203.0.113.9"));
        assert_eq!(of("::ffff:127.0.0.1", &xff, &["127.0.0.0/8"]), key("203.0.113.9"));
        assert_eq!(of("127.0.0.1", &[], &["127.0.0.1"]), key("127.0.0.1"), "a proxy without the header");
    }

    #[test]
    fn right_most_untrusted_hop_of_a_chain() {
        let trusted = ["127.0.0.1", "10.0.0.0/8"];
        // the client forged the first entry; the edge proxy (10.0.0.5) appended the address it saw
        let chain = [("X-Forwarded-For", "6.6.6.6, 203.0.113.9, 10.0.0.5")];
        assert_eq!(of("127.0.0.1", &chain, &trusted), key("203.0.113.9"));
        let split = [("X-Forwarded-For", "6.6.6.6, 203.0.113.9"), ("X-Forwarded-For", "10.0.0.5")];
        assert_eq!(of("127.0.0.1", &split, &trusted), key("203.0.113.9"), "repeated headers form one list");
        let all_trusted = [("X-Forwarded-For", "10.0.0.7, 10.0.0.5")];
        assert_eq!(of("127.0.0.1", &all_trusted, &trusted), key("10.0.0.7"));
        let garbage = [("X-Forwarded-For", "203.0.113.9, unknown, 10.0.0.5")];
        assert_eq!(of("127.0.0.1", &garbage, &trusted), key("10.0.0.5"), "stops at the hop that wrote garbage");
        let ports = [("X-Forwarded-For", "[2001:db8:5::1]:443, 203.0.113.9:5678")];
        assert_eq!(of("127.0.0.1", &ports, &["127.0.0.1", "203.0.113.0/24"]), key("2001:db8:5::1"));
        let v6 = [("X-Forwarded-For", "2001:db8:7:7::1, fd00::2")];
        assert_eq!(of("::1", &v6, &["::1", "fd00::/8"]), key("2001:db8:7:7:ffff::9"), "grouped by /64");
    }

    #[test]
    fn real_ip_when_there_is_no_forwarded_for() {
        let real = [("X-Real-IP", "203.0.113.9")];
        assert_eq!(of("127.0.0.1", &real, &["127.0.0.1"]), key("203.0.113.9"));
        assert_eq!(of("198.51.100.7", &real, &["127.0.0.1"]), key("198.51.100.7"));
        let both = [("X-Real-IP", "6.6.6.6"), ("X-Forwarded-For", "203.0.113.9")];
        assert_eq!(of("127.0.0.1", &both, &["127.0.0.1"]), key("203.0.113.9"));
        assert_eq!(of("127.0.0.1", &[("X-Real-IP", "nonsense")], &["127.0.0.1"]), key("127.0.0.1"));
    }
}
