// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! codec configuration extraction (avcC, vpcC, av1C, esds, dOps/OpusHead) and RFC 6381 strings.
//!
//! Written from ISO/IEC 14496-15 (`avcC`), ISO/IEC 14496-1 and -3 (`esds`, AudioSpecificConfig), the
//! VP Codec ISO Media File Format Binding, the AV1 ISOBMFF binding, the Opus-in-ISOBMFF and
//! Opus-in-Matroska encapsulations (RFC 7845 for `OpusHead`), and the codec registrations of RFC 6381.
//! The string rules start from the spike's `mse-segments` `mp4_init.rs` (Mercury's own).

use super::bmff::Reader;
use super::{PResult, check_audio_numbers, malformed};

/// `avc1.PPCCLL` from the first four bytes of an `avcC` record (profile, constraint flags, level, as
/// upper-case hex); `entry` is the sample entry type (`avc1` or `avc3`).
pub(crate) fn avc_codec_string(entry: &[u8; 4], avcc: &[u8]) -> PResult<String> {
    if avcc.len() < 7 || avcc[0] != 1 {
        return Err(malformed(
            "an avcC record that is too short or not version 1",
        ));
    }
    Ok(format!(
        "{}.{:02X}{:02X}{:02X}",
        String::from_utf8_lossy(entry),
        avcc[1],
        avcc[2],
        avcc[3]
    ))
}

/// `vp09.PP.LL.DD` from a VPCodecConfigurationRecord (the `vpcC` payload after its version and flags):
/// profile, level, then the bit depth in the high nibble of the third byte.
pub(crate) fn vp9_codec_string(record: &[u8]) -> PResult<String> {
    let (profile, level, packed) = match record {
        [p, l, d, ..] => (*p, *l, *d),
        _ => return Err(malformed("a vpcC record shorter than 3 bytes")),
    };
    Ok(format!("vp09.{profile:02}.{level:02}.{:02}", packed >> 4))
}

/// `av01.P.LLT.DD` from an `av1C` record (four fixed bytes, then the configOBUs).
pub(crate) fn av1_codec_string(av1c: &[u8]) -> PResult<String> {
    let (b0, b1, b2) = match av1c {
        [a, b, c, _, ..] => (*a, *b, *c),
        _ => return Err(malformed("an av1C record shorter than 4 bytes")),
    };
    if b0 != 0x81 {
        return Err(malformed("an av1C record with a wrong marker or version"));
    }
    let profile = b1 >> 5;
    let level = b1 & 0x1f;
    let tier = if b2 & 0x80 != 0 { 'H' } else { 'M' };
    let depth = if b2 & 0x40 != 0 {
        if profile == 2 && b2 & 0x20 != 0 {
            12
        } else {
            10
        }
    } else {
        8
    };
    Ok(format!("av01.{profile}.{level:02}{tier}.{depth:02}"))
}

/// What an `esds` box says about an audio track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Esds {
    /// MPEG-4 Systems `objectTypeIndication`: 0x40 is MPEG-4 audio (AAC).
    pub object_type: u8,
    /// The DecoderSpecificInfo: the AudioSpecificConfig for AAC.
    pub asc: Vec<u8>,
}

/// One MPEG-4 descriptor: its tag and its body. A length that runs past the data is cut to the data, as
/// real files need (muxers write wrong lengths here).
fn descriptor<'a>(r: &mut Reader<'a>) -> PResult<(u8, &'a [u8])> {
    let tag = r.u8()?;
    let mut len = 0usize;
    for _ in 0..4 {
        let b = r.u8()?;
        len = (len << 7) | usize::from(b & 0x7f);
        if b & 0x80 == 0 {
            break;
        }
    }
    let len = len.min(r.remaining());
    Ok((tag, r.take(len)?))
}

