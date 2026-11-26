// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Place words and zone abbreviations as IANA zone ids, and the `UTC+h:mm` offset form.

/// `(word, IANA id, display)`: the lower-case words a time question may name.
const PLACES: &[(&str, &str, &str)] = &[
    ("utc", "UTC", "UTC"),
    ("gmt", "UTC", "GMT"),
    ("z", "UTC", "Z"),
    ("est", "America/New_York", "EST"),
    ("edt", "America/New_York", "EDT"),
    ("et", "America/New_York", "ET"),
    ("cst", "America/Chicago", "CST"),
    ("cdt", "America/Chicago", "CDT"),
    ("ct", "America/Chicago", "CT"),
    ("mst", "America/Denver", "MST"),
    ("mdt", "America/Denver", "MDT"),
    ("mt", "America/Denver", "MT"),
    ("pst", "America/Los_Angeles", "PST"),
    ("pdt", "America/Los_Angeles", "PDT"),
    ("pt", "America/Los_Angeles", "PT"),
    ("akst", "America/Anchorage", "AKST"),
    ("akdt", "America/Anchorage", "AKDT"),
    ("hst", "Pacific/Honolulu", "HST"),
    ("wet", "Europe/Lisbon", "WET"),
    ("west", "Europe/Lisbon", "WEST"),
    ("bst", "Europe/London", "BST"),
    ("cet", "Europe/Paris", "CET"),
    ("cest", "Europe/Paris", "CEST"),
    ("eet", "Europe/Athens", "EET"),
    ("eest", "Europe/Athens", "EEST"),
    ("msk", "Europe/Moscow", "MSK"),
    ("ist", "Asia/Kolkata", "IST"),
    ("pkt", "Asia/Karachi", "PKT"),
    ("jst", "Asia/Tokyo", "JST"),
    ("kst", "Asia/Seoul", "KST"),
    ("hkt", "Asia/Hong_Kong", "HKT"),
    ("sgt", "Asia/Singapore", "SGT"),
    ("aest", "Australia/Sydney", "AEST"),
    ("aedt", "Australia/Sydney", "AEDT"),
    ("acst", "Australia/Adelaide", "ACST"),
    ("awst", "Australia/Perth", "AWST"),
    ("nzst", "Pacific/Auckland", "NZST"),
    ("nzdt", "Pacific/Auckland", "NZDT"),
    ("new york", "America/New_York", "New York"),
    ("nyc", "America/New_York", "New York"),
    ("ny", "America/New_York", "New York"),
    ("boston", "America/New_York", "Boston"),
    ("miami", "America/New_York", "Miami"),
    ("atlanta", "America/New_York", "Atlanta"),
    ("washington", "America/New_York", "Washington"),
    ("toronto", "America/Toronto", "Toronto"),
    ("montreal", "America/Toronto", "Montreal"),
    ("chicago", "America/Chicago", "Chicago"),
    ("houston", "America/Chicago", "Houston"),
    ("dallas", "America/Chicago", "Dallas"),
    ("winnipeg", "America/Winnipeg", "Winnipeg"),
    ("denver", "America/Denver", "Denver"),
    ("calgary", "America/Edmonton", "Calgary"),
    ("edmonton", "America/Edmonton", "Edmonton"),
    ("phoenix", "America/Phoenix", "Phoenix"),
    ("los angeles", "America/Los_Angeles", "Los Angeles"),
    ("la", "America/Los_Angeles", "Los Angeles"),
    ("las vegas", "America/Los_Angeles", "Las Vegas"),
    ("san francisco", "America/Los_Angeles", "San Francisco"),
    ("sf", "America/Los_Angeles", "San Francisco"),
    ("seattle", "America/Los_Angeles", "Seattle"),
    ("vancouver", "America/Vancouver", "Vancouver"),
    ("anchorage", "America/Anchorage", "Anchorage"),
    ("honolulu", "Pacific/Honolulu", "Honolulu"),
    ("mexico city", "America/Mexico_City", "Mexico City"),
    ("havana", "America/Havana", "Havana"),
    ("panama", "America/Panama", "Panama"),
    ("bogota", "America/Bogota", "Bogota"),
    ("lima", "America/Lima", "Lima"),
    ("quito", "America/Guayaquil", "Quito"),
    ("caracas", "America/Caracas", "Caracas"),
    ("santiago", "America/Santiago", "Santiago"),
    (
        "buenos aires",
        "America/Argentina/Buenos_Aires",
        "Buenos Aires",
    ),
    ("sao paulo", "America/Sao_Paulo", "Sao Paulo"),
    ("montevideo", "America/Montevideo", "Montevideo"),
    ("la paz", "America/La_Paz", "La Paz"),
    ("halifax", "America/Halifax", "Halifax"),
    ("london", "Europe/London", "London"),
    ("dublin", "Europe/Dublin", "Dublin"),
    ("lisbon", "Europe/Lisbon", "Lisbon"),
    ("reykjavik", "Atlantic/Reykjavik", "Reykjavik"),
    ("paris", "Europe/Paris", "Paris"),
    ("brussels", "Europe/Brussels", "Brussels"),
    ("amsterdam", "Europe/Amsterdam", "Amsterdam"),
    ("berlin", "Europe/Berlin", "Berlin"),
    ("frankfurt", "Europe/Berlin", "Frankfurt"),
    ("munich", "Europe/Berlin", "Munich"),
    ("madrid", "Europe/Madrid", "Madrid"),
    ("barcelona", "Europe/Madrid", "Barcelona"),
    ("rome", "Europe/Rome", "Rome"),
    ("milan", "Europe/Rome", "Milan"),
    ("zurich", "Europe/Zurich", "Zurich"),
    ("geneva", "Europe/Zurich", "Geneva"),
    ("vienna", "Europe/Vienna", "Vienna"),
    ("prague", "Europe/Prague", "Prague"),
    ("warsaw", "Europe/Warsaw", "Warsaw"),
    ("budapest", "Europe/Budapest", "Budapest"),
    ("stockholm", "Europe/Stockholm", "Stockholm"),
    ("oslo", "Europe/Oslo", "Oslo"),
    ("copenhagen", "Europe/Copenhagen", "Copenhagen"),
    ("helsinki", "Europe/Helsinki", "Helsinki"),
    ("athens", "Europe/Athens", "Athens"),
    ("bucharest", "Europe/Bucharest", "Bucharest"),
    ("sofia", "Europe/Sofia", "Sofia"),
    ("istanbul", "Europe/Istanbul", "Istanbul"),
    ("moscow", "Europe/Moscow", "Moscow"),
    ("kyiv", "Europe/Kyiv", "Kyiv"),
    ("cairo", "Africa/Cairo", "Cairo"),
    ("johannesburg", "Africa/Johannesburg", "Johannesburg"),
    ("cape town", "Africa/Johannesburg", "Cape Town"),
    ("lagos", "Africa/Lagos", "Lagos"),
    ("nairobi", "Africa/Nairobi", "Nairobi"),
    ("casablanca", "Africa/Casablanca", "Casablanca"),
    ("algiers", "Africa/Algiers", "Algiers"),
    ("tunis", "Africa/Tunis", "Tunis"),
    ("accra", "Africa/Accra", "Accra"),
    ("dakar", "Africa/Dakar", "Dakar"),
    ("addis ababa", "Africa/Addis_Ababa", "Addis Ababa"),
    ("khartoum", "Africa/Khartoum", "Khartoum"),
    ("kinshasa", "Africa/Kinshasa", "Kinshasa"),
    ("lusaka", "Africa/Lusaka", "Lusaka"),
    ("harare", "Africa/Harare", "Harare"),
    ("tokyo", "Asia/Tokyo", "Tokyo"),
    ("osaka", "Asia/Tokyo", "Osaka"),
    ("seoul", "Asia/Seoul", "Seoul"),
    ("beijing", "Asia/Shanghai", "Beijing"),
    ("shanghai", "Asia/Shanghai", "Shanghai"),
    ("hong kong", "Asia/Hong_Kong", "Hong Kong"),
    ("taipei", "Asia/Taipei", "Taipei"),
    ("manila", "Asia/Manila", "Manila"),
    ("singapore", "Asia/Singapore", "Singapore"),
    ("kuala lumpur", "Asia/Kuala_Lumpur", "Kuala Lumpur"),
    ("bangkok", "Asia/Bangkok", "Bangkok"),
    ("jakarta", "Asia/Jakarta", "Jakarta"),
    ("hanoi", "Asia/Ho_Chi_Minh", "Hanoi"),
    ("ho chi minh", "Asia/Ho_Chi_Minh", "Ho Chi Minh City"),
    ("delhi", "Asia/Kolkata", "Delhi"),
    ("mumbai", "Asia/Kolkata", "Mumbai"),
    ("kolkata", "Asia/Kolkata", "Kolkata"),
    ("bangalore", "Asia/Kolkata", "Bangalore"),
    ("chennai", "Asia/Kolkata", "Chennai"),
    ("karachi", "Asia/Karachi", "Karachi"),
    ("lahore", "Asia/Karachi", "Lahore"),
    ("dhaka", "Asia/Dhaka", "Dhaka"),
    ("kathmandu", "Asia/Kathmandu", "Kathmandu"),
    ("dubai", "Asia/Dubai", "Dubai"),
    ("abu dhabi", "Asia/Dubai", "Abu Dhabi"),
    ("riyadh", "Asia/Riyadh", "Riyadh"),
    ("tehran", "Asia/Tehran", "Tehran"),
    ("jerusalem", "Asia/Jerusalem", "Jerusalem"),
    ("tel aviv", "Asia/Jerusalem", "Tel Aviv"),
    ("baghdad", "Asia/Baghdad", "Baghdad"),
    ("almaty", "Asia/Almaty", "Almaty"),
    ("tashkent", "Asia/Tashkent", "Tashkent"),
    ("sydney", "Australia/Sydney", "Sydney"),
    ("melbourne", "Australia/Melbourne", "Melbourne"),
    ("brisbane", "Australia/Brisbane", "Brisbane"),
    ("perth", "Australia/Perth", "Perth"),
    ("adelaide", "Australia/Adelaide", "Adelaide"),
    ("darwin", "Australia/Darwin", "Darwin"),
    ("hobart", "Australia/Hobart", "Hobart"),
    ("auckland", "Pacific/Auckland", "Auckland"),
    ("wellington", "Pacific/Auckland", "Wellington"),
    ("fiji", "Pacific/Fiji", "Fiji"),
];

