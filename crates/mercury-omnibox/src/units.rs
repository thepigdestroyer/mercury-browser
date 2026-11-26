// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Unit conversion: `<expression> <unit> to <unit>` for length, area, volume, mass, temperature,
//! time, speed and data (spec D5). The factors are physical constants, written out by hand.

use crate::calc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dimension {
    Length,
    Area,
    Volume,
    Mass,
    Temperature,
    Time,
    Speed,
    Data,
}

struct Unit {
    dimension: Dimension,
    symbol: &'static str,
    /// Multiplies a value in this unit to give the base unit (metre, square metre, litre,
    /// kilogram, kelvin, second, metre per second or byte).
    factor: f64,
    /// Added after `factor` to reach the base unit; zero except for the temperature scales.
    offset: f64,
    names: &'static [&'static str],
}

const fn unit(
    dimension: Dimension,
    symbol: &'static str,
    factor: f64,
    names: &'static [&'static str],
) -> Unit {
    Unit {
        dimension,
        symbol,
        factor,
        offset: 0.0,
        names,
    }
}

const UNITS: &[Unit] = &[
    unit(
        Dimension::Length,
        "m",
        1.0,
        &["m", "meter", "meters", "metre", "metres"],
    ),
    unit(
        Dimension::Length,
        "km",
        1000.0,
        &["km", "kilometer", "kilometers", "kilometre", "kilometres"],
    ),
    unit(
        Dimension::Length,
        "cm",
        0.01,
        &[
            "cm",
            "centimeter",
            "centimeters",
            "centimetre",
            "centimetres",
        ],
    ),
    unit(
        Dimension::Length,
        "mm",
        0.001,
        &[
            "mm",
            "millimeter",
            "millimeters",
            "millimetre",
            "millimetres",
        ],
    ),
    unit(
        Dimension::Length,
        "µm",
        0.000001,
        &[
            "um",
            "micrometer",
            "micrometers",
            "micrometre",
            "micrometres",
        ],
    ),
    unit(Dimension::Length, "mi", 1609.344, &["mi", "mile", "miles"]),
    unit(Dimension::Length, "yd", 0.9144, &["yd", "yard", "yards"]),
    unit(Dimension::Length, "ft", 0.3048, &["ft", "foot", "feet"]),
    unit(Dimension::Length, "in", 0.0254, &["in", "inch", "inches"]),
    unit(
        Dimension::Length,
        "nmi",
        1852.0,
        &["nmi", "nautical mile", "nautical miles"],
    ),
    unit(
        Dimension::Area,
        "m²",
        1.0,
        &[
            "m2",
            "square meter",
            "square meters",
            "square metre",
            "square metres",
        ],
    ),
    unit(
        Dimension::Area,
        "km²",
        1_000_000.0,
        &[
            "km2",
            "square kilometer",
            "square kilometers",
            "square kilometre",
            "square kilometres",
        ],
    ),
    unit(
        Dimension::Area,
        "cm²",
        0.0001,
        &[
            "cm2",
            "square centimeter",
            "square centimeters",
            "square centimetre",
            "square centimetres",
        ],
    ),
    unit(
        Dimension::Area,
        "ha",
        10_000.0,
        &["ha", "hectare", "hectares"],
    ),
    unit(
        Dimension::Area,
        "ac",
        4046.8564224,
        &["ac", "acre", "acres"],
    ),
    unit(
        Dimension::Area,
        "ft²",
        0.09290304,
        &["ft2", "square foot", "square feet"],
    ),
    unit(
        Dimension::Area,
        "in²",
        0.00064516,
        &["in2", "square inch", "square inches"],
    ),
    unit(
        Dimension::Area,
        "mi²",
        2_589_988.110336,
        &["mi2", "square mile", "square miles"],
    ),
    unit(
        Dimension::Volume,
        "L",
        1.0,
        &["l", "liter", "liters", "litre", "litres"],
    ),
    unit(
        Dimension::Volume,
        "mL",
        0.001,
        &[
            "ml",
            "milliliter",
            "milliliters",
            "millilitre",
            "millilitres",
        ],
    ),
    unit(
        Dimension::Volume,
        "m³",
        1000.0,
        &[
            "m3",
            "cubic meter",
            "cubic meters",
            "cubic metre",
            "cubic metres",
        ],
    ),
    unit(
        Dimension::Volume,
        "gal",
        3.785411784,
        &["gal", "gallon", "gallons"],
    ),
    unit(
        Dimension::Volume,
        "impgal",
        4.54609,
        &[
            "impgal",
            "imperial gallon",
            "imperial gallons",
            "uk gallon",
            "uk gallons",
        ],
    ),
    unit(
        Dimension::Volume,
        "qt",
        3.785411784 / 4.0,
        &["qt", "quart", "quarts"],
    ),
    unit(
        Dimension::Volume,
        "pt",
        3.785411784 / 8.0,
        &["pt", "pint", "pints"],
    ),
    unit(
        Dimension::Volume,
        "cup",
        3.785411784 / 16.0,
        &["cup", "cups"],
    ),
    unit(
        Dimension::Volume,
        "fl oz",
        3.785411784 / 128.0,
        &["floz", "fl oz", "fluid ounce", "fluid ounces"],
    ),
    unit(
        Dimension::Volume,
        "tbsp",
        3.785411784 / 256.0,
        &["tbsp", "tablespoon", "tablespoons"],
    ),
    unit(
        Dimension::Volume,
        "tsp",
        3.785411784 / 768.0,
        &["tsp", "teaspoon", "teaspoons"],
    ),
    unit(
        Dimension::Mass,
        "kg",
        1.0,
        &["kg", "kilo", "kilos", "kilogram", "kilograms"],
    ),
    unit(Dimension::Mass, "g", 0.001, &["g", "gram", "grams"]),
    unit(
        Dimension::Mass,
        "mg",
        0.000001,
        &["mg", "milligram", "milligrams"],
    ),
    unit(
        Dimension::Mass,
        "t",
        1000.0,
        &["t", "tonne", "tonnes", "metric ton", "metric tons"],
    ),
    unit(
        Dimension::Mass,
        "lb",
        0.45359237,
        &["lb", "lbs", "pound", "pounds"],
    ),
    unit(
        Dimension::Mass,
        "oz",
        0.45359237 / 16.0,
        &["oz", "ounce", "ounces"],
    ),
    unit(
        Dimension::Mass,
        "st",
        0.45359237 * 14.0,
        &["st", "stone", "stones"],
    ),
    Unit {
        dimension: Dimension::Temperature,
        symbol: "°C",
        factor: 1.0,
        offset: 273.15,
        names: &["c", "°c", "celsius", "degree celsius", "degrees celsius"],
    },
    Unit {
        dimension: Dimension::Temperature,
        symbol: "°F",
        factor: 5.0 / 9.0,
        offset: 459.67 * 5.0 / 9.0,
        names: &[
            "f",
            "°f",
            "fahrenheit",
            "degree fahrenheit",
            "degrees fahrenheit",
        ],
    },
    Unit {
        dimension: Dimension::Temperature,
        symbol: "K",
        factor: 1.0,
        offset: 0.0,
        names: &["k", "kelvin"],
    },
    unit(
        Dimension::Time,
        "ms",
        0.001,
        &["ms", "millisecond", "milliseconds"],
    ),
    unit(
        Dimension::Time,
        "s",
        1.0,
        &["s", "sec", "secs", "second", "seconds"],
    ),
    unit(
        Dimension::Time,
        "min",
        60.0,
        &["min", "mins", "minute", "minutes"],
    ),
    unit(
        Dimension::Time,
        "h",
        3600.0,
        &["h", "hr", "hrs", "hour", "hours"],
    ),
    unit(Dimension::Time, "d", 86_400.0, &["d", "day", "days"]),
    unit(Dimension::Time, "wk", 604_800.0, &["wk", "week", "weeks"]),
    unit(
        Dimension::Time,
        "yr",
        31_557_600.0,
        &["yr", "year", "years"],
    ),
    unit(
        Dimension::Speed,
        "m/s",
        1.0,
        &[
            "m/s",
            "meter per second",
            "meters per second",
            "metre per second",
            "metres per second",
        ],
    ),
    unit(
        Dimension::Speed,
        "km/h",
        1000.0 / 3600.0,
        &[
            "km/h",
            "kph",
            "kilometer per hour",
            "kilometers per hour",
            "kilometre per hour",
            "kilometres per hour",
        ],
    ),
    unit(
        Dimension::Speed,
        "mph",
        1609.344 / 3600.0,
        &["mph", "mile per hour", "miles per hour"],
    ),
    unit(
        Dimension::Speed,
        "kn",
        1852.0 / 3600.0,
        &["kn", "kt", "kts", "knot", "knots"],
    ),
    unit(Dimension::Data, "bit", 0.125, &["b", "bit", "bits"]),
    unit(Dimension::Data, "B", 1.0, &["B", "byte", "bytes"]),
    unit(
        Dimension::Data,
        "kB",
        1_000.0,
        &["kb", "kilobyte", "kilobytes"],
    ),
    unit(
        Dimension::Data,
        "MB",
        1_000_000.0,
        &["mb", "megabyte", "megabytes"],
    ),
    unit(
        Dimension::Data,
        "GB",
        1_000_000_000.0,
        &["gb", "gigabyte", "gigabytes"],
    ),
    unit(
        Dimension::Data,
        "TB",
        1_000_000_000_000.0,
        &["tb", "terabyte", "terabytes"],
    ),
    unit(
        Dimension::Data,
        "KiB",
        1024.0,
        &["kib", "kibibyte", "kibibytes"],
    ),
    unit(
        Dimension::Data,
        "MiB",
        1_048_576.0,
        &["mib", "mebibyte", "mebibytes"],
    ),
    unit(
        Dimension::Data,
        "GiB",
        1_073_741_824.0,
        &["gib", "gibibyte", "gibibytes"],
    ),
    unit(
        Dimension::Data,
        "TiB",
        1_099_511_627_776.0,
        &["tib", "tebibyte", "tebibytes"],
    ),
    unit(
        Dimension::Data,
        "kbit",
        125.0,
        &["kbit", "kilobit", "kilobits"],
    ),
    unit(
        Dimension::Data,
        "Mbit",
        125_000.0,
        &["mbit", "megabit", "megabits"],
    ),
    unit(
        Dimension::Data,
        "Gbit",
        125_000_000.0,
        &["gbit", "gigabit", "gigabits"],
    ),
];