/// Parses the payload of an `esds` box: version and flags, then an ES_Descriptor (0x03) that holds a
/// DecoderConfigDescriptor (0x04) that holds the DecoderSpecificInfo (0x05).
pub(crate) fn parse_esds(payload: &[u8]) -> PResult<Esds> {
    let mut r = Reader::new(payload);
    r.skip(4)?;
    let (tag, es) = descriptor(&mut r)?;
    if tag != 0x03 {
        return Err(malformed("an esds box without an ES_Descriptor"));
    }
    let mut r = Reader::new(es);
    r.skip(2)?; // ES_ID
    let flags = r.u8()?;
    if flags & 0x80 != 0 {
        r.skip(2)?; // dependsOn_ES_ID
    }
    if flags & 0x40 != 0 {
        let n = usize::from(r.u8()?);
        r.skip(n)?; // URL
    }
    if flags & 0x20 != 0 {
        r.skip(2)?; // OCR_ES_Id
    }
    while r.remaining() > 0 {
        let (tag, body) = descriptor(&mut r)?;
        if tag != 0x04 {
            continue;
        }
        let mut d = Reader::new(body);
        let object_type = d.u8()?;
        d.skip(12)?; // streamType(1) bufferSizeDB(3) maxBitrate(4) avgBitrate(4)
        let mut asc = Vec::new();
        while d.remaining() > 0 {
            let (tag, body) = descriptor(&mut d)?;
            if tag == 0x05 {
                asc = body.to_vec();
                break;
            }
        }
        return Ok(Esds { object_type, asc });
    }
    Err(malformed("an esds box without a DecoderConfigDescriptor"))
}

/// The parts of an AudioSpecificConfig a decoder setup and the codec string need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AudioSpecificConfig {
    /// The audio object type (2 is AAC-LC, 5 is HE-AAC, 29 is HE-AAC v2).
    pub object_type: u32,
    /// Output sample rate (the extension rate for SBR).
    pub sample_rate: u32,
    /// 0 when the layout is in a program config element.
    pub channels: u32,
}

const ASC_RATES: [u32; 13] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
];

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn bits(&mut self, n: usize) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = *self.d.get(self.pos / 8)?;
            v = (v << 1) | u32::from((byte >> (7 - self.pos % 8)) & 1);
            self.pos += 1;
        }
        Some(v)
    }

    fn rate(&mut self) -> Option<u32> {
        let index = self.bits(4)?;
        if index == 15 {
            self.bits(24)
        } else {
            ASC_RATES.get(index as usize).copied()
        }
    }
}

pub(crate) fn parse_asc(asc: &[u8]) -> PResult<AudioSpecificConfig> {
    let bad = || malformed("an AudioSpecificConfig that is cut short or names a reserved value");
    let mut b = Bits { d: asc, pos: 0 };
    let mut object_type = b.bits(5).ok_or_else(bad)?;
    if object_type == 31 {
        object_type = 32 + b.bits(6).ok_or_else(bad)?;
    }
    let mut sample_rate = b.rate().ok_or_else(bad)?;
    let config = b.bits(4).ok_or_else(bad)?;
    let mut channels = match config {
        1..=6 => config,
        7 => 8,
        _ => 0,
    };
    if object_type == 5 || object_type == 29 {
        sample_rate = b.rate().ok_or_else(bad)?;
        if object_type == 29 {
            channels = 2;
        }
    }
    // An explicit 24-bit rate can be anything; 0 names no rate at all.
    if sample_rate == 0 {
        return Err(malformed("an AudioSpecificConfig with a sample rate of 0"));
    }
    check_audio_numbers(sample_rate, channels)?;
    Ok(AudioSpecificConfig {
        object_type,
        sample_rate,
        channels,
    })
}

/// `mp4a.40.<audioObjectType>` from an AudioSpecificConfig.
pub(crate) fn aac_codec_string(asc: &AudioSpecificConfig) -> String {
    format!("mp4a.40.{}", asc.object_type)
}

/// What an `OpusHead` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OpusHead {
    pub channels: u8,
    pub pre_skip: u16,
    pub input_rate: u32,
}

