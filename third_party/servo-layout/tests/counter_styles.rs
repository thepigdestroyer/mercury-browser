/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: new file for ED5 (lane txt-1). Tests of `counter_styles.rs`, which names no Stylo or
// Servo type and is therefore included by path: this test needs no engine at all.
//
// The expected texts come from CSS Counter Styles Level 3
// (<https://drafts.csswg.org/css-counter-styles-3/>): the tables of section 6, the CJK table
// of section 7.1 and the Ethiopic examples of section 7.2.

#[path = "../counter_styles.rs"]
#[allow(dead_code)]
mod counter_styles;

mod counter_styles_tests {
    use super::counter_styles::{
        Algorithm, CounterSystem, PREDEFINED_NAMES, SymbolsKind, anonymous, generate, predefined,
        represent,
    };

    /// `generate` for a predefined style.
    fn text(style: &str, value: i32) -> String {
        let system = predefined(style).unwrap_or_else(|| panic!("{style} is predefined"));
        generate(&system, value)
    }

    /// The code points of a string, for failure messages and exact expectations.
    fn from_code_points(code_points: &[u32]) -> String {
        code_points
            .iter()
            .map(|code_point| char::from_u32(*code_point).expect("a scalar value"))
            .collect()
    }

    #[test]
    fn every_predefined_name_is_known_and_the_list_is_complete() {
        assert_eq!(PREDEFINED_NAMES.len(), 55);
        for name in PREDEFINED_NAMES {
            assert!(predefined(name).is_some(), "{name} is missing");
        }
        // No duplicates.
        let mut names = PREDEFINED_NAMES.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), PREDEFINED_NAMES.len());
        // Names that Stylo does not give to `predefined` (it lowercases the ones it knows).
        assert!(predefined("none").is_none());
        assert!(predefined("no-such-style").is_none());
        assert!(predefined("Decimal").is_none());
        assert!(predefined("-moz-hangul").is_none());
    }

    #[test]
    fn no_style_panics_or_gives_an_empty_text() {
        let values = (-130..=1300).chain([
            i32::MIN,
            i32::MIN + 1,
            -99_999_999,
            9_999,
            10_000,
            10_999,
            11_000,
            19_999,
            20_000,
            99_999_999,
            i32::MAX - 1,
            i32::MAX,
        ]);
        let values: Vec<i32> = values.collect();
        for name in PREDEFINED_NAMES {
            for value in &values {
                let representation = text(name, *value);
                assert!(!representation.is_empty(), "{name} {value}");
            }
        }
    }

    #[test]
    fn decimal() {
        assert_eq!(text("decimal", 0), "0");
        assert_eq!(text("decimal", 1), "1");
        assert_eq!(text("decimal", 42), "42");
        assert_eq!(text("decimal", -7), "-7");
        assert_eq!(text("decimal", i32::MAX), "2147483647");
        assert_eq!(text("decimal", i32::MIN), "-2147483648");
    }

    #[test]
    fn decimal_leading_zero_pads_to_two_digits_and_counts_the_sign() {
        assert_eq!(text("decimal-leading-zero", 0), "00");
        assert_eq!(text("decimal-leading-zero", 1), "01");
        assert_eq!(text("decimal-leading-zero", 9), "09");
        assert_eq!(text("decimal-leading-zero", 10), "10");
        assert_eq!(text("decimal-leading-zero", 100), "100");
        // The minimum length of two includes the minus sign, as it does in the specification's
        // "pad: 3" example below ("-05"): -5 already has two characters.
        assert_eq!(text("decimal-leading-zero", -5), "-5");
        assert_eq!(text("decimal-leading-zero", -10), "-10");
        assert_eq!(text("decimal-leading-zero", -100), "-100");
    }

    #[test]
    fn pad_the_example_of_the_specification() {
        // pad: 3 "0": 1 -> "001", 20 -> "020", 300 -> "300", 4000 -> "4000", -5 -> "-05".
        let decimal = predefined("decimal").unwrap();
        let padded = decimal.with_pad(3, "0");
        let written = |value| generate(&padded, value);
        assert_eq!(written(1), "001");
        assert_eq!(written(20), "020");
        assert_eq!(written(300), "300");
        assert_eq!(written(4000), "4000");
        assert_eq!(written(-5), "-05");
    }

    #[test]
    fn a_pad_that_is_too_long_uses_the_fallback() {
        let decimal = predefined("decimal").unwrap();
        let padded = decimal.with_pad(10_000, "0");
        assert_eq!(represent(&padded, 5), None);
        assert_eq!(generate(&padded, 5), "5");
    }

    #[test]
    fn negative_can_wrap_the_value() {
        let decimal = predefined("decimal").unwrap();
        let mut wrapped = decimal;
        wrapped.negative = ("(", ")");
        assert_eq!(generate(&wrapped, -2), "(2)");
        assert_eq!(generate(&wrapped, 2), "2");
    }

    #[test]
    fn roman_numerals() {
        let lower = |value| text("lower-roman", value);
        let upper = |value| text("upper-roman", value);
        for (value, roman) in [
            (1, "I"),
            (2, "II"),
            (3, "III"),
            (4, "IV"),
            (5, "V"),
            (9, "IX"),
            (14, "XIV"),
            (40, "XL"),
            (49, "XLIX"),
            (90, "XC"),
            (99, "XCIX"),
            (400, "CD"),
            (444, "CDXLIV"),
            (900, "CM"),
            (1994, "MCMXCIV"),
            (2024, "MMXXIV"),
            (3888, "MMMDCCCLXXXVIII"),
            (3999, "MMMCMXCIX"),
        ] {
            assert_eq!(upper(value), roman, "upper {value}");
            assert_eq!(lower(value), roman.to_lowercase(), "lower {value}");
        }
    }

    #[test]
    fn roman_numerals_stop_at_3999_and_start_at_1() {
        // The range of both styles is 1 to 3999; anything else is written in decimal.
        for style in ["lower-roman", "upper-roman"] {
            assert_eq!(text(style, 4000), "4000");
            assert_eq!(text(style, 12345), "12345");
            assert_eq!(text(style, 0), "0");
            assert_eq!(text(style, -1), "-1");
            assert_eq!(text(style, i32::MIN), "-2147483648");
            assert_eq!(represent(&predefined(style).unwrap(), 4000), None);
        }
    }

    #[test]
    fn every_roman_numeral_from_1_to_3999_reads_back_to_its_value() {
        fn parse(roman: &str) -> u32 {
            let value = |c: char| match c {
                'I' => 1,
                'V' => 5,
                'X' => 10,
                'L' => 50,
                'C' => 100,
                'D' => 500,
                'M' => 1000,
                other => panic!("not a roman digit: {other}"),
            };
            let digits: Vec<u32> = roman.chars().map(value).collect();
            let mut total = 0;
            for (index, digit) in digits.iter().enumerate() {
                if digits.get(index + 1).is_some_and(|next| next > digit) {
                    total -= *digit;
                } else {
                    total += *digit;
                }
            }
            total
        }
        for value in 1..=3999 {
            assert_eq!(parse(&text("upper-roman", value)), value as u32, "{value}");
        }
    }

    #[test]
    fn alphabetic_styles_count_like_a_spreadsheet_column() {
        for (value, letters) in [
            (1, "a"),
            (2, "b"),
            (26, "z"),
            (27, "aa"),
            (28, "ab"),
            (52, "az"),
            (53, "ba"),
            (702, "zz"),
            (703, "aaa"),
        ] {
            assert_eq!(text("lower-alpha", value), letters, "{value}");
            assert_eq!(text("lower-latin", value), letters, "{value}");
            assert_eq!(
                text("upper-alpha", value),
                letters.to_uppercase(),
                "{value}"
            );
            assert_eq!(
                text("upper-latin", value),
                letters.to_uppercase(),
                "{value}"
            );
        }
        // Zero and negative values are outside the range of an alphabetic style.
        assert_eq!(text("lower-alpha", 0), "0");
        assert_eq!(text("upper-alpha", -3), "-3");
    }

    #[test]
    fn greek_hiragana_and_katakana() {
        // lower-greek has 24 letters (no final sigma).
        assert_eq!(text("lower-greek", 1), "\u{3b1}");
        assert_eq!(text("lower-greek", 17), "\u{3c1}");
        assert_eq!(text("lower-greek", 18), "\u{3c3}");
        assert_eq!(text("lower-greek", 24), "\u{3c9}");
        assert_eq!(text("lower-greek", 25), "\u{3b1}\u{3b1}");
        // hiragana: a i u e o ... (the 50 sounds order, 48 symbols).
        assert_eq!(text("hiragana", 1), "\u{3042}");
        assert_eq!(text("hiragana", 5), "\u{304a}");
        assert_eq!(text("hiragana", 48), "\u{3093}");
        assert_eq!(text("hiragana", 49), "\u{3042}\u{3042}");
        // hiragana-iroha starts with "i ro ha".
        assert_eq!(text("hiragana-iroha", 1), "\u{3044}");
        assert_eq!(text("hiragana-iroha", 2), "\u{308d}");
        assert_eq!(text("hiragana-iroha", 3), "\u{306f}");
        assert_eq!(text("katakana", 1), "\u{30a2}");
        assert_eq!(text("katakana", 48), "\u{30f3}");
        assert_eq!(text("katakana-iroha", 1), "\u{30a4}");
        assert_eq!(text("katakana-iroha", 47), "\u{30b9}");
        assert_eq!(text("katakana-iroha", 48), "\u{30a4}\u{30a4}");
    }

    #[test]
    fn every_digit_style_writes_its_own_digits() {
        // The zero of each positional style; the ten digits are consecutive code points.
        for (name, zero) in [
            ("arabic-indic", 0x660u32),
            ("bengali", 0x9e6),
            ("cambodian", 0x17e0),
            ("khmer", 0x17e0),
            ("devanagari", 0x966),
            ("gujarati", 0xae6),
            ("gurmukhi", 0xa66),
            ("kannada", 0xce6),
            ("lao", 0xed0),
            ("malayalam", 0xd66),
            ("mongolian", 0x1810),
            ("myanmar", 0x1040),
            ("oriya", 0xb66),
            ("persian", 0x6f0),
            ("tamil", 0xbe6),
            ("telugu", 0xc66),
            ("thai", 0xe50),
            ("tibetan", 0xf20),
        ] {
            for digit in 0..10 {
                assert_eq!(
                    text(name, digit),
                    from_code_points(&[zero + digit as u32]),
                    "{name} {digit}"
                );
            }
            // 105 and -42 use the same digits, the second with an ASCII minus.
            assert_eq!(
                text(name, 105),
                from_code_points(&[zero + 1, zero, zero + 5]),
                "{name} 105"
            );
            assert_eq!(
                text(name, -42),
                format!("-{}", from_code_points(&[zero + 4, zero + 2])),
                "{name} -42"
            );
        }
        // The examples of the specification: 1, 2, 3 ... 98, 99, 100 in Thai and Arabic-Indic.
        assert_eq!(text("thai", 100), "\u{e51}\u{e50}\u{e50}");
        assert_eq!(text("arabic-indic", 98), "\u{669}\u{668}");
    }

    #[test]
    fn cjk_decimal_has_a_range_that_starts_at_zero() {
        assert_eq!(text("cjk-decimal", 0), "\u{3007}");
        assert_eq!(
            text("cjk-decimal", 2024),
            "\u{4e8c}\u{3007}\u{4e8c}\u{56db}"
        );
        // Below zero it is out of range, so decimal is used.
        assert_eq!(text("cjk-decimal", -3), "-3");
        assert_eq!(represent(&predefined("cjk-decimal").unwrap(), -3), None);
    }

    #[test]
    fn armenian() {
        assert_eq!(text("armenian", 1), "\u{531}");
        assert_eq!(text("armenian", 10), "\u{53a}");
        assert_eq!(text("armenian", 11), "\u{53a}\u{531}");
        // 2000 + 20 + 4
        assert_eq!(text("armenian", 2024), "\u{54d}\u{53b}\u{534}");
        assert_eq!(text("upper-armenian", 2024), "\u{54d}\u{53b}\u{534}");
        assert_eq!(text("armenian", 9999), "\u{554}\u{54b}\u{542}\u{539}");
        assert_eq!(text("lower-armenian", 1), "\u{561}");
        assert_eq!(text("lower-armenian", 2024), "\u{57d}\u{56b}\u{564}");
        // The range is 1 to 9999.
        assert_eq!(text("armenian", 10_000), "10000");
        assert_eq!(text("lower-armenian", 0), "0");
        assert_eq!(text("armenian", -2), "-2");
    }

    #[test]
    fn georgian() {
        assert_eq!(text("georgian", 1), "\u{10d0}");
        assert_eq!(text("georgian", 10), "\u{10d8}");
        assert_eq!(text("georgian", 100), "\u{10e0}");
        assert_eq!(text("georgian", 1000), "\u{10e9}");
        assert_eq!(text("georgian", 10_000), "\u{10f5}");
        // 10000 + 9000 + 900 + 90 + 9
        assert_eq!(
            text("georgian", 19_999),
            "\u{10f5}\u{10f0}\u{10e8}\u{10df}\u{10d7}"
        );
        // The range is 1 to 19999.
        assert_eq!(text("georgian", 20_000), "20000");
        assert_eq!(text("georgian", 0), "0");
    }

    #[test]
    fn hebrew() {
        // 1, 2, 3 ... 98, 99, 100 of the specification: alef, bet, gimel ... tsadi-het, tsadi-tet, qof.
        assert_eq!(text("hebrew", 1), "\u{5d0}");
        assert_eq!(text("hebrew", 2), "\u{5d1}");
        assert_eq!(text("hebrew", 3), "\u{5d2}");
        assert_eq!(text("hebrew", 10), "\u{5d9}");
        assert_eq!(text("hebrew", 11), "\u{5d9}\u{5d0}");
        assert_eq!(text("hebrew", 98), "\u{5e6}\u{5d7}");
        assert_eq!(text("hebrew", 99), "\u{5e6}\u{5d8}");
        assert_eq!(text("hebrew", 100), "\u{5e7}");
        // 15 and 16 are not written with yod-he and yod-vav.
        assert_eq!(text("hebrew", 15), "\u{5d8}\u{5d5}");
        assert_eq!(text("hebrew", 16), "\u{5d8}\u{5d6}");
        assert_eq!(text("hebrew", 17), "\u{5d9}\u{5d6}");
        assert_eq!(text("hebrew", 19), "\u{5d9}\u{5d8}");
        assert_eq!(text("hebrew", 400), "\u{5ea}");
        assert_eq!(text("hebrew", 500), "\u{5ea}\u{5e7}");
        // 1000 and up use the geresh.
        assert_eq!(text("hebrew", 1000), "\u{5d0}\u{5f3}");
        assert_eq!(
            text("hebrew", 5785),
            "\u{5d4}\u{5f3}\u{5ea}\u{5e9}\u{5e4}\u{5d4}"
        );
        assert_eq!(text("hebrew", 10_000), "\u{5d9}\u{5f3}");
        // The range is 1 to 10999.
        assert_eq!(text("hebrew", 11_000), "11000");
        assert_eq!(text("hebrew", 0), "0");
        assert_eq!(text("hebrew", -1), "-1");
    }

    /// The table of "Longhand East Asian Counter Styles" in the specification: 0, 1, 2, 3, 10,
    /// 11, 99, 100, 101 and 6001.
    #[test]
    fn longhand_east_asian_styles_on_the_examples_of_the_specification() {
        const VALUES: [i32; 10] = [0, 1, 2, 3, 10, 11, 99, 100, 101, 6001];
        let table: [(&str, [&str; 10]); 9] = [
            (
                "japanese-informal",
                [
                    "〇",
                    "一",
                    "二",
                    "三",
                    "十",
                    "十一",
                    "九十九",
                    "百",
                    "百一",
                    "六千一",
                ],
            ),
            (
                "japanese-formal",
                [
                    "零",
                    "壱",
                    "弐",
                    "参",
                    "壱拾",
                    "壱拾壱",
                    "九拾九",
                    "壱百",
                    "壱百壱",
                    "六阡壱",
                ],
            ),
            (
                "korean-hangul-formal",
                [
                    "영",
                    "일",
                    "이",
                    "삼",
                    "일십",
                    "일십일",
                    "구십구",
                    "일백",
                    "일백일",
                    "육천일",
                ],
            ),
            (
                "korean-hanja-informal",
                [
                    "零",
                    "一",
                    "二",
                    "三",
                    "十",
                    "十一",
                    "九十九",
                    "百",
                    "百一",
                    "六千一",
                ],
            ),
            (
                "korean-hanja-formal",
                [
                    "零",
                    "壹",
                    "貳",
                    "參",
                    "壹拾",
                    "壹拾壹",
                    "九拾九",
                    "壹百",
                    "壹百壹",
                    "六仟壹",
                ],
            ),
            (
                "simp-chinese-informal",
                [
                    "零",
                    "一",
                    "二",
                    "三",
                    "十",
                    "十一",
                    "九十九",
                    "一百",
                    "一百零一",
                    "六千零一",
                ],
            ),
            (
                "simp-chinese-formal",
                [
                    "零",
                    "壹",
                    "贰",
                    "叁",
                    "壹拾",
                    "壹拾壹",
                    "玖拾玖",
                    "壹佰",
                    "壹佰零壹",
                    "陆仟零壹",
                ],
            ),
            (
                "trad-chinese-informal",
                [
                    "零",
                    "一",
                    "二",
                    "三",
                    "十",
                    "十一",
                    "九十九",
                    "一百",
                    "一百零一",
                    "六千零一",
                ],
            ),
            (
                "trad-chinese-formal",
                [
                    "零",
                    "壹",
                    "貳",
                    "參",
                    "壹拾",
                    "壹拾壹",
                    "玖拾玖",
                    "壹佰",
                    "壹佰零壹",
                    "陸仟零壹",
                ],
            ),
        ];
        for (style, expected) in table {
            for (value, expected) in VALUES.iter().zip(expected) {
                assert_eq!(text(style, *value), expected, "{style} {value}");
            }
        }
        // cjk-ideographic is another name of trad-chinese-informal.
        for value in VALUES {
            assert_eq!(
                text("cjk-ideographic", value),
                text("trad-chinese-informal", value)
            );
        }
    }

    #[test]
    fn longhand_east_asian_styles_over_ten_thousand_and_negative() {
        assert_eq!(text("simp-chinese-informal", 10_000), "一万");
        assert_eq!(text("trad-chinese-informal", 10_000), "一萬");
        assert_eq!(text("japanese-informal", 10_000), "一万");
        // Korean leaves out the 1 before 10000 in the informal style.
        assert_eq!(text("korean-hanja-informal", 10_000), "萬");
        assert_eq!(text("korean-hangul-formal", 10_000), "일만");
        assert_eq!(text("trad-chinese-informal", 10_001), "一萬零一");
        assert_eq!(
            text("trad-chinese-informal", 123_456),
            "十二萬三千四百五十六"
        );
        assert_eq!(text("japanese-formal", 20_000), "弐萬");
        // The sign is a word in these styles.
        assert_eq!(text("simp-chinese-informal", -1), "负一");
        assert_eq!(text("trad-chinese-informal", -1), "負一");
        assert_eq!(text("japanese-informal", -1), "マイナス一");
        assert_eq!(text("korean-hangul-formal", -1), "마이너스 일");
        // 2,147,483,647 is written without a panic and ends with 7 (七).
        assert!(text("trad-chinese-informal", i32::MAX).ends_with('七'));
        assert!(text("japanese-formal", i32::MIN).starts_with("マイナス"));
    }

    #[test]
    fn ethiopic_numerals() {
        // The examples of the specification.
        assert_eq!(text("ethiopic-numeric", 1), "\u{1369}");
        assert_eq!(text("ethiopic-numeric", 2), "\u{136a}");
        assert_eq!(text("ethiopic-numeric", 10), "\u{1372}");
        assert_eq!(text("ethiopic-numeric", 11), "\u{1372}\u{1369}");
        assert_eq!(text("ethiopic-numeric", 20), "\u{1373}");
        // 100 is one hundred, written as the hundred sign alone.
        assert_eq!(text("ethiopic-numeric", 100), "\u{137b}");
        assert_eq!(text("ethiopic-numeric", 10_000), "\u{137c}");
        // 78010092
        assert_eq!(
            text("ethiopic-numeric", 78_010_092),
            from_code_points(&[0x1378, 0x1370, 0x137b, 0x1369, 0x137c, 0x137a, 0x136a])
        );
        // 100000000 is two ten-thousand signs.
        assert_eq!(text("ethiopic-numeric", 100_000_000), "\u{137c}\u{137c}");
        // Zero and below are outside the range.
        assert_eq!(text("ethiopic-numeric", 0), "0");
        assert_eq!(text("ethiopic-numeric", -5), "-5");
    }

    #[test]
    fn heavenly_stems_and_earthly_branches_are_fixed_and_fall_back_to_cjk_decimal() {
        assert_eq!(text("cjk-heavenly-stem", 1), "\u{7532}");
        assert_eq!(text("cjk-heavenly-stem", 10), "\u{7678}");
        assert_eq!(text("cjk-earthly-branch", 1), "\u{5b50}");
        assert_eq!(text("cjk-earthly-branch", 12), "\u{4ea5}");
        // Out of the ten (twelve) symbols: cjk-decimal, whose own range starts at 0.
        assert_eq!(text("cjk-heavenly-stem", 11), "\u{4e00}\u{4e00}");
        assert_eq!(text("cjk-earthly-branch", 13), "\u{4e00}\u{4e09}");
        assert_eq!(text("cjk-heavenly-stem", 0), "\u{3007}");
        // ... and below zero cjk-decimal falls back to decimal.
        assert_eq!(text("cjk-heavenly-stem", -2), "-2");
    }

    #[test]
    fn bullets_ignore_the_value() {
        for (style, bullet) in [
            ("disc", "\u{2022}"),
            ("circle", "\u{25e6}"),
            ("square", "\u{25aa}"),
            ("disclosure-open", "\u{25be}"),
            ("disclosure-closed", "\u{25b8}"),
        ] {
            for value in [-5, 0, 1, 2, 1000, i32::MAX] {
                assert_eq!(text(style, value), bullet, "{style} {value}");
            }
            assert!(!predefined(style).unwrap().depends_on_value());
        }
        assert!(predefined("decimal").unwrap().depends_on_value());
        assert!(predefined("lower-roman").unwrap().depends_on_value());
    }

    #[test]
    fn markers_use_the_suffix_of_the_style() {
        let suffix = |style: &str| predefined(style).unwrap().suffix;
        assert_eq!(suffix("decimal"), ". ");
        assert_eq!(suffix("lower-roman"), ". ");
        assert_eq!(suffix("hebrew"), ". ");
        assert_eq!(suffix("disc"), " ");
        assert_eq!(suffix("disclosure-closed"), " ");
        assert_eq!(suffix("hiragana"), "\u{3001}");
        assert_eq!(suffix("katakana-iroha"), "\u{3001}");
        assert_eq!(suffix("cjk-decimal"), "\u{3001}");
        assert_eq!(suffix("cjk-earthly-branch"), "\u{3001}");
        assert_eq!(suffix("japanese-formal"), "\u{3001}");
        assert_eq!(suffix("simp-chinese-informal"), "\u{3001}");
        assert_eq!(suffix("cjk-ideographic"), "\u{3001}");
        assert_eq!(suffix("korean-hangul-formal"), ", ");
        assert_eq!(suffix("korean-hanja-formal"), ", ");
        assert_eq!(suffix("ethiopic-numeric"), "/ ");
        // Every prefix is empty.
        for name in PREDEFINED_NAMES {
            assert_eq!(predefined(name).unwrap().prefix, "", "{name}");
        }
    }

    #[test]
    fn symbols_function_symbolic() {
        let symbols = ["*", "\u{2020}"];
        let system = anonymous(SymbolsKind::Symbolic, &symbols);
        assert_eq!(system.suffix, " ");
        let written = |value| generate(&system, value);
        assert_eq!(written(1), "*");
        assert_eq!(written(2), "\u{2020}");
        assert_eq!(written(3), "**");
        assert_eq!(written(4), "\u{2020}\u{2020}");
        assert_eq!(written(5), "***");
        // Zero and negative values are not defined: decimal.
        assert_eq!(written(0), "0");
        assert_eq!(written(-4), "-4");
        // 1000 stars would be longer than the limit: decimal.
        let one = ["*"];
        let one = anonymous(SymbolsKind::Symbolic, &one);
        assert_eq!(generate(&one, 3), "***");
        assert_eq!(generate(&one, 1000), "1000");
    }

    #[test]
    fn symbols_function_cyclic_numeric_alphabetic_and_fixed() {
        let symbols = ["a", "b", "c"];
        let cyclic = anonymous(SymbolsKind::Cyclic, &symbols);
        assert_eq!(generate(&cyclic, 1), "a");
        assert_eq!(generate(&cyclic, 3), "c");
        assert_eq!(generate(&cyclic, 4), "a");
        assert_eq!(generate(&cyclic, 0), "c");
        assert_eq!(generate(&cyclic, -1), "b");
        assert!(cyclic.depends_on_value());

        let digits = ["o", "i"];
        let binary = anonymous(SymbolsKind::Numeric, &digits);
        assert_eq!(generate(&binary, 0), "o");
        assert_eq!(generate(&binary, 1), "i");
        assert_eq!(generate(&binary, 2), "io");
        assert_eq!(generate(&binary, 5), "ioi");
        assert_eq!(generate(&binary, -5), "-ioi");

        let letters = ["x", "y"];
        let alphabetic = anonymous(SymbolsKind::Alphabetic, &letters);
        assert_eq!(generate(&alphabetic, 1), "x");
        assert_eq!(generate(&alphabetic, 2), "y");
        assert_eq!(generate(&alphabetic, 3), "xx");
        assert_eq!(generate(&alphabetic, 4), "xy");
        assert_eq!(generate(&alphabetic, 0), "0");

        let fixed = anonymous(SymbolsKind::Fixed, &symbols);
        assert_eq!(generate(&fixed, 1), "a");
        assert_eq!(generate(&fixed, 3), "c");
        assert_eq!(generate(&fixed, 4), "4");
        assert_eq!(generate(&fixed, 0), "0");
        assert_eq!(generate(&fixed, -1), "-1");
    }

    #[test]
    fn fixed_with_another_first_value_and_a_range() {
        let symbols = ["a", "b", "c"];
        let fixed = CounterSystem::new(Algorithm::Fixed {
            first: 5,
            symbols: &symbols,
        });
        assert_eq!(generate(&fixed, 5), "a");
        assert_eq!(generate(&fixed, 7), "c");
        assert_eq!(generate(&fixed, 4), "4");
        assert_eq!(generate(&fixed, 8), "8");
        // A range narrower than the symbols: the fallback outside it.
        let ranged = fixed.with_range(6, 7);
        assert_eq!(generate(&ranged, 5), "5");
        assert_eq!(generate(&ranged, 6), "b");
        assert_eq!(generate(&ranged, 7), "c");
        assert_eq!(represent(&ranged, 5), None);
    }

    #[test]
    fn additive_symbols_must_add_up_exactly() {
        // 5 and 3 cannot make 4 or 7.
        let table = [(5, "V"), (3, "III")];
        let system = CounterSystem::new(Algorithm::Additive(&table));
        assert_eq!(generate(&system, 8), "VIII");
        assert_eq!(generate(&system, 5), "V");
        assert_eq!(generate(&system, 4), "4");
        assert_eq!(generate(&system, 7), "7");
        // Zero needs a symbol of weight zero at the end of the table.
        assert_eq!(generate(&system, 0), "0");
        let with_zero = [(5, "V"), (0, "nil")];
        let system = CounterSystem::new(Algorithm::Additive(&with_zero));
        assert_eq!(generate(&system, 0), "nil");
        assert_eq!(generate(&system, 10), "VV");
    }

    #[test]
    fn a_style_extended_with_another_prefix_range_or_negative() {
        let mut decimal = predefined("decimal").unwrap();
        decimal.range = (0, 9);
        assert_eq!(generate(&decimal, 9), "9");
        assert_eq!(generate(&decimal, 10), "10");
        assert_eq!(represent(&decimal, 10), None);
        assert_eq!(represent(&decimal, -1), None);
    }

    #[test]
    fn very_large_and_very_small_values_do_not_overflow() {
        for name in PREDEFINED_NAMES {
            let _ = text(name, i32::MIN);
            let _ = text(name, i32::MAX);
        }
        assert_eq!(text("lower-alpha", i32::MAX), "fxshrxw");
        // A cyclic style with the smallest value.
        assert_eq!(text("disc", i32::MIN), "\u{2022}");
        assert_eq!(text("thai", i32::MIN).chars().next(), Some('-'));
    }
}
