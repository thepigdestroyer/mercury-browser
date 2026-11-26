// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The relying party id rules of `validate_rp_id`: what a passkey may be bound to.

use mercury_passkeys::validate_rp_id;

fn label(length: usize) -> String {
    "a".repeat(length)
}

#[test]
fn ordinary_host_names_are_accepted() {
    for rp_id in [
        "example.com",
        "a.b.example.co.uk",
        "localhost",
        "my_host.example",
        "xn--bcher-kva.example",
        "a-b.example",
    ] {
        assert!(validate_rp_id(rp_id).is_ok(), "{rp_id:?} must be accepted");
    }
}

#[test]
fn a_label_of_63_bytes_is_accepted_and_64_is_refused() {
    let longest = format!("{}.example.com", label(63));
    assert!(validate_rp_id(&longest).is_ok());
    let too_long = format!("{}.example.com", label(64));
    assert!(validate_rp_id(&too_long).is_err());
}

#[test]
fn a_total_of_253_bytes_is_accepted_and_254_is_refused() {
    let longest = format!("{}.{}.{}.{}", label(63), label(63), label(63), label(61));
    assert_eq!(longest.len(), 253);
    assert!(validate_rp_id(&longest).is_ok());
    let too_long = format!("{}.{}.{}.{}", label(63), label(63), label(63), label(62));
    assert_eq!(too_long.len(), 254);
    assert!(validate_rp_id(&too_long).is_err());
}

#[test]
fn ip_addresses_are_refused() {
    for rp_id in [
        "1.2.3.4",
        "127.0.0.1",
        "192.168.1.1",
        "0",
        "0x7f.0.0.1",
        "::1",
        "[::1]",
        "2001:db8::1",
    ] {
        assert!(validate_rp_id(rp_id).is_err(), "{rp_id:?} must be refused");
    }
}

#[test]
fn a_leading_or_trailing_hyphen_in_a_label_is_refused() {
    for rp_id in [
        "-example.com",
        "example-.com",
        "example.-com",
        "example.com-",
    ] {
        assert!(validate_rp_id(rp_id).is_err(), "{rp_id:?} must be refused");
    }
}

#[test]
fn an_empty_label_is_refused() {
    for rp_id in ["", ".", ".example.com", "example.com.", "a..b"] {
        assert!(validate_rp_id(rp_id).is_err(), "{rp_id:?} must be refused");
    }
}

#[test]
fn uppercase_is_refused() {
    for rp_id in ["Example.com", "EXAMPLE.COM", "example.COM"] {
        assert!(validate_rp_id(rp_id).is_err(), "{rp_id:?} must be refused");
    }
}

#[test]
fn non_ascii_is_refused() {
    for rp_id in ["bücher.example", "例え.jp", "exämple.com"] {
        assert!(validate_rp_id(rp_id).is_err(), "{rp_id:?} must be refused");
    }
}

#[test]
fn bidi_controls_are_refused() {
    for rp_id in [
        "example\u{202E}moc.elpmaxe",
        "exa\u{2066}mple.com",
        "example\u{200B}.com",
    ] {
        assert!(validate_rp_id(rp_id).is_err(), "{rp_id:?} must be refused");
    }
}
