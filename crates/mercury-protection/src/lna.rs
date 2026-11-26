// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The local-network address classifier (R14c MH56, spec §5.5).
//!
//! [`classify_ip`] and [`classify_host`] say which address space a target lives in; [`lna_decision`] says whether a
//! page in one space may reach a target in another.
//!
//! DNS rebinding: classifying a name says nothing about where it resolves. The check that closes rebinding runs
//! after resolution, on the address that will be connected to ([`classify_ip`]). A document's space comes from the
//! remote address it was loaded from ([`space_of_document`]), not from its name. A name that matches none of the
//! rules is [`AddressSpace::Public`], whatever it resolves to.

use std::net::IpAddr;

use url::{Host, Url};

use crate::settings::LnaPolicy;

/// The longest host name the classifier reads (RFC 1035 presentation form).
const MAX_HOST_BYTES: usize = 253;

/// `64:ff9b::/96` (RFC 6052), as the top 96 bits of an IPv6 address.
const NAT64_WELL_KNOWN: u128 = 0x0064_ff9b << 64;

/// `64:ff9b:1::/48` (RFC 8215), as the top 48 bits of an IPv6 address.
const NAT64_LOCAL_USE: u128 = 0x0064_ff9b_0001;

/// Where an address lives, ordered from the most public to this machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AddressSpace {
    /// The internet.
    Public,
    /// The user's local network.
    Local,
    /// This machine.
    Loopback,
}

/// What a target address is, by the block it falls in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressClass {
    /// A routable address on the internet.
    Public,
    /// `127.0.0.0/8`, `::1`, `localhost` and `*.localhost`.
    Loopback,
    /// RFC 1918 (`10/8`, `172.16/12`, `192.168/16`), and `*.local`, `*.internal`, `*.home.arpa`, `*.lan`, and
    /// single-label names.
    Private,
    /// `169.254/16` and `fe80::/10`.
    LinkLocal,
    /// `fc00::/7`.
    UniqueLocal,
    /// `fec0::/10`, the deprecated site-local block.
    SiteLocal,
    /// `100.64/10`, carrier-grade NAT.
    SharedAddress,
    /// `0.0.0.0/8` and `::`: connecting to these reaches this machine.
    Unspecified,
    /// `224/4` and `ff00::/8`.
    Multicast,
    /// `255.255.255.255`.
    Broadcast,
    /// `192.0.2/24`, `198.51.100/24`, `203.0.113/24` and `2001:db8::/32`.
    Documentation,
    /// `240/4`, `192.0.0/24` and `198.18/15`.
    Reserved,
}

impl AddressClass {
    /// The address space this class belongs to.
    pub fn space(self) -> AddressSpace {
        match self {
            Self::Loopback => AddressSpace::Loopback,
            Self::Private
            | Self::LinkLocal
            | Self::UniqueLocal
            | Self::SiteLocal
            | Self::SharedAddress
            | Self::Unspecified => AddressSpace::Local,
            Self::Public
            | Self::Multicast
            | Self::Broadcast
            | Self::Documentation
            | Self::Reserved => AddressSpace::Public,
        }
    }

    /// Whether this class is in the public space.
    pub fn is_public(self) -> bool {
        self.space() == AddressSpace::Public
    }
}

/// What to do with a request from one space to a more private one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LnaDecision {
    /// Let the request through.
    Allow,
    /// Refuse it.
    Block,
    /// Ask the user first.
    Ask,
}

/// Classifies an IP address. Embedded IPv4 (mapped, compatible, NAT64, 6to4) is classified as the IPv4 address.
pub fn classify_ip(ip: IpAddr) -> AddressClass {
    match ip {
        IpAddr::V4(address) => classify_v4(u32::from(address)),
        IpAddr::V6(address) => classify_v6(u128::from(address)),
    }
}

/// Classifies a parsed URL host.
pub fn classify_host(host: &Host<&str>) -> AddressClass {
    match host {
        Host::Domain(name) => classify_name(name),
        Host::Ipv4(address) => classify_v4(u32::from(*address)),
        Host::Ipv6(address) => classify_v6(u128::from(*address)),
    }
}