/// Checks an `OpusHead` (the Matroska `CodecPrivate`, or what [`dops_to_opus_head`] builds) and reads its
/// channel count, pre-skip and input rate.
pub(crate) fn parse_opus_head(head: &[u8]) -> PResult<OpusHead> {
    if head.len() < 19 || &head[..8] != b"OpusHead" {
        return Err(malformed(
            "an Opus identification header that is missing or too short",
        ));
    }
    if head[8] >> 4 != 0 {
        return Err(malformed(
            "an Opus identification header of an unknown version",
        ));
    }
    let channels = head[9];
    if channels == 0 {
        return Err(malformed("an Opus stream with no channels"));
    }
    let family = head[18];
    if family != 0 && head.len() < 21 + usize::from(channels) {
        return Err(malformed("an Opus channel mapping table that is cut short"));
    }
    let input_rate = u32::from_le_bytes([head[12], head[13], head[14], head[15]]);
    check_audio_numbers(input_rate, u32::from(channels))?;
    Ok(OpusHead {
        channels,
        pre_skip: u16::from_le_bytes([head[10], head[11]]),
        input_rate,
    })
}

/// Builds the `OpusHead` for a `dOps` box (ISO BMFF stores the same fields big-endian).
pub(crate) fn dops_to_opus_head(dops: &[u8]) -> PResult<Vec<u8>> {
    let mut r = Reader::new(dops);
    let version = r.u8()?;
    if version != 0 {
        return Err(malformed("a dOps box of an unknown version"));
    }
    let channels = r.u8()?;
    let pre_skip = r.u16()?;
    let rate = r.u32()?;
    let gain = r.u16()?;
    let family = r.u8()?;
    let mut head = Vec::with_capacity(19 + 2 + usize::from(channels));
    head.extend_from_slice(b"OpusHead");
    head.push(1);
    head.push(channels);
    head.extend_from_slice(&pre_skip.to_le_bytes());
    head.extend_from_slice(&rate.to_le_bytes());
    head.extend_from_slice(&gain.to_le_bytes());
    head.push(family);
    if family != 0 {
        head.extend_from_slice(r.take(2 + usize::from(channels))?);
    }
    parse_opus_head(&head)?;
    Ok(head)
}

/// The profile, level and bit depth a WebM VP9 `CodecPrivate` lists (a sequence of one-byte ID, one-byte
/// length, value: 1 profile, 2 level, 3 bit depth, 4 chroma subsampling).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Vp9Features {
    pub profile: Option<u8>,
    pub level: Option<u8>,
    pub bit_depth: Option<u8>,
}

pub(crate) fn parse_vp9_codec_private(p: &[u8]) -> Vp9Features {
    let mut out = Vp9Features::default();
    let mut i = 0;
    while i + 2 <= p.len() {
        let (id, len) = (p[i], usize::from(p[i + 1]));
        let Some(value) = p.get(i + 2..i + 2 + len) else {
            break;
        };
        if let [v] = value {
            match id {
                1 => out.profile = Some(*v),
                2 => out.level = Some(*v),
                3 => out.bit_depth = Some(*v),
                _ => {}
            }
        }
        i += 2 + len;
    }
    out
}

/// The RFC 6381 string of a WebM VP9 track: `vp09.PP.LL.DD` when its `CodecPrivate` gives all three,
/// else `vp9` ("any VP9": libvpx's muxer writes no `CodecPrivate`).
pub(crate) fn vp9_webm_codec_string(codec_private: &[u8]) -> String {
    let f = parse_vp9_codec_private(codec_private);
    match (f.profile, f.level, f.bit_depth) {
        (Some(p), Some(l), Some(d)) => format!("vp09.{p:02}.{l:02}.{d:02}"),
        _ => "vp9".to_string(),
    }
}

