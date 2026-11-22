/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: new file for ED5 (lane txt-1). How a counter value is written as text.
//
// The algorithms are written from CSS Counter Styles Level 3,
// <https://drafts.csswg.org/css-counter-styles-3/>, and the table of predefined styles
// transcribes the normative definitions in sections 6 and 7 of that specification. The
// longhand East Asian algorithm (`cjk_ideographic`), the Ethiopic numeral algorithm
// (`ethiopic`), the length limit and the order of the steps of `represent` are translated from
// Gecko's layout/style/CounterStyleManager.cpp (MPL-2.0,
// https://github.com/mozilla-firefox/firefox/blob/main/layout/style/CounterStyleManager.cpp);
// the symbol tables were checked against Gecko's layout/style/res/counterstyles.css. No other
// project's code is copied here.
//
// Nothing in this file names a Stylo or Servo type, so it is compiled and tested on its own
// (`tests/counter_styles.rs` includes it by path).

/// The longest text, in characters, that `symbolic`, `additive` and `pad` may build before the
/// style gives up and its fallback is used. Gecko uses 150 UTF-16 units.
const LENGTH_LIMIT: usize = 150;

/// <https://drafts.csswg.org/css-counter-styles-3/#counter-style-system>
#[derive(Clone, Copy, Debug)]
pub(crate) enum Algorithm<'a> {
    /// The value 1 is the first symbol, then they repeat.
    Cyclic(&'a [&'a str]),
    /// The symbols are used once, starting at `first`; any other value uses the fallback.
    Fixed { first: i32, symbols: &'a [&'a str] },
    /// The symbol for the value is repeated as many times as the list has been run through.
    Symbolic(&'a [&'a str]),
    /// a, b, ..., z, aa, ab, ... (bijective numeration).
    Alphabetic(&'a [&'a str]),
    /// Positional notation with the symbols as digits; the first one is zero.
    Numeric(&'a [&'a str]),
    /// Symbols with weights, added up from the heaviest. Roman numerals are one.
    Additive(&'a [(u32, &'a str)]),
    /// The longhand East Asian styles (`japanese-formal`, `simp-chinese-informal`, ...).
    Cjk(&'a CjkData),
    /// `ethiopic-numeric`.
    Ethiopic,
}

impl Algorithm<'_> {
    /// <https://drafts.csswg.org/css-counter-styles-3/#counter-style-negative>: whether the
    /// negative sign is used. Without it the value itself is given to the algorithm.
    fn uses_negative_sign(&self) -> bool {
        !matches!(self, Algorithm::Cyclic(_) | Algorithm::Fixed { .. })
    }
}

/// Which style takes over when a style cannot write a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fallback {
    Decimal,
    CjkDecimal,
}

/// A counter style: an algorithm and the descriptors that go with it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CounterSystem<'a> {
    pub algorithm: Algorithm<'a>,
    /// The values the style is defined for, both ends included.
    pub range: (i32, i32),
    /// Written before and after a negative value.
    pub negative: (&'a str, &'a str),
    /// A minimum length in characters and the symbol that fills it up.
    pub pad: Option<(usize, &'a str)>,
    pub prefix: &'a str,
    pub suffix: &'a str,
    pub fallback: Fallback,
}

impl<'a> CounterSystem<'a> {
    /// A style with the initial value of every descriptor (`range: auto`).
    pub(crate) const fn new(algorithm: Algorithm<'a>) -> Self {
        let range = match algorithm {
            Algorithm::Cyclic(_)
            | Algorithm::Fixed { .. }
            | Algorithm::Numeric(_)
            | Algorithm::Cjk(_) => (i32::MIN, i32::MAX),
            Algorithm::Alphabetic(_) | Algorithm::Symbolic(_) | Algorithm::Ethiopic => {
                (1, i32::MAX)
            },
            Algorithm::Additive(_) => (0, i32::MAX),
        };
        Self {
            algorithm,
            range,
            negative: ("-", ""),
            pad: None,
            prefix: "",
            suffix: ". ",
            fallback: Fallback::Decimal,
        }
    }

    pub(crate) const fn with_range(self, low: i32, high: i32) -> Self {
        Self {
            range: (low, high),
            ..self
        }
    }

    pub(crate) const fn with_suffix(self, suffix: &'a str) -> Self {
        Self { suffix, ..self }
    }

    pub(crate) const fn with_pad(self, width: usize, symbol: &'a str) -> Self {
        Self {
            pad: Some((width, symbol)),
            ..self
        }
    }

    pub(crate) const fn with_negative(self, before: &'a str) -> Self {
        Self {
            negative: (before, ""),
            ..self
        }
    }

    pub(crate) const fn with_fallback(self, fallback: Fallback) -> Self {
        Self { fallback, ..self }
    }

    /// Whether the text this style gives changes with the value. A bullet does not, so a list
    /// that uses one never needs its numbers.
    pub(crate) fn depends_on_value(&self) -> bool {
        match self.algorithm {
            Algorithm::Cyclic(symbols) => symbols.len() > 1,
            _ => true,
        }
    }
}

/// `symbols()` takes one of these as its first argument.
/// <https://drafts.csswg.org/css-counter-styles-3/#symbols-function>
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SymbolsKind {
    Cyclic,
    Numeric,
    Alphabetic,
    Symbolic,
    Fixed,
}