const CONNECTORS: &[&str] = &[" to ", " in ", " as ", " into ", "->", "→"];

/// `(value, unit symbol)` for `5 km to mi`-style input, `None` if it is not a conversion.
///
/// Each connector occurrence is tried from the left; the first split whose right side is a unit
/// and whose left side is a quantity decides. A different dimension or a non-finite result gives
/// `None`.
pub(crate) fn convert(input: &str) -> Option<(f64, &'static str)> {
    for (at, end) in connector_spans(input) {
        let (Some(left), Some(right)) = (input.get(..at), input.get(end..)) else {
            continue;
        };
        let Some((value, from)) = quantity(left.trim()) else {
            continue;
        };
        let Some(to) = named(right.trim()) else {
            continue;
        };
        if from.dimension != to.dimension {
            return None;
        }
        let base = value * from.factor + from.offset;
        let result = (base - to.offset) / to.factor;
        return result.is_finite().then_some((result, to.symbol));
    }
    None
}

/// Every connector occurrence, left to right, as `(start, end)` byte offsets. Overlapping
/// occurrences count (`20 in in cm` has two).
fn connector_spans(input: &str) -> Vec<(usize, usize)> {
    let lower = input.to_ascii_lowercase();
    let mut spans = Vec::new();
    for (at, _) in lower.char_indices() {
        for connector in CONNECTORS {
            if lower
                .get(at..)
                .is_some_and(|rest| rest.starts_with(connector))
            {
                spans.push((at, at + connector.len()));
            }
        }
    }
    spans
}