// ----------------------------------------------------------------------------------------------
// The codecs of lane med-1b (decision D7): HEVC, AC-3, E-AC-3, FLAC, Vorbis. Written from ISO/IEC
// 14496-15 (`hvcC`, Annex E codec strings), ETSI TS 102 366 Annex F (`dac3`, `dec3`), the "Encapsulation
// of FLAC in ISO Base Media File Format" (`dfLa`), the FLAC format specification (metadata blocks, the
// STREAMINFO layout) and the Matroska codec mapping (`A_FLAC`, `A_VORBIS` `CodecPrivate`, Xiph lacing).

/// `hvc1.<space><profile>.<compatibility>.<tier><level>[.<constraint byte>...]` from an `hvcC` record
/// (ISO/IEC 14496-15 Annex E): the general profile space (none, `A`, `B`, `C`), profile idc, the 32
/// compatibility flags in bit-reversed order as hex without leading zeros, the tier (`L` or `H`) and level
/// idc, then the six constraint indicator bytes as hex with the trailing zero bytes left out. `entry` is
/// the sample entry type (`hvc1` or `hev1`). The record is checked as far as a decoder needs it: at least
/// the 23 fixed bytes, version 1, and parameter set arrays that fit in the record.
pub(crate) fn hevc_codec_string(entry: &[u8; 4], hvcc: &[u8]) -> PResult<String> {
    let bad =
        || malformed("an hvcC record that is too short, not version 1 or cut inside its arrays");
    if hvcc.len() < 23 || hvcc[0] != 1 {
        return Err(bad());
    }
    let mut r = Reader::new(&hvcc[23..]);
    for _ in 0..hvcc[22] {
        r.skip(1).map_err(|_| bad())?; // array_completeness, reserved, NAL_unit_type
        let count = r.u16().map_err(|_| bad())?;
        for _ in 0..count {
            let len = r.u16().map_err(|_| bad())?;
            r.skip(usize::from(len)).map_err(|_| bad())?;
        }
    }
    let space = match hvcc[1] >> 6 {
        1 => "A",
        2 => "B",
        3 => "C",
        _ => "",
    };
    let profile = hvcc[1] & 0x1f;
    let tier = if hvcc[1] & 0x20 != 0 { 'H' } else { 'L' };
    let compatibility = u32::from_be_bytes([hvcc[2], hvcc[3], hvcc[4], hvcc[5]]).reverse_bits();
    let level = hvcc[12];
    let mut constraints = hvcc[6..12].to_vec();
    while constraints.last() == Some(&0) {
        constraints.pop();
    }
    let mut s = format!(
        "{}.{space}{profile}.{compatibility:X}.{tier}{level}",
        String::from_utf8_lossy(entry)
    );
    for c in constraints {
        s.push_str(&format!(".{c:X}"));
    }
    Ok(s)
}

/// What an AC-3 or E-AC-3 configuration box says about the audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ac3Info {
    pub sample_rate: u32,
    /// Full-bandwidth channels, plus the low-frequency effects channel(s).
    pub channels: u32,
}

const AC3_RATES: [u32; 3] = [48_000, 44_100, 32_000];
/// Full-bandwidth channels of each `acmod` (1+1 is two mono programs, counted as two channels).
const AC3_ACMOD_CHANNELS: [u32; 8] = [2, 1, 2, 3, 3, 4, 4, 5];

/// The payload of a `dac3` box (ETSI TS 102 366 F.4): `fscod`(2) `bsid`(5) `bsmod`(3) `acmod`(3)
/// `lfeon`(1) `bit_rate_code`(5) and five reserved bits.
pub(crate) fn parse_dac3(dac3: &[u8]) -> PResult<Ac3Info> {
    let bad = || malformed("a dac3 box that is too short or names a reserved sample rate");
    let mut b = Bits { d: dac3, pos: 0 };
    let fscod = b.bits(2).ok_or_else(bad)?;
    b.bits(5).ok_or_else(bad)?; // bsid
    b.bits(3).ok_or_else(bad)?; // bsmod
    let acmod = b.bits(3).ok_or_else(bad)?;
    let lfeon = b.bits(1).ok_or_else(bad)?;
    b.bits(10).ok_or_else(bad)?; // bit_rate_code and the reserved bits
    let sample_rate = *AC3_RATES.get(fscod as usize).ok_or_else(bad)?;
    Ok(Ac3Info {
        sample_rate,
        channels: AC3_ACMOD_CHANNELS[acmod as usize] + lfeon,
    })
}