/// The anonymous style that `symbols()` stands for: the descriptors have their initial value
/// except for the suffix, which is a space.
pub(crate) fn anonymous<'a>(kind: SymbolsKind, symbols: &'a [&'a str]) -> CounterSystem<'a> {
    let algorithm = match kind {
        SymbolsKind::Cyclic => Algorithm::Cyclic(symbols),
        SymbolsKind::Numeric => Algorithm::Numeric(symbols),
        SymbolsKind::Alphabetic => Algorithm::Alphabetic(symbols),
        SymbolsKind::Symbolic => Algorithm::Symbolic(symbols),
        SymbolsKind::Fixed => Algorithm::Fixed { first: 1, symbols },
    };
    CounterSystem::new(algorithm).with_suffix(" ")
}

/// <https://drafts.csswg.org/css-counter-styles-3/#generate-a-counter>, up to and including the
/// negative sign: `None` when the value is outside the range of the style or the style cannot
/// write it, in which case the caller uses the fallback (see [`generate`]).
pub(crate) fn represent(system: &CounterSystem<'_>, value: i32) -> Option<String> {
    if value < system.range.0 || value > system.range.1 {
        return None;
    }

    let negative = value < 0 && system.algorithm.uses_negative_sign();
    let ordinal = if system.algorithm.uses_negative_sign() {
        i64::from(value).abs()
    } else {
        i64::from(value)
    };
    let initial = initial_representation(&system.algorithm, ordinal)?;

    let (before, after) = if negative { system.negative } else { ("", "") };
    let mut result = String::from(before);
    if let Some((width, symbol)) = system.pad {
        let used = initial.chars().count() + before.chars().count() + after.chars().count();
        if let Some(difference) = width.checked_sub(used) {
            let symbol_length = symbol.chars().count();
            if difference > LENGTH_LIMIT
                || symbol_length > LENGTH_LIMIT
                || difference * symbol_length > LENGTH_LIMIT
            {
                return None;
            }
            for _ in 0..difference {
                result.push_str(symbol);
            }
        }
    }
    result.push_str(&initial);
    result.push_str(after);
    Some(result)
}

/// <https://drafts.csswg.org/css-counter-styles-3/#generate-a-counter>: the whole algorithm,
/// with the fallback styles. It always gives a string, because `decimal` can write every value.
pub(crate) fn generate(system: &CounterSystem<'_>, value: i32) -> String {
    if let Some(text) = represent(system, value) {
        return text;
    }
    // The fallback of `cjk-decimal` and of every style that falls back to it is `decimal`, so
    // this is at most two steps; a loop is not possible.
    if system.fallback == Fallback::CjkDecimal
        && let Some(cjk_decimal) = predefined("cjk-decimal")
        && let Some(text) = represent(&cjk_decimal, value)
    {
        return text;
    }
    decimal(value)
}

/// `decimal`.
pub(crate) fn decimal(value: i32) -> String {
    value.to_string()
}

fn initial_representation(algorithm: &Algorithm<'_>, ordinal: i64) -> Option<String> {
    match *algorithm {
        Algorithm::Cyclic(symbols) => cyclic(ordinal, symbols),
        Algorithm::Fixed { first, symbols } => fixed(ordinal, i64::from(first), symbols),
        Algorithm::Symbolic(symbols) => symbolic(ordinal, symbols),
        Algorithm::Alphabetic(symbols) => alphabetic(ordinal, symbols),
        Algorithm::Numeric(symbols) => numeric(ordinal, symbols),
        Algorithm::Additive(symbols) => additive(ordinal, symbols),
        Algorithm::Cjk(data) => u32::try_from(ordinal)
            .ok()
            .map(|ordinal| cjk_ideographic(ordinal, data)),
        Algorithm::Ethiopic => u32::try_from(ordinal).ok().and_then(ethiopic),
    }
}

/// <https://drafts.csswg.org/css-counter-styles-3/#cyclic-system>
fn cyclic(ordinal: i64, symbols: &[&str]) -> Option<String> {
    let count = i64::try_from(symbols.len())
        .ok()
        .filter(|count| *count > 0)?;
    let index = (ordinal - 1).rem_euclid(count);
    Some(symbols[index as usize].to_owned())
}

/// <https://drafts.csswg.org/css-counter-styles-3/#fixed-system>
fn fixed(ordinal: i64, first: i64, symbols: &[&str]) -> Option<String> {
    let index = usize::try_from(ordinal - first).ok()?;
    symbols.get(index).map(|symbol| (*symbol).to_owned())
}

/// <https://drafts.csswg.org/css-counter-styles-3/#symbolic-system>
fn symbolic(ordinal: i64, symbols: &[&str]) -> Option<String> {
    let count = u64::try_from(symbols.len())
        .ok()
        .filter(|count| *count > 0)?;
    let ordinal = u64::try_from(ordinal).ok().filter(|ordinal| *ordinal > 0)?;
    let symbol = symbols[((ordinal - 1) % count) as usize];
    let repeats = ordinal.div_ceil(count) as usize;
    let symbol_length = symbol.chars().count();
    if symbol_length > 0
        && (repeats > LENGTH_LIMIT
            || symbol_length > LENGTH_LIMIT
            || repeats * symbol_length > LENGTH_LIMIT)
    {
        return None;
    }
    Some(symbol.repeat(repeats))
}

/// <https://drafts.csswg.org/css-counter-styles-3/#alphabetic-system>
fn alphabetic(ordinal: i64, symbols: &[&str]) -> Option<String> {
    if symbols.len() < 2 {
        return None;
    }
    let count = symbols.len() as u64;
    let mut ordinal = u64::try_from(ordinal).ok().filter(|ordinal| *ordinal > 0)?;
    let mut indexes = Vec::new();
    while ordinal > 0 {
        ordinal -= 1;
        indexes.push((ordinal % count) as usize);
        ordinal /= count;
    }
    Some(indexes.iter().rev().map(|index| symbols[*index]).collect())
}

/// <https://drafts.csswg.org/css-counter-styles-3/#numeric-system>
fn numeric(ordinal: i64, symbols: &[&str]) -> Option<String> {
    if symbols.len() < 2 {
        return None;
    }
    let count = symbols.len() as u64;
    let mut ordinal = u64::try_from(ordinal).ok()?;
    if ordinal == 0 {
        return Some(symbols[0].to_owned());
    }
    let mut indexes = Vec::new();
    while ordinal > 0 {
        indexes.push((ordinal % count) as usize);
        ordinal /= count;
    }
    Some(indexes.iter().rev().map(|index| symbols[*index]).collect())
}

/// <https://drafts.csswg.org/css-counter-styles-3/#additive-system>
fn additive(ordinal: i64, symbols: &[(u32, &str)]) -> Option<String> {
    let mut ordinal = u64::try_from(ordinal).ok()?;
    if ordinal == 0 {
        // Only a table that ends with a symbol of weight zero can write zero.
        return match symbols.last() {
            Some((0, symbol)) => Some((*symbol).to_owned()),
            _ => None,
        };
    }

    let mut result = String::new();
    let mut length = 0;
    for (weight, symbol) in symbols {
        if *weight == 0 {
            break;
        }
        let times = ordinal / u64::from(*weight);
        if times == 0 {
            continue;
        }
        let symbol_length = symbol.chars().count();
        if symbol_length > 0 {
            length += times as usize * symbol_length;
            if times as usize > LENGTH_LIMIT
                || symbol_length > LENGTH_LIMIT
                || length > LENGTH_LIMIT
            {
                return None;
            }
            for _ in 0..times {
                result.push_str(symbol);
            }
        }
        ordinal -= times * u64::from(*weight);
    }
    (ordinal == 0).then_some(result)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CjkLang {
    Chinese,
    Korean,
    Japanese,
}

/// The characters of one of the longhand East Asian styles.
#[derive(Debug)]
pub(crate) struct CjkData {
    digit: [char; 10],
    /// Ten, hundred, thousand.
    unit: [char; 3],
    /// Ten thousand, hundred million (the next unit is the same one with a different value).
    unit_10k: [char; 2],
    lang: CjkLang,
    informal: bool,
}

static JAPANESE_INFORMAL: CjkData = CjkData {
    digit: [
        '\u{3007}', '\u{4e00}', '\u{4e8c}', '\u{4e09}', '\u{56db}', '\u{4e94}', '\u{516d}',
        '\u{4e03}', '\u{516b}', '\u{4e5d}',
    ],
    unit: ['\u{5341}', '\u{767e}', '\u{5343}'],
    unit_10k: ['\u{4e07}', '\u{5104}'],
    lang: CjkLang::Japanese,
    informal: true,
};

static JAPANESE_FORMAL: CjkData = CjkData {
    digit: [
        '\u{96f6}', '\u{58f1}', '\u{5f10}', '\u{53c2}', '\u{56db}', '\u{4f0d}', '\u{516d}',
        '\u{4e03}', '\u{516b}', '\u{4e5d}',
    ],
    unit: ['\u{62fe}', '\u{767e}', '\u{9621}'],
    unit_10k: ['\u{842c}', '\u{5104}'],
    lang: CjkLang::Japanese,
    informal: false,
};

static KOREAN_HANGUL_FORMAL: CjkData = CjkData {
    digit: [
        '\u{c601}', '\u{c77c}', '\u{c774}', '\u{c0bc}', '\u{c0ac}', '\u{c624}', '\u{c721}',
        '\u{ce60}', '\u{d314}', '\u{ad6c}',
    ],
    unit: ['\u{c2ed}', '\u{bc31}', '\u{cc9c}'],
    unit_10k: ['\u{b9cc}', '\u{c5b5}'],
    lang: CjkLang::Korean,
    informal: false,
};

static KOREAN_HANJA_INFORMAL: CjkData = CjkData {
    digit: [
        '\u{96f6}', '\u{4e00}', '\u{4e8c}', '\u{4e09}', '\u{56db}', '\u{4e94}', '\u{516d}',
        '\u{4e03}', '\u{516b}', '\u{4e5d}',
    ],
    unit: ['\u{5341}', '\u{767e}', '\u{5343}'],
    unit_10k: ['\u{842c}', '\u{5104}'],
    lang: CjkLang::Korean,
    informal: true,
};

static KOREAN_HANJA_FORMAL: CjkData = CjkData {
    digit: [
        '\u{96f6}', '\u{58f9}', '\u{8cb3}', '\u{53c3}', '\u{56db}', '\u{4e94}', '\u{516d}',
        '\u{4e03}', '\u{516b}', '\u{4e5d}',
    ],
    unit: ['\u{62fe}', '\u{767e}', '\u{4edf}'],
    unit_10k: ['\u{842c}', '\u{5104}'],
    lang: CjkLang::Korean,
    informal: false,
};

static SIMP_CHINESE_INFORMAL: CjkData = CjkData {
    digit: [
        '\u{96f6}', '\u{4e00}', '\u{4e8c}', '\u{4e09}', '\u{56db}', '\u{4e94}', '\u{516d}',
        '\u{4e03}', '\u{516b}', '\u{4e5d}',
    ],
    unit: ['\u{5341}', '\u{767e}', '\u{5343}'],
    unit_10k: ['\u{4e07}', '\u{4ebf}'],
    lang: CjkLang::Chinese,
    informal: true,
};

static SIMP_CHINESE_FORMAL: CjkData = CjkData {
    digit: [
        '\u{96f6}', '\u{58f9}', '\u{8d30}', '\u{53c1}', '\u{8086}', '\u{4f0d}', '\u{9646}',
        '\u{67d2}', '\u{634c}', '\u{7396}',
    ],
    unit: ['\u{62fe}', '\u{4f70}', '\u{4edf}'],
    unit_10k: ['\u{4e07}', '\u{4ebf}'],
    lang: CjkLang::Chinese,
    informal: false,
};

static TRAD_CHINESE_INFORMAL: CjkData = CjkData {
    digit: [
        '\u{96f6}', '\u{4e00}', '\u{4e8c}', '\u{4e09}', '\u{56db}', '\u{4e94}', '\u{516d}',
        '\u{4e03}', '\u{516b}', '\u{4e5d}',
    ],
    unit: ['\u{5341}', '\u{767e}', '\u{5343}'],
    unit_10k: ['\u{842c}', '\u{5104}'],
    lang: CjkLang::Chinese,
    informal: true,
};

static TRAD_CHINESE_FORMAL: CjkData = CjkData {
    digit: [
        '\u{96f6}', '\u{58f9}', '\u{8cb3}', '\u{53c3}', '\u{8086}', '\u{4f0d}', '\u{9678}',
        '\u{67d2}', '\u{634c}', '\u{7396}',
    ],
    unit: ['\u{62fe}', '\u{4f70}', '\u{4edf}'],
    unit_10k: ['\u{842c}', '\u{5104}'],
    lang: CjkLang::Chinese,
    informal: false,
};

/// <https://drafts.csswg.org/css-counter-styles-3/#complex-cjk>, over the extended range
/// (every value an `i32` can have): translated from Gecko's `CJKIdeographicToText`.
fn cjk_ideographic(mut ordinal: u32, data: &CjkData) -> String {
    // Built from the least significant digit, so it is reversed at the end.
    let mut reversed: Vec<char> = Vec::with_capacity(34);
    let mut need_zero = ordinal == 0;
    let mut position = 0usize;
    let mut unit_10k_index = 0usize;
    loop {
        let unit_index = position % 4;
        if unit_index == 0 {
            unit_10k_index = position / 4;
        }
        let current = (ordinal % 10) as usize;
        if current == 0 {
            if need_zero {
                need_zero = false;
                reversed.push(data.digit[0]);
            }
        } else {
            if data.lang == CjkLang::Chinese {
                need_zero = true;
            }
            if unit_10k_index != 0 {
                if data.lang == CjkLang::Korean && !reversed.is_empty() {
                    reversed.push(' ');
                }
                reversed.push(data.unit_10k[unit_10k_index - 1]);
            }
            if unit_index != 0 {
                reversed.push(data.unit[unit_index - 1]);
            }
            if current != 1 {
                reversed.push(data.digit[current]);
            } else {
                let mut need_one = true;
                if data.informal {
                    match data.lang {
                        CjkLang::Chinese => {
                            if unit_index == 1
                                && (ordinal == 1 || (position > 4 && ordinal % 1000 == 1))
                            {
                                need_one = false;
                            }
                        },
                        CjkLang::Japanese => {
                            if unit_index > 0
                                && (unit_index != 3 || (position == 3 && ordinal == 1))
                            {
                                need_one = false;
                            }
                        },
                        CjkLang::Korean => {
                            if unit_index > 0 || (position == 4 && ordinal % 1000 == 1) {
                                need_one = false;
                            }
                        },
                    }
                }
                if need_one {
                    reversed.push(data.digit[1]);
                }
            }
            unit_10k_index = 0;
        }
        ordinal /= 10;
        position += 1;
        if ordinal == 0 {
            break;
        }
    }
    reversed.iter().rev().collect()
}

/// <https://drafts.csswg.org/css-counter-styles-3/#ethiopic-numeric-counter-style>: translated
/// from Gecko's `EthiopicToText`. Only positive values have a representation.
fn ethiopic(ordinal: u32) -> Option<String> {
    const ONE: u32 = 0x1369;
    const TEN: u32 = 0x1372;
    const HUNDRED: char = '\u{137B}';
    const TEN_THOUSAND: char = '\u{137C}';

    if ordinal < 1 {
        return None;
    }
    let mut digits = ordinal.to_string();
    if digits.len() % 2 == 1 {
        digits.insert(0, '0');
    }
    let groups = digits.len() / 2;
    let mut result = String::new();
    for (index_from_left, pair) in digits.as_bytes().chunks(2).enumerate() {
        let group_index_from_right = groups - 1 - index_from_left;
        let tens = u32::from(pair[0] - b'0');
        let mut units = u32::from(pair[1] - b'0');
        let group_value = tens * 10 + units;
        let odd_group = group_index_from_right % 2 == 1;

        // A "one" without a tens digit is dropped when it comes before a hundred (an odd group)
        // or starts the number.
        if ordinal > 1 && group_value == 1 && (odd_group || index_from_left == 0) {
            units = 0;
        }
        if tens != 0 {
            result.extend(char::from_u32(TEN + tens - 1));
        }
        if units != 0 {
            result.extend(char::from_u32(ONE + units - 1));
        }
        if odd_group {
            if group_value != 0 {
                result.push(HUNDRED);
            }
        } else if group_index_from_right != 0 {
            result.push(TEN_THOUSAND);
        }
    }
    Some(result)
}

const DECIMAL_DIGITS: &[&str] = &["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

const ARABIC_INDIC: &[&str] = &[
    "\u{660}", "\u{661}", "\u{662}", "\u{663}", "\u{664}", "\u{665}", "\u{666}", "\u{667}",
    "\u{668}", "\u{669}",
];

const BENGALI: &[&str] = &[
    "\u{9E6}", "\u{9E7}", "\u{9E8}", "\u{9E9}", "\u{9EA}", "\u{9EB}", "\u{9EC}", "\u{9ED}",
    "\u{9EE}", "\u{9EF}",
];

const KHMER: &[&str] = &[
    "\u{17E0}", "\u{17E1}", "\u{17E2}", "\u{17E3}", "\u{17E4}", "\u{17E5}", "\u{17E6}", "\u{17E7}",
    "\u{17E8}", "\u{17E9}",
];

const CJK_DECIMAL: &[&str] = &[
    "\u{3007}", "\u{4E00}", "\u{4E8C}", "\u{4E09}", "\u{56DB}", "\u{4E94}", "\u{516D}", "\u{4E03}",
    "\u{516B}", "\u{4E5D}",
];

const DEVANAGARI: &[&str] = &[
    "\u{966}", "\u{967}", "\u{968}", "\u{969}", "\u{96A}", "\u{96B}", "\u{96C}", "\u{96D}",
    "\u{96E}", "\u{96F}",
];

const GUJARATI: &[&str] = &[
    "\u{AE6}", "\u{AE7}", "\u{AE8}", "\u{AE9}", "\u{AEA}", "\u{AEB}", "\u{AEC}", "\u{AED}",
    "\u{AEE}", "\u{AEF}",
];

const GURMUKHI: &[&str] = &[
    "\u{A66}", "\u{A67}", "\u{A68}", "\u{A69}", "\u{A6A}", "\u{A6B}", "\u{A6C}", "\u{A6D}",
    "\u{A6E}", "\u{A6F}",
];

const KANNADA: &[&str] = &[
    "\u{CE6}", "\u{CE7}", "\u{CE8}", "\u{CE9}", "\u{CEA}", "\u{CEB}", "\u{CEC}", "\u{CED}",
    "\u{CEE}", "\u{CEF}",
];

const LAO: &[&str] = &[
    "\u{ED0}", "\u{ED1}", "\u{ED2}", "\u{ED3}", "\u{ED4}", "\u{ED5}", "\u{ED6}", "\u{ED7}",
    "\u{ED8}", "\u{ED9}",
];

const MALAYALAM: &[&str] = &[
    "\u{D66}", "\u{D67}", "\u{D68}", "\u{D69}", "\u{D6A}", "\u{D6B}", "\u{D6C}", "\u{D6D}",
    "\u{D6E}", "\u{D6F}",
];

const MONGOLIAN: &[&str] = &[
    "\u{1810}", "\u{1811}", "\u{1812}", "\u{1813}", "\u{1814}", "\u{1815}", "\u{1816}", "\u{1817}",
    "\u{1818}", "\u{1819}",
];

const MYANMAR: &[&str] = &[
    "\u{1040}", "\u{1041}", "\u{1042}", "\u{1043}", "\u{1044}", "\u{1045}", "\u{1046}", "\u{1047}",
    "\u{1048}", "\u{1049}",
];

const ORIYA: &[&str] = &[
    "\u{B66}", "\u{B67}", "\u{B68}", "\u{B69}", "\u{B6A}", "\u{B6B}", "\u{B6C}", "\u{B6D}",
    "\u{B6E}", "\u{B6F}",
];

const PERSIAN: &[&str] = &[
    "\u{6F0}", "\u{6F1}", "\u{6F2}", "\u{6F3}", "\u{6F4}", "\u{6F5}", "\u{6F6}", "\u{6F7}",
    "\u{6F8}", "\u{6F9}",
];

const TAMIL: &[&str] = &[
    "\u{BE6}", "\u{BE7}", "\u{BE8}", "\u{BE9}", "\u{BEA}", "\u{BEB}", "\u{BEC}", "\u{BED}",
    "\u{BEE}", "\u{BEF}",
];

const TELUGU: &[&str] = &[
    "\u{C66}", "\u{C67}", "\u{C68}", "\u{C69}", "\u{C6A}", "\u{C6B}", "\u{C6C}", "\u{C6D}",
    "\u{C6E}", "\u{C6F}",
];

const THAI: &[&str] = &[
    "\u{E50}", "\u{E51}", "\u{E52}", "\u{E53}", "\u{E54}", "\u{E55}", "\u{E56}", "\u{E57}",
    "\u{E58}", "\u{E59}",
];

const TIBETAN: &[&str] = &[
    "\u{F20}", "\u{F21}", "\u{F22}", "\u{F23}", "\u{F24}", "\u{F25}", "\u{F26}", "\u{F27}",
    "\u{F28}", "\u{F29}",
];

const LOWER_ALPHA: &[&str] = &[
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s",
    "t", "u", "v", "w", "x", "y", "z",
];

const UPPER_ALPHA: &[&str] = &[
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S",
    "T", "U", "V", "W", "X", "Y", "Z",
];

const LOWER_GREEK: &[&str] = &[
    "\u{3B1}", "\u{3B2}", "\u{3B3}", "\u{3B4}", "\u{3B5}", "\u{3B6}", "\u{3B7}", "\u{3B8}",
    "\u{3B9}", "\u{3BA}", "\u{3BB}", "\u{3BC}", "\u{3BD}", "\u{3BE}", "\u{3BF}", "\u{3C0}",
    "\u{3C1}", "\u{3C3}", "\u{3C4}", "\u{3C5}", "\u{3C6}", "\u{3C7}", "\u{3C8}", "\u{3C9}",
];

const HIRAGANA: &[&str] = &[
    "\u{3042}", "\u{3044}", "\u{3046}", "\u{3048}", "\u{304A}", "\u{304B}", "\u{304D}", "\u{304F}",
    "\u{3051}", "\u{3053}", "\u{3055}", "\u{3057}", "\u{3059}", "\u{305B}", "\u{305D}", "\u{305F}",
    "\u{3061}", "\u{3064}", "\u{3066}", "\u{3068}", "\u{306A}", "\u{306B}", "\u{306C}", "\u{306D}",
    "\u{306E}", "\u{306F}", "\u{3072}", "\u{3075}", "\u{3078}", "\u{307B}", "\u{307E}", "\u{307F}",
    "\u{3080}", "\u{3081}", "\u{3082}", "\u{3084}", "\u{3086}", "\u{3088}", "\u{3089}", "\u{308A}",
    "\u{308B}", "\u{308C}", "\u{308D}", "\u{308F}", "\u{3090}", "\u{3091}", "\u{3092}", "\u{3093}",
];

const HIRAGANA_IROHA: &[&str] = &[
    "\u{3044}", "\u{308D}", "\u{306F}", "\u{306B}", "\u{307B}", "\u{3078}", "\u{3068}", "\u{3061}",
    "\u{308A}", "\u{306C}", "\u{308B}", "\u{3092}", "\u{308F}", "\u{304B}", "\u{3088}", "\u{305F}",
    "\u{308C}", "\u{305D}", "\u{3064}", "\u{306D}", "\u{306A}", "\u{3089}", "\u{3080}", "\u{3046}",
    "\u{3090}", "\u{306E}", "\u{304A}", "\u{304F}", "\u{3084}", "\u{307E}", "\u{3051}", "\u{3075}",
    "\u{3053}", "\u{3048}", "\u{3066}", "\u{3042}", "\u{3055}", "\u{304D}", "\u{3086}", "\u{3081}",
    "\u{307F}", "\u{3057}", "\u{3091}", "\u{3072}", "\u{3082}", "\u{305B}", "\u{3059}",
];

const KATAKANA: &[&str] = &[
    "\u{30A2}", "\u{30A4}", "\u{30A6}", "\u{30A8}", "\u{30AA}", "\u{30AB}", "\u{30AD}", "\u{30AF}",
    "\u{30B1}", "\u{30B3}", "\u{30B5}", "\u{30B7}", "\u{30B9}", "\u{30BB}", "\u{30BD}", "\u{30BF}",
    "\u{30C1}", "\u{30C4}", "\u{30C6}", "\u{30C8}", "\u{30CA}", "\u{30CB}", "\u{30CC}", "\u{30CD}",
    "\u{30CE}", "\u{30CF}", "\u{30D2}", "\u{30D5}", "\u{30D8}", "\u{30DB}", "\u{30DE}", "\u{30DF}",
    "\u{30E0}", "\u{30E1}", "\u{30E2}", "\u{30E4}", "\u{30E6}", "\u{30E8}", "\u{30E9}", "\u{30EA}",
    "\u{30EB}", "\u{30EC}", "\u{30ED}", "\u{30EF}", "\u{30F0}", "\u{30F1}", "\u{30F2}", "\u{30F3}",
];

const KATAKANA_IROHA: &[&str] = &[
    "\u{30A4}", "\u{30ED}", "\u{30CF}", "\u{30CB}", "\u{30DB}", "\u{30D8}", "\u{30C8}", "\u{30C1}",
    "\u{30EA}", "\u{30CC}", "\u{30EB}", "\u{30F2}", "\u{30EF}", "\u{30AB}", "\u{30E8}", "\u{30BF}",
    "\u{30EC}", "\u{30BD}", "\u{30C4}", "\u{30CD}", "\u{30CA}", "\u{30E9}", "\u{30E0}", "\u{30A6}",
    "\u{30F0}", "\u{30CE}", "\u{30AA}", "\u{30AF}", "\u{30E4}", "\u{30DE}", "\u{30B1}", "\u{30D5}",
    "\u{30B3}", "\u{30A8}", "\u{30C6}", "\u{30A2}", "\u{30B5}", "\u{30AD}", "\u{30E6}", "\u{30E1}",
    "\u{30DF}", "\u{30B7}", "\u{30F1}", "\u{30D2}", "\u{30E2}", "\u{30BB}", "\u{30B9}",
];

const CJK_HEAVENLY_STEM: &[&str] = &[
    "\u{7532}", "\u{4E59}", "\u{4E19}", "\u{4E01}", "\u{620A}", "\u{5DF1}", "\u{5E9A}", "\u{8F9B}",
    "\u{58EC}", "\u{7678}",
];

const CJK_EARTHLY_BRANCH: &[&str] = &[
    "\u{5B50}", "\u{4E11}", "\u{5BC5}", "\u{536F}", "\u{8FB0}", "\u{5DF3}", "\u{5348}", "\u{672A}",
    "\u{7533}", "\u{9149}", "\u{620C}", "\u{4EA5}",
];

const ARMENIAN: &[(u32, &str)] = &[
    (9000, "\u{554}"),
    (8000, "\u{553}"),
    (7000, "\u{552}"),
    (6000, "\u{551}"),
    (5000, "\u{550}"),
    (4000, "\u{54F}"),
    (3000, "\u{54E}"),
    (2000, "\u{54D}"),
    (1000, "\u{54C}"),
    (900, "\u{54B}"),
    (800, "\u{54A}"),
    (700, "\u{549}"),
    (600, "\u{548}"),
    (500, "\u{547}"),
    (400, "\u{546}"),
    (300, "\u{545}"),
    (200, "\u{544}"),
    (100, "\u{543}"),
    (90, "\u{542}"),
    (80, "\u{541}"),
    (70, "\u{540}"),
    (60, "\u{53F}"),
    (50, "\u{53E}"),
    (40, "\u{53D}"),
    (30, "\u{53C}"),
    (20, "\u{53B}"),
    (10, "\u{53A}"),
    (9, "\u{539}"),
    (8, "\u{538}"),
    (7, "\u{537}"),
    (6, "\u{536}"),
    (5, "\u{535}"),
    (4, "\u{534}"),
    (3, "\u{533}"),
    (2, "\u{532}"),
    (1, "\u{531}"),
];

const LOWER_ARMENIAN: &[(u32, &str)] = &[
    (9000, "\u{584}"),
    (8000, "\u{583}"),
    (7000, "\u{582}"),
    (6000, "\u{581}"),
    (5000, "\u{580}"),
    (4000, "\u{57F}"),
    (3000, "\u{57E}"),
    (2000, "\u{57D}"),
    (1000, "\u{57C}"),
    (900, "\u{57B}"),
    (800, "\u{57A}"),
    (700, "\u{579}"),
    (600, "\u{578}"),
    (500, "\u{577}"),
    (400, "\u{576}"),
    (300, "\u{575}"),
    (200, "\u{574}"),
    (100, "\u{573}"),
    (90, "\u{572}"),
    (80, "\u{571}"),
    (70, "\u{570}"),
    (60, "\u{56F}"),
    (50, "\u{56E}"),
    (40, "\u{56D}"),
    (30, "\u{56C}"),
    (20, "\u{56B}"),
    (10, "\u{56A}"),
    (9, "\u{569}"),
    (8, "\u{568}"),
    (7, "\u{567}"),
    (6, "\u{566}"),
    (5, "\u{565}"),
    (4, "\u{564}"),
    (3, "\u{563}"),
    (2, "\u{562}"),
    (1, "\u{561}"),
];

const GEORGIAN: &[(u32, &str)] = &[
    (10000, "\u{10F5}"),
    (9000, "\u{10F0}"),
    (8000, "\u{10EF}"),
    (7000, "\u{10F4}"),
    (6000, "\u{10EE}"),
    (5000, "\u{10ED}"),
    (4000, "\u{10EC}"),
    (3000, "\u{10EB}"),
    (2000, "\u{10EA}"),
    (1000, "\u{10E9}"),
    (900, "\u{10E8}"),
    (800, "\u{10E7}"),
    (700, "\u{10E6}"),
    (600, "\u{10E5}"),
    (500, "\u{10E4}"),
    (400, "\u{10F3}"),
    (300, "\u{10E2}"),
    (200, "\u{10E1}"),
    (100, "\u{10E0}"),
    (90, "\u{10DF}"),
    (80, "\u{10DE}"),
    (70, "\u{10DD}"),
    (60, "\u{10F2}"),
    (50, "\u{10DC}"),
    (40, "\u{10DB}"),
    (30, "\u{10DA}"),
    (20, "\u{10D9}"),
    (10, "\u{10D8}"),
    (9, "\u{10D7}"),
    (8, "\u{10F1}"),
    (7, "\u{10D6}"),
    (6, "\u{10D5}"),
    (5, "\u{10D4}"),
    (4, "\u{10D3}"),
    (3, "\u{10D2}"),
    (2, "\u{10D1}"),
    (1, "\u{10D0}"),
];

const HEBREW: &[(u32, &str)] = &[
    (10000, "\u{5D9}\u{5F3}"),
    (9000, "\u{5D8}\u{5F3}"),
    (8000, "\u{5D7}\u{5F3}"),
    (7000, "\u{5D6}\u{5F3}"),
    (6000, "\u{5D5}\u{5F3}"),
    (5000, "\u{5D4}\u{5F3}"),
    (4000, "\u{5D3}\u{5F3}"),
    (3000, "\u{5D2}\u{5F3}"),
    (2000, "\u{5D1}\u{5F3}"),
    (1000, "\u{5D0}\u{5F3}"),
    (400, "\u{5EA}"),
    (300, "\u{5E9}"),
    (200, "\u{5E8}"),
    (100, "\u{5E7}"),
    (90, "\u{5E6}"),
    (80, "\u{5E4}"),
    (70, "\u{5E2}"),
    (60, "\u{5E1}"),
    (50, "\u{5E0}"),
    (40, "\u{5DE}"),
    (30, "\u{5DC}"),
    (20, "\u{5DB}"),
    (19, "\u{5D9}\u{5D8}"),
    (18, "\u{5D9}\u{5D7}"),
    (17, "\u{5D9}\u{5D6}"),
    (16, "\u{5D8}\u{5D6}"),
    (15, "\u{5D8}\u{5D5}"),
    (10, "\u{5D9}"),
    (9, "\u{5D8}"),
    (8, "\u{5D7}"),
    (7, "\u{5D6}"),
    (6, "\u{5D5}"),
    (5, "\u{5D4}"),
    (4, "\u{5D3}"),
    (3, "\u{5D2}"),
    (2, "\u{5D1}"),
    (1, "\u{5D0}"),
];

const LOWER_ROMAN: &[(u32, &str)] = &[
    (1000, "m"),
    (900, "cm"),
    (500, "d"),
    (400, "cd"),
    (100, "c"),
    (90, "xc"),
    (50, "l"),
    (40, "xl"),
    (10, "x"),
    (9, "ix"),
    (5, "v"),
    (4, "iv"),
    (1, "i"),
];

const UPPER_ROMAN: &[(u32, &str)] = &[
    (1000, "M"),
    (900, "CM"),
    (500, "D"),
    (400, "CD"),
    (100, "C"),
    (90, "XC"),
    (50, "L"),
    (40, "XL"),
    (10, "X"),
    (9, "IX"),
    (5, "V"),
    (4, "IV"),
    (1, "I"),
];

const DISC: &[&str] = &["\u{2022}"];
const CIRCLE: &[&str] = &["\u{25E6}"];
const SQUARE: &[&str] = &["\u{25AA}"];
// TODO: Use U+25C2 "◂" and the vertical forms depending on the writing mode, as the
// specification asks for; `represent` does not know the direction yet.
const DISCLOSURE_OPEN: &[&str] = &["\u{25BE}"];
const DISCLOSURE_CLOSED: &[&str] = &["\u{25B8}"];

const IDEOGRAPHIC_COMMA: &str = "\u{3001}";
const JAPANESE_NEGATIVE: &str = "\u{30de}\u{30a4}\u{30ca}\u{30b9}";
const KOREAN_NEGATIVE: &str = "\u{b9c8}\u{c774}\u{b108}\u{c2a4} ";
const SIMP_CHINESE_NEGATIVE: &str = "\u{8d1f}";
const TRAD_CHINESE_NEGATIVE: &str = "\u{8ca0}";

const fn numeric_style(digits: &'static [&'static str]) -> CounterSystem<'static> {
    CounterSystem::new(Algorithm::Numeric(digits))
}

const fn alphabetic_style(symbols: &'static [&'static str]) -> CounterSystem<'static> {
    CounterSystem::new(Algorithm::Alphabetic(symbols))
}