/// Whether `word` is the unit name `name`. `b` (bit) and `B` (byte) differ only by case; every
/// other name ignores case.
fn same(word: &str, name: &str) -> bool {
    if name == "b" || name == "B" {
        word == name
    } else {
        word.eq_ignore_ascii_case(name)
    }
}

/// The unit a whole piece of text names.
fn named(text: &str) -> Option<&'static Unit> {
    UNITS
        .iter()
        .find(|unit| unit.names.iter().any(|&name| same(text, name)))
}

/// The value of `<expression><unit>` and its unit. The longest unit name that leaves a valid
/// expression wins, so `km/h` beats `h`.
fn quantity(text: &str) -> Option<(f64, &'static Unit)> {
    let mut suffixes: Vec<(usize, &'static Unit)> = Vec::new();
    for unit in UNITS {
        for &name in unit.names {
            let Some(split) = text.len().checked_sub(name.len()) else {
                continue;
            };
            if text.get(split..).is_some_and(|tail| same(tail, name)) {
                suffixes.push((split, unit));
            }
        }
    }
    suffixes.sort_by_key(|&(split, _)| split);
    suffixes.into_iter().find_map(|(split, unit)| {
        let expression = text
            .get(..split)
            .filter(|expression| !expression.trim().is_empty())?;
        calc::value_of(expression).map(|value| (value, unit))
    })
}