/// The payload of a `dec3` box (ETSI TS 102 366 F.6): `data_rate`(13), `num_ind_sub`(3), then per
/// independent substream `fscod`(2) `bsid`(5) a reserved bit, `asvc`(1) `bsmod`(3) `acmod`(3) `lfeon`(1),
/// three reserved bits, `num_dep_sub`(4) and `chan_loc`(9) when there are dependent substreams (else one
/// reserved bit). Every substream is read, so a record that is cut anywhere is refused. The sample rate
/// and the channels are the first independent substream's: the channels of its dependent substreams
/// (`chan_loc`, which adds speakers beyond 5.1) are not counted, so a stream with them is better described
/// by its sample entry's channel count, which the parser prefers when it is not 0.
pub(crate) fn parse_dec3(dec3: &[u8]) -> PResult<Ac3Info> {
    let bad = || malformed("a dec3 box that is too short or names a reserved sample rate");
    let mut b = Bits { d: dec3, pos: 0 };
    b.bits(13).ok_or_else(bad)?; // data_rate
    let num_ind_sub = b.bits(3).ok_or_else(bad)?;
    let mut first = None;
    for i in 0..=num_ind_sub {
        let fscod = b.bits(2).ok_or_else(bad)?;
        b.bits(5).ok_or_else(bad)?; // bsid
        b.bits(2).ok_or_else(bad)?; // reserved, asvc
        b.bits(3).ok_or_else(bad)?; // bsmod
        let acmod = b.bits(3).ok_or_else(bad)?;
        let lfeon = b.bits(1).ok_or_else(bad)?;
        b.bits(3).ok_or_else(bad)?; // reserved
        let num_dep_sub = b.bits(4).ok_or_else(bad)?;
        // chan_loc (9 bits) with dependent substreams, else one reserved bit.
        b.bits(if num_dep_sub > 0 { 9 } else { 1 })
            .ok_or_else(bad)?;
        if i == 0 {
            let sample_rate = *AC3_RATES.get(fscod as usize).ok_or_else(bad)?;
            first = Some((sample_rate, AC3_ACMOD_CHANNELS[acmod as usize] + lfeon));
        }
    }
    let (sample_rate, channels) = first.ok_or_else(bad)?;
    Ok(Ac3Info {
        sample_rate,
        channels,
    })
}

/// What a FLAC STREAMINFO block says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FlacInfo {
    pub sample_rate: u32,
    pub channels: u32,
}

/// Checks the FLAC metadata blocks of a `dfLa` box or a Matroska `A_FLAC` `CodecPrivate` (after the
/// `fLaC` marker): each block has its 4-byte header (last-block flag, type, 24-bit length) and its data
/// within the record, the first is STREAMINFO (type 0, 34 bytes) and no block has the reserved type 127.
/// Reads the sample rate and channel count from STREAMINFO (an `fLaC` sample entry's own 16.16 rate field
/// cannot hold rates over 65,535 Hz, so STREAMINFO is the authority).
pub(crate) fn parse_flac_blocks(blocks: &[u8]) -> PResult<FlacInfo> {
    let bad = |why: &str| malformed(format!("FLAC metadata that {why}"));
    let mut r = Reader::new(blocks);
    let mut info = None;
    loop {
        let header = r.u32().map_err(|_| bad("is cut inside a block header"))?;
        let (last, kind, len) = (header >> 31 != 0, (header >> 24) & 0x7f, header & 0xff_ffff);
        let data = r
            .take(len as usize)
            .map_err(|_| bad("has a block that runs past the record"))?;
        if info.is_none() {
            if kind != 0 || len != 34 {
                return Err(bad("does not start with a 34-byte STREAMINFO block"));
            }
            let sample_rate =
                (u32::from(data[10]) << 12) | (u32::from(data[11]) << 4) | u32::from(data[12] >> 4);
            let channels = u32::from((data[12] >> 1) & 0x07) + 1;
            if sample_rate == 0 {
                return Err(bad("has a sample rate of 0"));
            }
            check_audio_numbers(sample_rate, channels)?;
            info = Some(FlacInfo {
                sample_rate,
                channels,
            });
        } else if kind == 127 {
            return Err(bad("has a block of the invalid type 127"));
        }
        if last || r.remaining() == 0 {
            break;
        }
    }
    info.ok_or_else(|| bad("has no STREAMINFO block"))
}

