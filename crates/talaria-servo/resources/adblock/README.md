# Bundled adblock filters

Mercury builds its network blocker from the same validated filter snapshots distributed to Brave
clients. The files are pinned to commit `d4a2a0485d69067be2f6a412aeacdffef8291533` on the `lists`
branch of <https://github.com/brave/adblock-lists-mirror>.

The mirror maps each source URL to an MD5-named artifact. These snapshots are stored with readable
names here:

| Local file | Brave mirror artifact | SHA-256 | Licence |
| --- | --- | --- | --- |
| `easylist.txt` | `17ba74de8f13543dbff29e37b3ce125d.txt` | `ca554da776cd140d0a4803bc6a298c5d2251595e13ddbdcad888c347ee7ad1f8` | CC-BY-SA-3.0 (dual-licensed upstream; Mercury uses the CC BY-SA option) |
| `easyprivacy.txt` | `3017d28ef6b8789fc7064dbd848b8a7d.txt` | `6b4e1f32bfd3363c5dfd1afa5a0bb50277352843b7df469556d335e96f53e275` | CC-BY-SA-3.0 (dual-licensed upstream; Mercury uses the CC BY-SA option) |
| `brave-firstparty.txt` | `b85138f942539c8e8909d0c42779d662.txt` | `ea61ac33974c3f71712ad2862bb53da0d7de6638551fd3ba322b7f5f5f0f2135` | MPL-2.0 |
| `brave-specific.txt` | `e8b588fd6284a3127c4b195d102b75bb.txt` | `f2fa61e972136c51b052cf1e832480cbba22adf03c1210eaff5a654b8e0935a1` | MPL-2.0 |
| `brave-unbreak.txt` | `a1e25973d7f1fa978e3db67c3114dd5b.txt` | `63a1fdc6aeebbb93b8c15602b6180c85f9aadfddfab200b7c8f4e0cc8ee76159` | MPL-2.0 |

The files retain their upstream headers and remain subject to their respective upstream licenses.
They are data read by the content blocker, redistributed unmodified, and are not part of Mercury
Browser's source code. EasyList and EasyPrivacy are dual licensed by the EasyList authors under the
GNU GPL version 3 or later and CC BY-SA 3.0 Unported or later (<https://easylist.to/pages/licence.html>);
Mercury Browser uses them under CC BY-SA 3.0, with credit to "The EasyList authors
(https://easylist.to/)". The three Brave lists carry no licence line; they are published from brave/adblock-lists, which is
MPL-2.0. The licence texts are in `LICENSES/` and `LICENSE` at the repository root; sources and
attribution are recorded in `THIRD-PARTY-NOTICES.md` under "Bundled data".

The uBlock filters (`filters/filters.txt` of uBlockOrigin/uAssets, GPL-3.0 only) were bundled until
18 Nov 2026 and are no longer: everything Mercury Browser ships must be usable in a commercial
product (licence audit R16, P1). No GPL-only list is bundled or fetched.

`manifest.json` names each list with its publisher URL (the URL the Brave mirror fetches it from:
the mirror's artifact name is the MD5 of that URL), licence, version or date and SHA-256. The files
are data, never compiled into the program: `filter-lists install` (the binary of `crates/mercury-filter-lists`) checks them against the
manifest and copies them beside the executable (`Contents/Resources/filter-lists/` in the macOS
bundle, `filter-lists/` in the Linux archive and in `target/<profile>/` after `make build-release`).
At start-up `crates/mercury-filter-lists` builds the network-rule engine off the UI thread from the
newest valid copy of each list (this one, or an update in the profile's `filter-lists/` cache) and
caches it compiled; it fetches updates from the publisher URLs at most once a day. Cosmetic
(element-hiding) rules are not used.