const fn bullet(symbols: &'static [&'static str]) -> CounterSystem<'static> {
    CounterSystem::new(Algorithm::Cyclic(symbols)).with_suffix(" ")
}

const fn cjk_style(
    data: &'static CjkData,
    negative: &'static str,
    suffix: &'static str,
) -> CounterSystem<'static> {
    CounterSystem::new(Algorithm::Cjk(data))
        .with_negative(negative)
        .with_suffix(suffix)
        .with_fallback(Fallback::CjkDecimal)
}

/// Every counter style that CSS Counter Styles 3 predefines (the names Stylo accepts for
/// `list-style-type` and `counter()`), by its lowercase name. `None` for any other name:
/// `@counter-style` is not supported, so the caller falls back to `decimal`.
/// <https://drafts.csswg.org/css-counter-styles-3/#predefined-counters>
pub(crate) fn predefined(name: &str) -> Option<CounterSystem<'static>> {
    Some(match name {
        "decimal" => numeric_style(DECIMAL_DIGITS),
        "decimal-leading-zero" => numeric_style(DECIMAL_DIGITS).with_pad(2, "0"),
        "arabic-indic" => numeric_style(ARABIC_INDIC),
        "armenian" | "upper-armenian" => {
            CounterSystem::new(Algorithm::Additive(ARMENIAN)).with_range(1, 9999)
        },
        "lower-armenian" => {
            CounterSystem::new(Algorithm::Additive(LOWER_ARMENIAN)).with_range(1, 9999)
        },
        "bengali" => numeric_style(BENGALI),
        "cambodian" | "khmer" => numeric_style(KHMER),
        "cjk-decimal" => numeric_style(CJK_DECIMAL)
            .with_range(0, i32::MAX)
            .with_suffix(IDEOGRAPHIC_COMMA),
        "devanagari" => numeric_style(DEVANAGARI),
        "georgian" => CounterSystem::new(Algorithm::Additive(GEORGIAN)).with_range(1, 19999),
        "gujarati" => numeric_style(GUJARATI),
        "gurmukhi" => numeric_style(GURMUKHI),
        "hebrew" => CounterSystem::new(Algorithm::Additive(HEBREW)).with_range(1, 10999),
        "kannada" => numeric_style(KANNADA),
        "lao" => numeric_style(LAO),
        "malayalam" => numeric_style(MALAYALAM),
        "mongolian" => numeric_style(MONGOLIAN),
        "myanmar" => numeric_style(MYANMAR),
        "oriya" => numeric_style(ORIYA),
        "persian" => numeric_style(PERSIAN),
        "lower-roman" => CounterSystem::new(Algorithm::Additive(LOWER_ROMAN)).with_range(1, 3999),
        "upper-roman" => CounterSystem::new(Algorithm::Additive(UPPER_ROMAN)).with_range(1, 3999),
        "tamil" => numeric_style(TAMIL),
        "telugu" => numeric_style(TELUGU),
        "thai" => numeric_style(THAI),
        "tibetan" => numeric_style(TIBETAN),

        "lower-alpha" | "lower-latin" => alphabetic_style(LOWER_ALPHA),
        "upper-alpha" | "upper-latin" => alphabetic_style(UPPER_ALPHA),
        "lower-greek" => alphabetic_style(LOWER_GREEK),
        "hiragana" => alphabetic_style(HIRAGANA).with_suffix(IDEOGRAPHIC_COMMA),
        "hiragana-iroha" => alphabetic_style(HIRAGANA_IROHA).with_suffix(IDEOGRAPHIC_COMMA),
        "katakana" => alphabetic_style(KATAKANA).with_suffix(IDEOGRAPHIC_COMMA),
        "katakana-iroha" => alphabetic_style(KATAKANA_IROHA).with_suffix(IDEOGRAPHIC_COMMA),
        "cjk-heavenly-stem" => CounterSystem::new(Algorithm::Fixed {
            first: 1,
            symbols: CJK_HEAVENLY_STEM,
        })
        .with_suffix(IDEOGRAPHIC_COMMA)
        .with_fallback(Fallback::CjkDecimal),
        "cjk-earthly-branch" => CounterSystem::new(Algorithm::Fixed {
            first: 1,
            symbols: CJK_EARTHLY_BRANCH,
        })
        .with_suffix(IDEOGRAPHIC_COMMA)
        .with_fallback(Fallback::CjkDecimal),

        "disc" => bullet(DISC),
        "circle" => bullet(CIRCLE),
        "square" => bullet(SQUARE),
        "disclosure-open" => bullet(DISCLOSURE_OPEN),
        "disclosure-closed" => bullet(DISCLOSURE_CLOSED),

        "japanese-informal" => cjk_style(&JAPANESE_INFORMAL, JAPANESE_NEGATIVE, IDEOGRAPHIC_COMMA),
        "japanese-formal" => cjk_style(&JAPANESE_FORMAL, JAPANESE_NEGATIVE, IDEOGRAPHIC_COMMA),
        "korean-hangul-formal" => cjk_style(&KOREAN_HANGUL_FORMAL, KOREAN_NEGATIVE, ", "),
        "korean-hanja-informal" => cjk_style(&KOREAN_HANJA_INFORMAL, KOREAN_NEGATIVE, ", "),
        "korean-hanja-formal" => cjk_style(&KOREAN_HANJA_FORMAL, KOREAN_NEGATIVE, ", "),
        "simp-chinese-informal" => cjk_style(
            &SIMP_CHINESE_INFORMAL,
            SIMP_CHINESE_NEGATIVE,
            IDEOGRAPHIC_COMMA,
        ),
        "simp-chinese-formal" => cjk_style(
            &SIMP_CHINESE_FORMAL,
            SIMP_CHINESE_NEGATIVE,
            IDEOGRAPHIC_COMMA,
        ),
        "trad-chinese-informal" | "cjk-ideographic" => cjk_style(
            &TRAD_CHINESE_INFORMAL,
            TRAD_CHINESE_NEGATIVE,
            IDEOGRAPHIC_COMMA,
        ),
        "trad-chinese-formal" => cjk_style(
            &TRAD_CHINESE_FORMAL,
            TRAD_CHINESE_NEGATIVE,
            IDEOGRAPHIC_COMMA,
        ),
        "ethiopic-numeric" => CounterSystem::new(Algorithm::Ethiopic).with_suffix("/ "),

        _ => return None,
    })
}

