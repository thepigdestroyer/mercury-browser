// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: tests for the partitioned cookie jar (EC7, storage side): a `Partitioned` cookie is kept per
// top-level site. See mercury_partitions.rs. New file.

//! The partitioned jar, on its own: no server, no fetch. The jar is driven through `push_scoped` and
//! `cookies_for_scope` with a scope written by hand, as the network policy builds it.

use net::cookie::ServoCookie;
use net::cookie_storage::CookieStorage;
use net::{CookieAccess, CookieScope, MAX_PARTITIONS};
use net_traits::CookieSource;
use servo_url::ServoUrl;

fn scope(access: CookieAccess, partition: &str) -> CookieScope {
    CookieScope {
        access,
        partition: Some(partition.to_owned()),
    }
}

fn partitioned(cookie: &str, url: &ServoUrl) -> ServoCookie {
    ServoCookie::from_cookie_string(
        &format!("{cookie}; Secure; Partitioned"),
        url,
        CookieSource::HTTP,
    )
    .unwrap()
}

fn plain(cookie: &str, url: &ServoUrl) -> ServoCookie {
    ServoCookie::from_cookie_string(cookie, url, CookieSource::HTTP).unwrap()
}

fn any(_: &cookie::Cookie<'static>) -> bool {
    true
}

fn read(jar: &mut CookieStorage, url: &ServoUrl, scope: &CookieScope) -> Option<String> {
    jar.cookies_for_scope(url, CookieSource::HTTP, scope, &any)
}

#[test]
fn a_partitioned_cookie_is_stored_and_read_only_under_its_own_partition() {
    let mut jar = CookieStorage::new(150);
    let url = ServoUrl::parse("https://embed.test/x").unwrap();
    let a = scope(CookieAccess::PartitionedOnly, "a.test");
    jar.push_scoped(partitioned("sid=1", &url), &url, CookieSource::HTTP, &a);
    assert_eq!(read(&mut jar, &url, &a).as_deref(), Some("sid=1"));
    let c = scope(CookieAccess::PartitionedOnly, "c.test");
    assert_eq!(read(&mut jar, &url, &c), None);
}

#[test]
fn the_same_name_domain_and_path_in_two_partitions_does_not_overwrite() {
    let mut jar = CookieStorage::new(150);
    let url = ServoUrl::parse("https://embed.test/x").unwrap();
    let a = scope(CookieAccess::PartitionedOnly, "a.test");
    let c = scope(CookieAccess::PartitionedOnly, "c.test");
    jar.push_scoped(partitioned("sid=1", &url), &url, CookieSource::HTTP, &a);
    jar.push_scoped(partitioned("sid=2", &url), &url, CookieSource::HTTP, &c);
    assert_eq!(jar.partition_count(), 2);
    assert_eq!(read(&mut jar, &url, &a).as_deref(), Some("sid=1"));
    assert_eq!(read(&mut jar, &url, &c).as_deref(), Some("sid=2"));
}

#[test]
fn an_unpartitioned_cookie_is_refused_in_partitioned_only_mode_and_stored_in_full_mode() {
    let mut jar = CookieStorage::new(150);
    let url = ServoUrl::parse("https://embed.test/x").unwrap();
    let partitioned_only = scope(CookieAccess::PartitionedOnly, "a.test");
    jar.push_scoped(
        plain("plain=1", &url),
        &url,
        CookieSource::HTTP,
        &partitioned_only,
    );
    assert_eq!(jar.partition_count(), 0);
    let full = scope(CookieAccess::Full, "a.test");
    assert_eq!(read(&mut jar, &url, &full), None);
    jar.push_scoped(plain("plain=1", &url), &url, CookieSource::HTTP, &full);
    assert_eq!(read(&mut jar, &url, &full).as_deref(), Some("plain=1"));
}

#[test]
fn full_mode_sends_the_unpartitioned_cookies_and_the_partition_of_the_top_level_site() {
    let mut jar = CookieStorage::new(150);
    let url = ServoUrl::parse("https://embed.test/x").unwrap();
    let a = scope(CookieAccess::Full, "a.test");
    jar.push_scoped(plain("plain=1", &url), &url, CookieSource::HTTP, &a);
    jar.push_scoped(partitioned("sid=1", &url), &url, CookieSource::HTTP, &a);
    assert_eq!(read(&mut jar, &url, &a).as_deref(), Some("plain=1; sid=1"));
    let c = scope(CookieAccess::Full, "c.test");
    assert_eq!(read(&mut jar, &url, &c).as_deref(), Some("plain=1"));
}

