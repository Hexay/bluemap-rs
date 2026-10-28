//! `webserver.conf` `trusted-proxies` entries: an IP address or a CIDR block.

use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// `10.0.0.0/8`, `fd00::/8`, or a bare address (all of its bits). IPv4-mapped IPv6 addresses match as IPv4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpCidr {
    addr: IpAddr,
    prefix: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("'{0}' is not an IP address or CIDR block")]
pub struct IpCidrError(String);

impl IpCidr {
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip.to_canonical()) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => masked(net.to_bits().into(), ip.to_bits().into(), 32, self.prefix),
            (IpAddr::V6(net), IpAddr::V6(ip)) => masked(net.to_bits(), ip.to_bits(), 128, self.prefix),
            _ => false,
        }
    }
}

fn masked(net: u128, ip: u128, bits: u8, prefix: u8) -> bool {
    let shift = bits - prefix;
    shift >= bits || (net ^ ip) >> shift == 0
}

impl FromStr for IpCidr {
    type Err = IpCidrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || IpCidrError(s.to_owned());
        let (addr, prefix) = match s.trim().split_once('/') {
            Some((a, p)) => (a, Some(p.parse::<u8>().map_err(|_| err())?)),
            None => (s.trim(), None),
        };
        let addr = addr.parse::<IpAddr>().map_err(|_| err())?.to_canonical();
        let bits = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = prefix.unwrap_or(bits);
        if prefix > bits {
            return Err(err());
        }
        Ok(Self { addr, prefix })
    }
}

impl fmt::Display for IpCidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

impl Serialize for IpCidr {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for IpCidr {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn parses_and_matches() {
        let net: IpCidr = "10.1.0.0/16".parse().unwrap();
        assert!(net.contains(ip("10.1.200.3")) && !net.contains(ip("10.2.0.1")));
        assert!(net.contains(ip("::ffff:10.1.0.9")), "mapped IPv4 matches as IPv4");
        let one: IpCidr = "127.0.0.1".parse().unwrap();
        assert!(one.contains(ip("127.0.0.1")) && !one.contains(ip("127.0.0.2")));
        let v6: IpCidr = "fd00:1::/32".parse().unwrap();
        assert!(v6.contains(ip("fd00:1:ffff::1")) && !v6.contains(ip("fd00:2::1")) && !v6.contains(ip("10.1.0.1")));
        assert!("0.0.0.0/0".parse::<IpCidr>().unwrap().contains(ip("8.8.8.8")));
        assert!("::/0".parse::<IpCidr>().unwrap().contains(ip("2001:db8::1")));
        assert_eq!("::ffff:192.168.0.0/120".parse::<IpCidr>(), Err(IpCidrError("::ffff:192.168.0.0/120".into())));
        for bad in ["", "10.0.0.0/33", "::/129", "proxy.local", "10.0.0/8", "10.0.0.0/"] {
            assert!(bad.parse::<IpCidr>().is_err(), "{bad}");
        }
        assert_eq!(one.to_string(), "127.0.0.1/32");
    }
}