/// What the Vorbis identification header says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VorbisInfo {
    pub sample_rate: u32,
    pub channels: u32,
}

/// Checks a Matroska `A_VORBIS` `CodecPrivate` (the three header packets in Xiph lacing: a count byte
/// of 2, the sizes of the first two packets as runs of 255 closed by a smaller byte, then the packets)
/// and reads the channel count and sample rate from the identification header. The packets must have the
/// types 1, 3 and 5 with the `vorbis` signature, and their sizes must add up to the record.
pub(crate) fn parse_vorbis_codec_private(p: &[u8]) -> PResult<VorbisInfo> {
    let bad = |why: &str| malformed(format!("a Vorbis CodecPrivate that {why}"));
    if p.first() != Some(&2) {
        return Err(bad("does not announce three header packets"));
    }
    let mut at = 1usize;
    let mut sizes = [0usize; 2];
    for size in &mut sizes {
        loop {
            let b = *p.get(at).ok_or_else(|| bad("is cut inside its lacing"))?;
            at += 1;
            *size += usize::from(b);
            if b != 255 {
                break;
            }
        }
    }
    let ident = p
        .get(at..)
        .and_then(|d| d.get(..sizes[0]))
        .ok_or_else(|| bad("has an identification header that runs past the record"))?;
    let comment = p
        .get(at + sizes[0]..)
        .and_then(|d| d.get(..sizes[1]))
        .ok_or_else(|| bad("has a comment header that runs past the record"))?;
    let setup = p
        .get(at + sizes[0] + sizes[1]..)
        .ok_or_else(|| bad("is cut"))?;
    for (packet, kind) in [(ident, 1u8), (comment, 3), (setup, 5)] {
        if packet.len() < 7 || packet[0] != kind || &packet[1..7] != b"vorbis" {
            return Err(bad(
                "has a header packet that is missing or of the wrong type",
            ));
        }
    }
    if ident.len() < 30 {
        return Err(bad("has an identification header shorter than 30 bytes"));
    }
    let channels = u32::from(ident[11]);
    let sample_rate = u32::from_le_bytes([ident[12], ident[13], ident[14], ident[15]]);
    if channels == 0 || sample_rate == 0 {
        return Err(bad("has no channels or a sample rate of 0"));
    }
    check_audio_numbers(sample_rate, channels)?;
    Ok(VorbisInfo {
        sample_rate,
        channels,
    })
}

