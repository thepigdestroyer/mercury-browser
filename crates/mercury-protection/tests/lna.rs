// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The local-network address classifier (R14c MH56, spec §5.5).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use mercury_protection::{
    AddressClass, AddressSpace, LnaDecision, LnaPolicy, classify_host_str, classify_ip,
    lna_decision, space_of_document, space_of_url,
};
use url::Url;

use AddressClass::*;

fn v4(text: &str) -> IpAddr {
    IpAddr::V4(text.parse().expect("an IPv4 address"))
}

fn v6(text: &str) -> IpAddr {
    IpAddr::V6(text.parse().expect("an IPv6 address"))
}

fn url(text: &str) -> Url {
    Url::parse(text).expect("a URL")
}

#[test]
fn ipv4_blocks_are_classified_at_their_first_and_last_address() {
    let cases = [
        ("9.255.255.255", Public),
        ("10.0.0.0", Private),
        ("10.255.255.255", Private),
        ("11.0.0.0", Public),
        ("100.63.255.255", Public),
        ("100.64.0.0", SharedAddress),
        ("100.127.255.255", SharedAddress),
        ("100.128.0.0", Public),
        ("127.0.0.1", Loopback),
        ("127.255.255.255", Loopback),
        ("169.254.0.0", LinkLocal),
        ("169.254.255.255", LinkLocal),
        ("172.15.255.255", Public),
        ("172.16.0.0", Private),
        ("172.31.255.255", Private),
        ("172.32.0.0", Public),
        ("192.168.0.0", Private),
        ("192.168.255.255", Private),
        ("0.0.0.0", Unspecified),
        ("255.255.255.255", Broadcast),
        ("224.0.0.1", Multicast),
        ("192.0.2.1", Documentation),
        ("198.51.100.1", Documentation),
        ("203.0.113.1", Documentation),
        ("240.0.0.1", Reserved),
        ("192.0.0.1", Reserved),
        ("198.18.0.1", Reserved),
        ("8.8.8.8", Public),
        ("1.1.1.1", Public),
    ];
    for (text, class) in cases {
        assert_eq!(classify_ip(v4(text)), class, "{text}");
    }
}

#[test]
fn ipv6_blocks_are_classified() {
    let cases = [
        ("::1", Loopback),
        ("::", Unspecified),
        ("fc00::1", UniqueLocal),
        ("fdff::1", UniqueLocal),
        ("fbff::1", Public),
        ("fe80::1", LinkLocal),
        ("febf::1", LinkLocal),
        ("fec0::1", SiteLocal),
        ("ff02::1", Multicast),
        ("2001:db8::1", Documentation),
        ("2606:4700::1111", Public),
    ];
    for (text, class) in cases {
        assert_eq!(classify_ip(v6(text)), class, "{text}");
    }
}

#[test]
fn embedded_ipv4_is_classified_as_the_ipv4_address() {
    let cases = [
        ("::ffff:127.0.0.1", Loopback),
        ("::ffff:7f00:1", Loopback),
        ("::ffff:10.0.0.1", Private),
        ("::ffff:8.8.8.8", Public),
        ("::127.0.0.1", Loopback),
        ("64:ff9b::7f00:1", Loopback),
        ("64:ff9b::a00:1", Private),
        ("64:ff9b:1::7f00:1", Loopback),
        ("2002:7f00:1::", Loopback),
        ("2002:c0a8:1::1", Private),
    ];
    for (text, class) in cases {
        assert_eq!(classify_ip(v6(text)), class, "{text}");
    }
}

#[test]
fn non_dotted_ipv4_spellings_are_loopback_through_the_url_parser() {
    let cases = [
        ("http://2130706433/", Loopback),
        ("http://0x7f.1/", Loopback),
        ("http://0177.0.0.1/", Loopback),
        ("http://127.1/", Loopback),
        ("http://[::ffff:127.0.0.1]/", Loopback),
        ("http://0x0a000001/", Private),
    ];
    for (text, class) in cases {
        let parsed = url(text);
        let host = parsed.host_str().expect("a host");
        assert_eq!(classify_host_str(host), Some(class), "{text}");
        assert_eq!(space_of_url(&parsed), Some(class.space()), "{text}");
    }
}

#[test]
fn local_names_follow_their_rules() {
    let cases = [
        ("localhost", Loopback),
        ("LOCALHOST.", Loopback),
        ("a.localhost", Loopback),
        ("printer.local", Private),
        ("Printer.LOCAL.", Private),
        ("nas.internal", Private),
        ("router.home.arpa", Private),
        ("box.lan", Private),
        ("intranet", Private),
        ("localhost.example.com", Public),
        ("mylocal.example", Public),
        ("example.com", Public),
        ("local.example.org", Public),
    ];
    for (text, class) in cases {
        assert_eq!(classify_host_str(text), Some(class), "{text}");
    }
}

#[test]
fn text_that_is_not_a_host_has_no_class() {
    assert_eq!(classify_host_str(""), None);
    assert_eq!(classify_host_str("a b"), None);
    assert_eq!(classify_host_str(&"a".repeat(300)), None);
}