#[test]
fn a_partitioned_cookie_needs_secure_and_a_key() {
    let mut jar = CookieStorage::new(150);
    let url = ServoUrl::parse("https://embed.test/x").unwrap();
    let a = scope(CookieAccess::Full, "a.test");
    let without_secure = ServoCookie::from_cookie_string(
        "sid=1; Partitioned",
        &url,
        CookieSource::HTTP,
    )
    .unwrap();
    jar.push_scoped(without_secure, &url, CookieSource::HTTP, &a);
    assert_eq!(jar.partition_count(), 0);
    assert_eq!(read(&mut jar, &url, &a), None);

    let no_key = CookieScope {
        access: CookieAccess::Full,
        partition: None,
    };
    jar.push_scoped(partitioned("sid=2", &url), &url, CookieSource::HTTP, &no_key);
    assert_eq!(jar.partition_count(), 0);
    assert_eq!(read(&mut jar, &url, &no_key), None);
}

#[test]
fn the_2049th_partition_is_refused_and_the_first_2048_keep_working() {
    let mut jar = CookieStorage::new(150);
    let url = ServoUrl::parse("https://embed.test/x").unwrap();
    for i in 0..=MAX_PARTITIONS {
        let site = scope(CookieAccess::Full, &format!("site{i}.test"));
        let cookie = partitioned(&format!("sid={i}"), &url);
        jar.push_scoped(cookie, &url, CookieSource::HTTP, &site);
    }
    assert_eq!(jar.partition_count(), MAX_PARTITIONS);
    let first = scope(CookieAccess::Full, "site0.test");
    assert_eq!(read(&mut jar, &url, &first).as_deref(), Some("sid=0"));
    let refused = scope(CookieAccess::Full, &format!("site{MAX_PARTITIONS}.test"));
    assert_eq!(read(&mut jar, &url, &refused), None);
}

#[test]
fn clearing_a_site_clears_its_cookies_in_every_partition_and_the_partition_keyed_by_it() {
    let mut jar = CookieStorage::new(150);
    let url = ServoUrl::parse("https://embed.test/x").unwrap();
    let other = ServoUrl::parse("https://other.test/x").unwrap();
    let a = scope(CookieAccess::Full, "a.test");
    let c = scope(CookieAccess::Full, "c.test");
    jar.push_scoped(partitioned("sid=1", &url), &url, CookieSource::HTTP, &a);
    jar.push_scoped(partitioned("sid=2", &url), &url, CookieSource::HTTP, &c);
    jar.push_scoped(partitioned("oth=3", &other), &other, CookieSource::HTTP, &a);
    jar.push_scoped(partitioned("oth=4", &other), &other, CookieSource::HTTP, &c);

    jar.delete_cookies_for_sites(&vec!["embed.test".to_owned()]);
    assert_eq!(read(&mut jar, &url, &a), None);
    assert_eq!(read(&mut jar, &url, &c), None);
    assert_eq!(read(&mut jar, &other, &a).as_deref(), Some("oth=3"));
    assert_eq!(read(&mut jar, &other, &c).as_deref(), Some("oth=4"));

    jar.delete_cookies_for_sites(&vec!["a.test".to_owned()]);
    assert_eq!(read(&mut jar, &other, &a), None);
    assert_eq!(read(&mut jar, &other, &c).as_deref(), Some("oth=4"));

    jar.clear_storage(None);
    assert_eq!(jar.partition_count(), 0);
}

#[test]
fn a_jar_with_partitions_round_trips_through_serialisation() {
    let mut jar = CookieStorage::new(150);
    let url = ServoUrl::parse("https://embed.test/x").unwrap();
    let a = scope(CookieAccess::Full, "a.test");
    jar.push_scoped(partitioned("sid=1", &url), &url, CookieSource::HTTP, &a);
    let bytes = postcard::to_stdvec(&jar).unwrap();
    let mut back: CookieStorage = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(back.partition_count(), 1);
    assert_eq!(read(&mut back, &url, &a).as_deref(), Some("sid=1"));
}