/// An `mdhd` language code (three letters of five bits each, offset 0x60) as a tag; `und` and 0 give an
/// empty string.
pub(crate) fn language_from_mdhd(code: u16) -> String {
    let letters: Vec<char> = (0..3)
        .map(|i| {
            let v = ((code >> (10 - 5 * i)) & 0x1f) as u8;
            char::from(0x60 + v)
        })
        .collect();
    let s: String = letters.into_iter().collect();
    if code == 0 || s == "und" || !s.chars().all(|c| c.is_ascii_lowercase()) {
        String::new()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avc_vp9_av1_strings() {
        let avcc = [1, 0x42, 0xC0, 0x0C, 0xFF, 0xE1, 0, 0];
        assert_eq!(avc_codec_string(b"avc1", &avcc).unwrap(), "avc1.42C00C");
        assert_eq!(avc_codec_string(b"avc3", &avcc).unwrap(), "avc3.42C00C");
        assert!(avc_codec_string(b"avc1", &[1, 2, 3]).is_err());
        assert!(avc_codec_string(b"avc1", &[2, 0x42, 0xC0, 0x0C, 0xFF, 0xE1, 0]).is_err());
        assert_eq!(
            vp9_codec_string(&[0, 11, 0x80, 0]).unwrap(),
            "vp09.00.11.08"
        );
        assert_eq!(vp9_codec_string(&[2, 51, 0xA0]).unwrap(), "vp09.02.51.10");
        assert!(vp9_codec_string(&[0, 1]).is_err());
        assert_eq!(
            av1_codec_string(&[0x81, 0x00, 0x0c, 0x00]).unwrap(),
            "av01.0.00M.08"
        );
        assert_eq!(
            av1_codec_string(&[0x81, 0x08, 0xcc, 0x00]).unwrap(),
            "av01.0.08H.10"
        );
        assert_eq!(
            av1_codec_string(&[0x81, 0x48, 0x6c, 0x00]).unwrap(),
            "av01.2.08M.12"
        );
        assert!(av1_codec_string(&[0x01, 0, 0, 0]).is_err());
        assert!(av1_codec_string(&[0x81, 0, 0]).is_err());
    }

    #[test]
    fn esds_gives_the_audio_specific_config() {
        // esds: version/flags, ES_Descriptor(ES_ID 1, flags 0) { DecoderConfig(0x40, ...) { DecSpecificInfo(0x12 0x08) } }
        let mut esds = vec![0, 0, 0, 0];
        let asc = [0x11, 0x88]; // AAC-LC, 48 kHz, mono
        let mut dcd = vec![0x40, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        dcd.extend_from_slice(&[0x05, asc.len() as u8]);
        dcd.extend_from_slice(&asc);
        let mut es = vec![0, 1, 0];
        es.extend_from_slice(&[0x04, dcd.len() as u8]);
        es.extend_from_slice(&dcd);
        esds.extend_from_slice(&[0x03, es.len() as u8]);
        esds.extend_from_slice(&es);
        let e = parse_esds(&esds).unwrap();
        assert_eq!(e.object_type, 0x40);
        assert_eq!(e.asc, asc);
        let c = parse_asc(&e.asc).unwrap();
        assert_eq!(
            c,
            AudioSpecificConfig {
                object_type: 2,
                sample_rate: 48_000,
                channels: 1
            }
        );
        assert_eq!(aac_codec_string(&c), "mp4a.40.2");
        // Cut anywhere: an error or a shorter parse, never a panic.
        for cut in 0..esds.len() {
            let _ = parse_esds(&esds[..cut]);
        }
        assert!(parse_esds(&[0, 0, 0, 0, 0x04, 0]).is_err());
    }

    /// Packs `(value, bit count)` fields into bytes, most significant bit first.
    fn pack(fields: &[(u32, usize)]) -> Vec<u8> {
        let mut bits = Vec::new();
        for &(v, n) in fields {
            for i in (0..n).rev() {
                bits.push((v >> i) & 1);
            }
        }
        bits.chunks(8)
            .map(|c| {
                c.iter()
                    .enumerate()
                    .fold(0u8, |a, (i, &b)| a | ((b as u8) << (7 - i)))
            })
            .collect()
    }

    #[test]
    fn asc_handles_sbr_escape_and_explicit_rates() {
        // HE-AAC: AOT 5, core rate 24 kHz (index 6), stereo, extension rate 48 kHz (index 3), AOT 2.
        let c = parse_asc(&pack(&[(5, 5), (6, 4), (2, 4), (3, 4), (2, 5)])).unwrap();
        assert_eq!((c.object_type, c.sample_rate, c.channels), (5, 48_000, 2));
        // HE-AAC v2: AOT 29 gives stereo whatever the core says.
        let c = parse_asc(&pack(&[(29, 5), (6, 4), (1, 4), (3, 4), (2, 5)])).unwrap();
        assert_eq!((c.object_type, c.channels), (29, 2));
        // AOT 31 is an escape: 32 + 2 = 34.
        let c = parse_asc(&pack(&[(31, 5), (2, 6), (3, 4), (1, 4)])).unwrap();
        assert_eq!((c.object_type, c.sample_rate, c.channels), (34, 48_000, 1));
        // An explicit 24-bit rate (index 15); channel configuration 7 is 7.1.
        let c = parse_asc(&pack(&[(2, 5), (15, 4), (44_100, 24), (7, 4)])).unwrap();
        assert_eq!((c.object_type, c.sample_rate, c.channels), (2, 44_100, 8));
        // Channel configuration 0 leaves the layout to the program config element.
        assert_eq!(
            parse_asc(&pack(&[(2, 5), (4, 4), (0, 4)]))
                .unwrap()
                .channels,
            0
        );
        // Empty, cut short, and a reserved frequency index.
        assert!(parse_asc(&[]).is_err());
        assert!(parse_asc(&pack(&[(2, 5)])).is_err());
        assert!(parse_asc(&pack(&[(2, 5), (14, 4), (1, 4)])).is_err());
    }

    #[test]
    fn opus_head_round_trips_through_dops() {
        let mut dops = vec![0, 2];
        dops.extend_from_slice(&312u16.to_be_bytes());
        dops.extend_from_slice(&48_000u32.to_be_bytes());
        dops.extend_from_slice(&0u16.to_be_bytes());
        dops.push(0);
        let head = dops_to_opus_head(&dops).unwrap();
        assert_eq!(&head[..8], b"OpusHead");
        assert_eq!(head.len(), 19);
        let h = parse_opus_head(&head).unwrap();
        assert_eq!((h.channels, h.pre_skip, h.input_rate), (2, 312, 48_000));
        assert_eq!(head[10..12], 312u16.to_le_bytes());
        // A mapping table is carried over.
        let mut dops = vec![0, 6];
        dops.extend_from_slice(&0u16.to_be_bytes());
        dops.extend_from_slice(&48_000u32.to_be_bytes());
        dops.extend_from_slice(&0u16.to_be_bytes());
        dops.push(1);
        dops.extend_from_slice(&[4, 2, 0, 4, 1, 2, 3, 5]);
        let head = dops_to_opus_head(&dops).unwrap();
        assert_eq!(head.len(), 19 + 2 + 6);
        // Wrong version, short table, no channels, bad magic.
        assert!(dops_to_opus_head(&[1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
        assert!(dops_to_opus_head(&dops[..12]).is_err());
        assert!(dops_to_opus_head(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
        assert!(parse_opus_head(b"NotOpusHeadAtAll...").is_err());
    }

    #[test]
    fn vp9_features_and_languages() {
        assert_eq!(
            vp9_webm_codec_string(&[1, 1, 0, 2, 1, 21, 3, 1, 8]),
            "vp09.00.21.08"
        );
        assert_eq!(vp9_webm_codec_string(&[1, 1, 0]), "vp9");
        assert_eq!(vp9_webm_codec_string(&[]), "vp9");
        assert_eq!(
            vp9_webm_codec_string(&[1, 9, 0]),
            "vp9",
            "a cut feature list gives the short form"
        );
        assert_eq!(language_from_mdhd(0x55C4), "", "und is no language");
        assert_eq!(language_from_mdhd(0), "");
        // 'eng' = e(5) n(14) g(7).
        assert_eq!(language_from_mdhd((5 << 10) | (14 << 5) | 7), "eng");
    }
}