/// The names `predefined` knows (the tests go through all of them).
#[allow(dead_code)]
pub(crate) const PREDEFINED_NAMES: &[&str] = &[
    "decimal",
    "decimal-leading-zero",
    "arabic-indic",
    "armenian",
    "upper-armenian",
    "lower-armenian",
    "bengali",
    "cambodian",
    "khmer",
    "cjk-decimal",
    "devanagari",
    "georgian",
    "gujarati",
    "gurmukhi",
    "hebrew",
    "kannada",
    "lao",
    "malayalam",
    "mongolian",
    "myanmar",
    "oriya",
    "persian",
    "lower-roman",
    "upper-roman",
    "tamil",
    "telugu",
    "thai",
    "tibetan",
    "lower-alpha",
    "lower-latin",
    "upper-alpha",
    "upper-latin",
    "cjk-earthly-branch",
    "cjk-heavenly-stem",
    "lower-greek",
    "hiragana",
    "hiragana-iroha",
    "katakana",
    "katakana-iroha",
    "disc",
    "circle",
    "square",
    "disclosure-open",
    "disclosure-closed",
    "japanese-informal",
    "japanese-formal",
    "korean-hangul-formal",
    "korean-hanja-informal",
    "korean-hanja-formal",
    "simp-chinese-informal",
    "simp-chinese-formal",
    "trad-chinese-informal",
    "trad-chinese-formal",
    "cjk-ideographic",
    "ethiopic-numeric",
];