#[test]
fn a_document_is_classified_by_its_remote_address_when_it_has_one() {
    let evil = url("https://evil.example/");
    assert_eq!(
        space_of_document(&evil, Some(v4("127.0.0.1"))),
        Some(AddressSpace::Loopback)
    );
    assert_eq!(
        space_of_document(&evil, Some(v4("8.8.8.8"))),
        Some(AddressSpace::Public)
    );
    assert_eq!(space_of_document(&evil, None), Some(AddressSpace::Public));
    assert_eq!(
        space_of_document(&url("https://app.local/"), None),
        Some(AddressSpace::Local)
    );
    assert_eq!(space_of_document(&url("data:text/plain,x"), None), None);
    assert_eq!(
        space_of_document(&url("data:text/plain,x"), Some(v4("8.8.8.8"))),
        None
    );
}

#[test]
fn a_local_or_loopback_target_is_refused_from_a_more_public_initiator() {
    let block = LnaPolicy::Block;
    use AddressSpace::{Local, Loopback, Public};
    assert_eq!(
        lna_decision(Public, Loopback, true, block),
        LnaDecision::Block
    );
    assert_eq!(
        lna_decision(Local, Loopback, true, block),
        LnaDecision::Block
    );
    assert_eq!(
        lna_decision(Loopback, Local, true, block),
        LnaDecision::Allow
    );
    assert_eq!(
        lna_decision(Public, Public, true, block),
        LnaDecision::Allow
    );
}

#[test]
fn the_decision_table_follows_the_space_order_and_the_protection_switch() {
    let spaces = [
        AddressSpace::Public,
        AddressSpace::Local,
        AddressSpace::Loopback,
    ];
    for initiator in spaces {
        for target in spaces {
            let lna = target > initiator;
            for policy in [LnaPolicy::Block, LnaPolicy::Ask] {
                let expected = match (lna, policy) {
                    (false, _) => LnaDecision::Allow,
                    (true, LnaPolicy::Block) => LnaDecision::Block,
                    (true, LnaPolicy::Ask) => LnaDecision::Ask,
                };
                assert_eq!(lna_decision(initiator, target, true, policy), expected);
                assert_eq!(
                    lna_decision(initiator, target, false, policy),
                    LnaDecision::Allow
                );
            }
        }
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn next_u32(&mut self) -> u32 {
        (self.next() >> 32) as u32
    }
}

fn reference_v4(v: u32) -> AddressClass {
    let in_block = |net: u32, bits: u32| v & (u32::MAX << (32 - bits)) == net;
    let a = Ipv4Addr::from(v);
    if a.is_loopback() {
        Loopback
    } else if a.is_private() {
        Private
    } else if a.is_link_local() {
        LinkLocal
    } else if in_block(0x6440_0000, 10) {
        SharedAddress
    } else if in_block(0, 8) {
        Unspecified
    } else if a.is_broadcast() {
        Broadcast
    } else if a.is_multicast() {
        Multicast
    } else if a.is_documentation() {
        Documentation
    } else if in_block(0xF000_0000, 4) || in_block(0xC000_0000, 24) || in_block(0xC612_0000, 15) {
        Reserved
    } else {
        Public
    }
}

#[test]
fn ipv4_classification_agrees_with_the_reference_on_random_addresses() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..20_000 {
        let v = rng.next_u32();
        assert_eq!(
            classify_ip(IpAddr::V4(Ipv4Addr::from(v))),
            reference_v4(v),
            "{v:#010x}"
        );
    }
}

#[test]
fn embedded_ipv4_classifies_as_the_address_it_embeds() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    for _ in 0..20_000 {
        let v = rng.next_u32();
        let expected = classify_ip(IpAddr::V4(Ipv4Addr::from(v)));
        let hi = (v >> 16) as u16;
        let lo = v as u16;
        let forms = [
            Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, hi, lo),
            Ipv6Addr::new(0x64, 0xff9b, 0, 0, 0, 0, hi, lo),
            Ipv6Addr::new(0x2002, hi, lo, 0, 0, 0, 0, 0),
        ];
        for form in forms {
            assert_eq!(
                classify_ip(IpAddr::V6(form)),
                expected,
                "{form} for {v:#010x}"
            );
        }
        if v >= 1 << 16 {
            let compatible = Ipv6Addr::from(u128::from(v));
            assert_eq!(
                classify_ip(IpAddr::V6(compatible)),
                expected,
                "{compatible}"
            );
        }
    }
}

#[test]
fn host_text_never_panics() {
    let alphabet: Vec<char> = (0x20u8..0x7f)
        .map(char::from)
        .chain(['é', 'ß', '中', '\u{1F600}'])
        .collect();
    let mut rng = Rng(0xA076_1D64_78BD_642F);
    for _ in 0..5_000 {
        let length = (rng.next() % 80) as usize;
        let text: String = (0..length)
            .map(|_| alphabet[(rng.next() % alphabet.len() as u64) as usize])
            .collect();
        let _ = classify_host_str(&text);
    }
}