/// Classifies text parsed the way a URL host is, so `2130706433` and `0x7f.1` are `127.0.0.1`. `None` when the text
/// is not a host.
pub fn classify_host_str(text: &str) -> Option<AddressClass> {
    if text.len() > MAX_HOST_BYTES {
        return None;
    }
    match Host::parse(text).ok()? {
        Host::Domain(name) => Some(classify_name(&name)),
        Host::Ipv4(address) => Some(classify_v4(u32::from(address))),
        Host::Ipv6(address) => Some(classify_v6(u128::from(address))),
    }
}

/// The address space of a URL's host. `None` for a URL with no host (`data:`, `about:`).
pub fn space_of_url(url: &Url) -> Option<AddressSpace> {
    url.host().map(|host| classify_host(&host).space())
}

/// The address space of a document: its remote address when known, else its host. `None` for a URL with no host.
pub fn space_of_document(url: &Url, remote_ip: Option<IpAddr>) -> Option<AddressSpace> {
    url.host()?;
    match remote_ip {
        Some(ip) => Some(classify_ip(ip).space()),
        None => space_of_url(url),
    }
}

/// Decides a request from a document in `initiator` to a target in `target`. A request to the same or a less
/// private space is allowed; a more private one is blocked or asked about, unless the protection is off.
pub fn lna_decision(
    initiator: AddressSpace,
    target: AddressSpace,
    protection_active: bool,
    policy: LnaPolicy,
) -> LnaDecision {
    if target <= initiator || !protection_active {
        return LnaDecision::Allow;
    }
    match policy {
        LnaPolicy::Block => LnaDecision::Block,
        LnaPolicy::Ask => LnaDecision::Ask,
    }
}

fn classify_name(name: &str) -> AddressClass {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    if name == "localhost" || name.ends_with(".localhost") {
        return AddressClass::Loopback;
    }
    let single_label = !name.contains('.');
    if single_label
        || [".local", ".internal", ".home.arpa", ".lan"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
    {
        return AddressClass::Private;
    }
    AddressClass::Public
}

fn classify_v4(v: u32) -> AddressClass {
    let [a, b, c, _] = v.to_be_bytes();
    if a == 127 {
        return AddressClass::Loopback;
    }
    if a == 10 || (a == 172 && (b & 0xf0) == 16) || (a == 192 && b == 168) {
        return AddressClass::Private;
    }
    if a == 169 && b == 254 {
        return AddressClass::LinkLocal;
    }
    if a == 100 && (b & 0xc0) == 64 {
        return AddressClass::SharedAddress;
    }
    if a == 0 {
        return AddressClass::Unspecified;
    }
    if v == u32::MAX {
        return AddressClass::Broadcast;
    }
    if (a & 0xf0) == 0xe0 {
        return AddressClass::Multicast;
    }
    if matches!((a, b, c), (192, 0, 2) | (198, 51, 100) | (203, 0, 113)) {
        return AddressClass::Documentation;
    }
    if a >= 240 || (a, b, c) == (192, 0, 0) || (a == 198 && (b & 0xfe) == 18) {
        return AddressClass::Reserved;
    }
    AddressClass::Public
}

fn classify_v6(v: u128) -> AddressClass {
    if v == 1 {
        return AddressClass::Loopback;
    }
    if v == 0 {
        return AddressClass::Unspecified;
    }
    let high96 = v >> 32;
    if high96 == 0xffff || high96 == 0 || high96 == NAT64_WELL_KNOWN {
        return classify_v4(v as u32);
    }
    if v >> 80 == NAT64_LOCAL_USE {
        // The embedded IPv4 is read from the last 32 bits or from bits 48..80; the more private reading wins.
        let suffix = classify_v4(v as u32);
        let embedded = classify_v4((v >> 48) as u32);
        return if suffix.space() >= embedded.space() {
            suffix
        } else {
            embedded
        };
    }
    if v >> 112 == 0x2002 {
        return classify_v4((v >> 80) as u32);
    }
    if v >> 121 == 0x7e {
        return AddressClass::UniqueLocal;
    }
    if v >> 118 == 0x3fa {
        return AddressClass::LinkLocal;
    }
    if v >> 118 == 0x3fb {
        return AddressClass::SiteLocal;
    }
    if v >> 120 == 0xff {
        return AddressClass::Multicast;
    }
    if v >> 96 == 0x2001_0db8 {
        return AddressClass::Documentation;
    }
    AddressClass::Public
}