/// The IANA id and the display name for a place word (`nyc`, `new york`, `est`), if known.
///
/// The word may also be a UTC offset (`utc+5:30`, `gmt-3`), whose id and display are the
/// offset text (`UTC+5:30`), or any IANA id the bundled data knows (`america/new_york`), whose
/// display is its last segment in title case (`New York`).
pub(crate) fn lookup(word: &str) -> Option<(String, String)> {
    let word = word
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if let Some((_, id, display)) = PLACES.iter().find(|(name, _, _)| *name == word) {
        return Some(((*id).to_string(), (*display).to_string()));
    }
    if let Some(seconds) = word
        .strip_prefix("utc")
        .or_else(|| word.strip_prefix("gmt"))
        .and_then(offset_from_sign)
    {
        let id = offset_id(seconds);
        return Some((id.clone(), id));
    }
    let (name, _) = jiff_tzdb::get(&word)?;
    let last = word.rsplit('/').next()?;
    let display = last
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ");
    Some((name.to_string(), display))
}

/// The canonical text of a UTC offset in whole seconds: `UTC+0`, `UTC-5`, `UTC+5:30`.
pub(crate) fn offset_id(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let minutes = seconds.abs() / 60;
    match minutes % 60 {
        0 => format!("UTC{sign}{}", minutes / 60),
        rest => format!("UTC{sign}{}:{rest:02}", minutes / 60),
    }
}

/// The offset in seconds of `+5`, `-3` or `+5:30` (the text after `UTC` or `GMT`), if in range.
pub(crate) fn offset_from_sign(text: &str) -> Option<i32> {
    let (sign, digits) = match text.strip_prefix('+') {
        Some(digits) => (1, digits),
        None => (-1, text.strip_prefix('-')?),
    };
    let (hours, minutes) = match digits.split_once(':') {
        Some((hours, minutes)) => (small(hours)?, small(minutes)?),
        None => (small(digits)?, 0),
    };
    if hours > 14 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 3600 + minutes * 60))
}

fn small(text: &str) -> Option<i32> {
    if (1..=2).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_digit()) {
        text.parse().ok()
    } else {
        None
    }
}
